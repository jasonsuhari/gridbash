use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
    thread,
};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, State,
    ipc::{Channel, InvokeResponseBody},
};

const READ_CHUNK_BYTES: usize = 64 * 1024;

struct Pane {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct Panes {
    next_id: AtomicU32,
    open: Mutex<HashMap<u32, Pane>>,
}

#[derive(Serialize)]
pub struct PaneInfo {
    program: Option<String>,
    folder: Option<PathBuf>,
}

#[derive(Clone, Serialize)]
struct PaneExited {
    id: u32,
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[tauri::command]
pub fn open_pane(
    app: AppHandle,
    panes: State<'_, Panes>,
    cwd: Option<PathBuf>,
    rows: u16,
    cols: u16,
    output: Channel<InvokeResponseBody>,
) -> Result<u32, String> {
    let pair = native_pty_system()
        .openpty(size(rows, cols))
        .map_err(|error| error.to_string())?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut command = CommandBuilder::new(shell);
    command.arg("-l");
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "GridBash");
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(folder) = cwd.or(home) {
        command.cwd(folder);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| error.to_string())?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| error.to_string())?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| error.to_string())?;
    let id = panes.next_id.fetch_add(1, Ordering::Relaxed) + 1;

    thread::spawn(move || {
        let mut buffer = vec![0u8; READ_CHUNK_BYTES];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if output
                        .send(InvokeResponseBody::Raw(buffer[..read].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = app.emit("pane-exited", PaneExited { id });
    });

    panes.open.lock().unwrap().insert(
        id,
        Pane {
            master: pair.master,
            writer,
            child,
        },
    );
    Ok(id)
}

#[tauri::command]
pub fn log_frontend(message: String) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let directory = PathBuf::from(home).join("Library/Logs/GridBash");
    let _ = std::fs::create_dir_all(&directory);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or_default();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("desktop.log"))
    {
        let _ = writeln!(file, "{stamp} {message}");
    }
}

#[tauri::command]
pub fn write_pane(panes: State<'_, Panes>, id: u32, data: String) -> Result<(), String> {
    let mut open = panes.open.lock().unwrap();
    let pane = open.get_mut(&id).ok_or("no such pane")?;
    pane.writer
        .write_all(data.as_bytes())
        .and_then(|()| pane.writer.flush())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn resize_pane(panes: State<'_, Panes>, id: u32, rows: u16, cols: u16) -> Result<(), String> {
    let open = panes.open.lock().unwrap();
    let pane = open.get(&id).ok_or("no such pane")?;
    pane.master
        .resize(size(rows, cols))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn close_pane(panes: State<'_, Panes>, id: u32) {
    if let Some(mut pane) = panes.open.lock().unwrap().remove(&id) {
        let _ = pane.child.kill();
    }
}

#[tauri::command]
pub fn pane_info(panes: State<'_, Panes>, id: u32) -> Option<PaneInfo> {
    let open = panes.open.lock().unwrap();
    let pane = open.get(&id)?;
    let foreground = pane
        .master
        .process_group_leader()
        .filter(|pid| *pid > 0)
        .or_else(|| pane.child.process_id().map(|pid| pid as i32))?;
    Some(PaneInfo {
        program: process_name(foreground),
        folder: process_cwd(foreground),
    })
}

impl Panes {
    pub fn close_all(&self) {
        for (_, mut pane) in self.open.lock().unwrap().drain() {
            let _ = pane.child.kill();
        }
    }
}

#[cfg(target_os = "macos")]
fn process_name(pid: libc::pid_t) -> Option<String> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let written =
        unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if written <= 0 {
        return None;
    }
    let path = String::from_utf8_lossy(&buffer[..written as usize]).into_owned();
    path.rsplit('/').next().map(str::to_owned)
}

#[cfg(target_os = "macos")]
fn process_cwd(pid: libc::pid_t) -> Option<PathBuf> {
    use std::{ffi::CStr, os::unix::ffi::OsStrExt};

    let mut info = unsafe { std::mem::zeroed::<libc::proc_vnodepathinfo>() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let filled = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if filled != size {
        return None;
    }
    let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    let path = std::ffi::OsStr::from_bytes(path.to_bytes());
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(not(target_os = "macos"))]
fn process_name(_pid: i32) -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
fn process_cwd(_pid: i32) -> Option<PathBuf> {
    None
}
