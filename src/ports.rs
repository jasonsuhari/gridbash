use std::{
    collections::{HashMap, HashSet},
    io::{self, Read},
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const INSPECTOR_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const INSPECTOR_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProcessRoot {
    pub pid: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPort {
    pub port: u16,
    pub pid: u32,
    pub process: String,
    pub owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessInfo {
    parent_pid: u32,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListeningSocket {
    port: u16,
    pid: u32,
    process: Option<String>,
}

pub fn discover_agent_ports(roots: &[AgentProcessRoot]) -> io::Result<Vec<AgentPort>> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    let processes = process_snapshot()?;
    let listeners = listening_sockets()?;
    Ok(associate_agent_ports(roots, &processes, listeners))
}

pub fn terminate_process(pid: u32) -> io::Result<()> {
    if pid == 0 || pid == std::process::id() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to terminate an invalid or current process",
        ));
    }

    terminate_process_platform(pid)
}

/// Every process running at one moment, so several questions about the process
/// tree can be answered from a single listing.
pub struct ProcessTable {
    processes: HashMap<u32, ProcessInfo>,
}

impl ProcessTable {
    pub fn capture() -> io::Result<Self> {
        Ok(Self {
            processes: process_snapshot()?,
        })
    }

    pub fn children_of(&self, parent: u32) -> impl Iterator<Item = u32> + '_ {
        self.processes
            .iter()
            .filter(move |(_, process)| process.parent_pid == parent)
            .map(|(pid, _)| *pid)
    }

    pub fn roots_with_descendant_named(&self, roots: &[u32], expected_name: &str) -> HashSet<u32> {
        let roots = roots.iter().copied().collect::<HashSet<_>>();
        let expected_name = expected_name.trim_end_matches(".exe").to_ascii_lowercase();
        self.processes
            .iter()
            .filter(|(_, process)| {
                let name = process
                    .name
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&process.name)
                    .to_ascii_lowercase();
                name.trim_end_matches(".exe") == expected_name
            })
            .filter_map(|(pid, _)| descendant_root(*pid, &self.processes, &roots))
            .collect()
    }

    pub fn descendants_of(&self, roots: &[u32]) -> ProcessTree {
        let roots = roots.iter().copied().collect::<HashSet<_>>();
        let running_roots = roots
            .iter()
            .copied()
            .filter(|root| self.processes.contains_key(root))
            .collect();
        let descendants = self
            .processes
            .keys()
            .filter_map(|pid| {
                let mut current = *pid;
                for depth in 1..=64 {
                    let parent = self.processes.get(&current)?.parent_pid;
                    if roots.contains(&parent) {
                        return Some((*pid, parent, depth));
                    }
                    if parent == 0 || parent == current {
                        return None;
                    }
                    current = parent;
                }
                None
            })
            .collect();
        ProcessTree {
            descendants,
            running_roots,
        }
    }
}

pub struct ProcessTree {
    /// Every running process under a root, as `(pid, root, depth)` where a
    /// direct child of its root has depth 1.
    pub descendants: Vec<(u32, u32, usize)>,
    /// The roots that were running.
    pub running_roots: HashSet<u32>,
}

fn descendant_root(
    pid: u32,
    processes: &HashMap<u32, ProcessInfo>,
    roots: &HashSet<u32>,
) -> Option<u32> {
    let mut current = pid;
    let mut visited = HashSet::new();
    for _ in 0..64 {
        if !visited.insert(current) {
            return None;
        }
        let parent = processes.get(&current)?.parent_pid;
        if roots.contains(&parent) {
            return Some(parent);
        }
        if parent == 0 || parent == current {
            return None;
        }
        current = parent;
    }
    None
}

fn associate_agent_ports(
    roots: &[AgentProcessRoot],
    processes: &HashMap<u32, ProcessInfo>,
    listeners: Vec<ListeningSocket>,
) -> Vec<AgentPort> {
    let roots = roots
        .iter()
        .filter(|root| {
            processes
                .get(&root.pid)
                .is_some_and(|process| is_gridbash_host(&process.name))
        })
        .map(|root| (root.pid, root.label.as_str()))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut ports = listeners
        .into_iter()
        .filter_map(|listener| {
            let owner = process_root(listener.pid, processes, &roots)?;
            seen.insert((listener.port, listener.pid))
                .then(|| AgentPort {
                    port: listener.port,
                    pid: listener.pid,
                    process: listener
                        .process
                        .filter(|name| !name.is_empty())
                        .or_else(|| {
                            processes
                                .get(&listener.pid)
                                .map(|process| process.name.clone())
                        })
                        .unwrap_or_else(|| "unknown".into()),
                    owner: owner.to_string(),
                })
        })
        .collect::<Vec<_>>();
    ports.sort_by(|left, right| {
        left.port
            .cmp(&right.port)
            .then_with(|| left.pid.cmp(&right.pid))
    });
    ports
}

fn is_gridbash_host(process_name: &str) -> bool {
    process_name
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("gridbash"))
}

fn process_root<'a>(
    pid: u32,
    processes: &HashMap<u32, ProcessInfo>,
    roots: &HashMap<u32, &'a str>,
) -> Option<&'a str> {
    let mut current = pid;
    let mut visited = HashSet::new();
    for _ in 0..64 {
        if !visited.insert(current) {
            return None;
        }
        let parent = processes.get(&current)?.parent_pid;
        if let Some(label) = roots.get(&parent) {
            return Some(*label);
        }
        if parent == 0 || parent == current {
            return None;
        }
        current = parent;
    }
    None
}

fn endpoint_port(endpoint: &str) -> Option<u16> {
    endpoint
        .trim()
        .rsplit_once(':')
        .and_then(|(_, port)| port.trim_end_matches(']').parse().ok())
}

#[cfg(windows)]
fn listening_sockets() -> io::Result<Vec<ListeningSocket>> {
    let mut command = Command::new("netstat");
    command.args(["-ano", "-p", "tcp"]);
    let output = command_output_with_timeout(&mut command)?;
    if !output.status.success() {
        return Err(io::Error::other("netstat failed while listing TCP ports"));
    }
    Ok(parse_windows_netstat(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

#[cfg(windows)]
fn parse_windows_netstat(output: &str) -> Vec<ListeningSocket> {
    output
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 5
                || !fields[0].eq_ignore_ascii_case("TCP")
                || !fields[3].eq_ignore_ascii_case("LISTENING")
            {
                return None;
            }
            Some(ListeningSocket {
                port: endpoint_port(fields[1])?,
                pid: fields[4].parse().ok()?,
                process: None,
            })
        })
        .collect()
}

#[cfg(windows)]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    use std::mem;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let mut entry = unsafe { mem::zeroed::<PROCESSENTRY32W>() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut processes = HashMap::new();
    let mut success = unsafe { Process32FirstW(snapshot, &mut entry) };
    while success != 0 {
        let length = entry
            .szExeFile
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
        processes.insert(
            entry.th32ProcessID,
            ProcessInfo {
                parent_pid: entry.th32ParentProcessID,
                name,
            },
        );
        success = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    unsafe { CloseHandle(snapshot) };
    Ok(processes)
}

#[cfg(windows)]
fn terminate_process_platform(pid: u32) -> io::Result<()> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
    };

    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { TerminateProcess(process, 1) };
    let error = (result == 0).then(io::Error::last_os_error);
    unsafe { CloseHandle(process) };
    error.map_or(Ok(()), Err)
}

#[cfg(unix)]
fn listening_sockets() -> io::Result<Vec<ListeningSocket>> {
    let lsof = if cfg!(target_os = "macos") {
        "/usr/sbin/lsof"
    } else {
        "lsof"
    };
    let mut command = Command::new(lsof);
    command.args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"]);
    match command_output_with_timeout(&mut command) {
        Ok(output) if output.status.success() || !output.stdout.is_empty() => {
            return Ok(parse_lsof(&String::from_utf8_lossy(&output.stdout)));
        }
        _ => {}
    }

    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new("ss");
        command.args(["-H", "-ltnp"]);
        let output = command_output_with_timeout(&mut command)?;
        if output.status.success() {
            return Ok(parse_linux_ss(&String::from_utf8_lossy(&output.stdout)));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "neither lsof nor a supported socket inspector is available",
    ))
}

#[cfg(unix)]
fn parse_lsof(output: &str) -> Vec<ListeningSocket> {
    let mut pid = None;
    let mut process = None::<String>;
    let mut listeners = Vec::new();
    for line in output.lines() {
        let Some(tag) = line.get(..1) else {
            continue;
        };
        let value = &line[1..];
        match tag {
            "p" => {
                pid = value.parse().ok();
                process = None;
            }
            "c" => process = Some(value.to_string()),
            "n" => {
                if let (Some(pid), Some(port)) = (pid, endpoint_port(value)) {
                    listeners.push(ListeningSocket {
                        port,
                        pid,
                        process: process.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    listeners
}

#[cfg(target_os = "linux")]
fn parse_linux_ss(output: &str) -> Vec<ListeningSocket> {
    output
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let local = *fields.get(3)?;
            let details = fields.get(5..)?.join(" ");
            let pid_start = details.find("pid=")? + 4;
            let pid_end = details[pid_start..]
                .find(|ch: char| !ch.is_ascii_digit())
                .map(|offset| pid_start + offset)
                .unwrap_or(details.len());
            let pid = details[pid_start..pid_end].parse().ok()?;
            let process = details
                .split_once("((\"")
                .and_then(|(_, tail)| tail.split_once('"'))
                .map(|(name, _)| name.to_string());
            Some(ListeningSocket {
                port: endpoint_port(local)?,
                pid,
                process,
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    let mut processes = HashMap::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(name_start) = stat.find('(') else {
            continue;
        };
        let Some(name_end) = stat.rfind(')') else {
            continue;
        };
        let Some(parent_pid) = stat[name_end + 1..]
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        processes.insert(
            pid,
            ProcessInfo {
                parent_pid,
                name: stat[name_start + 1..name_end].to_string(),
            },
        );
    }
    Ok(processes)
}

#[cfg(target_os = "macos")]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    use std::{ffi::CStr, mem};

    let needed = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if needed < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pids = vec![0 as libc::pid_t; usize::try_from(needed).unwrap_or(0) + 256];
    let capacity = libc::c_int::try_from(pids.len() * mem::size_of::<libc::pid_t>())
        .map_err(io::Error::other)?;
    let listed = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), capacity) };
    if listed < 0 {
        return Err(io::Error::last_os_error());
    }
    pids.truncate(usize::try_from(listed).unwrap_or(0));

    let uid = unsafe { libc::getuid() };
    let info_size = mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let mut arguments = vec![0_u8; argument_buffer_size()];
    let mut processes = HashMap::with_capacity(pids.len());
    for pid in pids {
        let mut info = unsafe { mem::zeroed::<libc::proc_bsdinfo>() };
        let filled = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                info_size,
            )
        };
        if filled != info_size {
            continue;
        }
        // `ps` names a process by its argv[0], which for Claude is the
        // `claude` link it was started through; the kernel's own name is the
        // versioned file behind it. Arguments are only readable for our own
        // processes, and `ps` falls back to the kernel name for the rest too.
        let name = (info.pbi_uid == uid)
            .then(|| first_argument(pid, &mut arguments))
            .flatten()
            .unwrap_or_else(|| {
                let name = unsafe { CStr::from_ptr(info.pbi_name.as_ptr()) };
                let name = if name.is_empty() {
                    unsafe { CStr::from_ptr(info.pbi_comm.as_ptr()) }
                } else {
                    name
                };
                name.to_string_lossy().into_owned()
            });
        processes.insert(
            pid as u32,
            ProcessInfo {
                parent_pid: info.pbi_ppid,
                name: name.rsplit('/').next().unwrap_or(&name).to_string(),
            },
        );
    }
    Ok(processes)
}

/// The folder of the program in front on `leader`'s terminal, which is where
/// the person last moved to in that pane; `leader`'s own folder otherwise.
pub fn foreground_cwd(leader: u32) -> Option<PathBuf> {
    foreground_cwd_platform(leader)
}

#[cfg(target_os = "macos")]
fn foreground_cwd_platform(leader: u32) -> Option<PathBuf> {
    let leader = libc::pid_t::try_from(leader).ok()?;
    let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let filled = unsafe {
        libc::proc_pidinfo(
            leader,
            libc::PROC_PIDTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    let foreground = (filled == size)
        .then(|| libc::pid_t::try_from(info.e_tpgid).ok())
        .flatten()
        .filter(|pid| *pid > 0);
    foreground
        .and_then(process_cwd)
        .or_else(|| process_cwd(leader))
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

#[cfg(target_os = "linux")]
fn foreground_cwd_platform(leader: u32) -> Option<PathBuf> {
    let foreground = std::fs::read_to_string(format!("/proc/{leader}/stat"))
        .ok()
        .and_then(|stat| {
            let fields = stat.get(stat.rfind(')')? + 1..)?;
            fields.split_whitespace().nth(5)?.parse::<i32>().ok()
        })
        .filter(|pid| *pid > 0);
    foreground
        .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/cwd")).ok())
        .or_else(|| std::fs::read_link(format!("/proc/{leader}/cwd")).ok())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn foreground_cwd_platform(_leader: u32) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn argument_buffer_size() -> usize {
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&raw mut value).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status == 0 && value > 0 {
        value as usize
    } else {
        1 << 18
    }
}

/// argv[0] of `pid`, read from the kernel's copy of its arguments: the argument
/// count, the executable path, padding, then the arguments themselves.
#[cfg(target_os = "macos")]
fn first_argument(pid: libc::pid_t, buffer: &mut [u8]) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size = buffer.len();
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return None;
    }
    let data = buffer.get(std::mem::size_of::<libc::c_int>()..size)?;
    let path_end = data.iter().position(|byte| *byte == 0)?;
    let rest = &data[path_end..];
    let start = rest.iter().position(|byte| *byte != 0)?;
    let rest = &rest[start..];
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(rest.len());
    let first = String::from_utf8_lossy(&rest[..end]).into_owned();
    (!first.is_empty()).then_some(first)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    let mut command = Command::new("ps");
    command.args(["-axo", "pid=,ppid=,comm="]);
    let output = command_output_with_timeout(&mut command)?;
    if !output.status.success() {
        return Err(io::Error::other("ps failed while listing processes"));
    }
    Ok(parse_ps(&String::from_utf8_lossy(&output.stdout)))
}

fn command_output_with_timeout(command: &mut Command) -> io::Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("failed to capture inspector stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("failed to capture inspector stderr"))?;
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + INSPECTOR_COMMAND_TIMEOUT;

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "port inspection command timed out",
            ));
        }
        thread::sleep(INSPECTOR_COMMAND_POLL_INTERVAL);
    };

    Ok(Output {
        status,
        stdout: join_reader(stdout_reader)?,
        stderr: join_reader(stderr_reader)?,
    })
}

fn read_all(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_reader(reader: thread::JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| io::Error::other("port inspection output reader panicked"))?
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn parse_ps(output: &str) -> HashMap<u32, ProcessInfo> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent_pid = fields.next()?.parse().ok()?;
            let name = fields.next()?.rsplit('/').next()?.to_string();
            Some((pid, ProcessInfo { parent_pid, name }))
        })
        .collect()
}

#[cfg(unix)]
fn terminate_process_platform(pid: u32) -> io::Result<()> {
    let pid = i32::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "process ID is too large"))?;
    let result = unsafe { libc::kill(pid, libc::SIGTERM) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(windows, unix)))]
fn listening_sockets() -> io::Result<Vec<ListeningSocket>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "port discovery is not supported on this platform",
    ))
}

#[cfg(not(any(windows, unix)))]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    Ok(HashMap::new())
}

#[cfg(not(any(windows, unix)))]
fn terminate_process_platform(_pid: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process termination is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn process_table_names_a_process_by_the_link_it_was_started_through() {
        let dir = std::env::temp_dir().join(format!("gridbash-ports-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("claude");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/bin/sleep", &link).unwrap();
        let mut child = Command::new(&link).arg("30").spawn().unwrap();

        let table = ProcessTable::capture().unwrap();
        let roots = table.roots_with_descendant_named(&[std::process::id()], "claude");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(roots, HashSet::from([std::process::id()]));
    }

    #[test]
    fn associates_only_descendants_of_agent_roots() {
        let roots = vec![AgentProcessRoot {
            pid: 10,
            label: "Grid 1 / Pane 2".into(),
        }];
        let processes = HashMap::from([
            (
                10,
                ProcessInfo {
                    parent_pid: 1,
                    name: "gridbash".into(),
                },
            ),
            (
                20,
                ProcessInfo {
                    parent_pid: 10,
                    name: "codex".into(),
                },
            ),
            (
                30,
                ProcessInfo {
                    parent_pid: 20,
                    name: "node".into(),
                },
            ),
            (
                40,
                ProcessInfo {
                    parent_pid: 1,
                    name: "postgres".into(),
                },
            ),
        ]);
        let listeners = vec![
            ListeningSocket {
                port: 41_000,
                pid: 10,
                process: None,
            },
            ListeningSocket {
                port: 3000,
                pid: 30,
                process: None,
            },
            ListeningSocket {
                port: 5432,
                pid: 40,
                process: None,
            },
        ];

        assert_eq!(
            associate_agent_ports(&roots, &processes, listeners),
            vec![AgentPort {
                port: 3000,
                pid: 30,
                process: "node".into(),
                owner: "Grid 1 / Pane 2".into(),
            }]
        );
    }

    #[test]
    fn finds_the_owning_root_for_nested_processes() {
        let processes = HashMap::from([
            (
                10,
                ProcessInfo {
                    parent_pid: 1,
                    name: "gridbash".into(),
                },
            ),
            (
                20,
                ProcessInfo {
                    parent_pid: 10,
                    name: "bash".into(),
                },
            ),
            (
                30,
                ProcessInfo {
                    parent_pid: 20,
                    name: "codex".into(),
                },
            ),
        ]);
        let roots = HashSet::from([10]);

        assert_eq!(descendant_root(30, &processes, &roots), Some(10));
        assert_eq!(descendant_root(10, &processes, &roots), None);
    }

    #[cfg(windows)]
    #[test]
    fn parses_windows_tcp_listeners() {
        let output = r#"
  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       1234
  TCP    [::1]:8080             [::]:0                 LISTENING       4321
  TCP    127.0.0.1:5000         127.0.0.1:6000         ESTABLISHED     9999
"#;
        assert_eq!(
            parse_windows_netstat(output),
            vec![
                ListeningSocket {
                    port: 3000,
                    pid: 1234,
                    process: None,
                },
                ListeningSocket {
                    port: 8080,
                    pid: 4321,
                    process: None,
                },
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn parses_lsof_machine_output() {
        let output = "p1234\ncnode\nf19\nPIPv4\nn*:3000\nn127.0.0.1:3001\n";
        assert_eq!(
            parse_lsof(output),
            vec![
                ListeningSocket {
                    port: 3000,
                    pid: 1234,
                    process: Some("node".into()),
                },
                ListeningSocket {
                    port: 3001,
                    pid: 1234,
                    process: Some("node".into()),
                },
            ]
        );
    }
}
