use std::{
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    thread,
};

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;
use vt100::{Parser, Screen};

use crate::layout::PaneId;

#[derive(Debug, Clone)]
pub enum PtyEvent {
    Output { pane: PaneId, bytes: Vec<u8> },
    Exited { pane: PaneId },
}

pub struct PtyPane {
    id: PaneId,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    parser: Parser,
    title: String,
    profile: String,
    pub active: bool,
    pub exited: bool,
    bytes_seen: u64,
}

impl PtyPane {
    pub fn spawn(
        id: PaneId,
        profile_name: &str,
        title: String,
        command: &Path,
        args: &[String],
        cwd: &Path,
        event_tx: mpsc::UnboundedSender<PtyEvent>,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to open PTY")?;

        let mut command_builder = CommandBuilder::new(command);
        for arg in args {
            command_builder.arg(arg);
        }
        command_builder.cwd(cwd);

        let child = pair
            .slave
            .spawn_command(command_builder)
            .with_context(|| format!("failed to spawn {}", command.display()))?;
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .context("failed to clone PTY reader")?;
        let writer = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .context("failed to open PTY writer")?,
        ));
        spawn_reader(id, event_tx, reader);

        Ok(Self {
            id,
            master: pair.master,
            child,
            writer,
            parser: Parser::new(24, 80, 10_000),
            title,
            profile: profile_name.to_string(),
            active: false,
            exited: false,
            bytes_seen: 0,
        })
    }

    pub fn id(&self) -> PaneId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn screen(&self) -> &Screen {
        self.parser.screen()
    }

    pub fn process_output(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.bytes_seen = self.bytes_seen.saturating_add(bytes.len() as u64);
        self.active = true;
    }

    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().expect("PTY writer lock poisoned");
        writer.write_all(bytes).context("failed to write to PTY")?;
        writer.flush().context("failed to flush PTY")
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.parser.screen_mut().set_size(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to resize PTY")
    }

    pub fn poll_exit(&mut self) {
        if self.exited {
            return;
        }

        if matches!(self.child.try_wait(), Ok(Some(_))) {
            self.exited = true;
        }
    }

    pub fn bytes_seen(&self) -> u64 {
        self.bytes_seen
    }
}

fn spawn_reader(
    pane: PaneId,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    mut reader: Box<dyn Read + Send>,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = event_tx.send(PtyEvent::Exited { pane });
                    break;
                }
                Ok(n) => {
                    let _ = event_tx.send(PtyEvent::Output {
                        pane,
                        bytes: buffer[..n].to_vec(),
                    });
                }
                Err(_) => {
                    let _ = event_tx.send(PtyEvent::Exited { pane });
                    break;
                }
            }
        }
    });
}
