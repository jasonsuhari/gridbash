use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;
use vt100::{Parser, Screen};

use crate::layout::PaneId;

#[derive(Debug, Clone)]
pub enum PtyEvent {
    Output {
        pane: PaneId,
        generation: u64,
        bytes: Vec<u8>,
    },
    Exited {
        pane: PaneId,
        generation: u64,
    },
}

#[derive(Debug, Clone, Default)]
struct OutputActivity {
    last_output_at: Option<Instant>,
    quiet: bool,
}

impl OutputActivity {
    fn record_output(&mut self, now: Instant) {
        self.last_output_at = Some(now);
        self.quiet = false;
    }

    fn refresh(&mut self, now: Instant, quiet_after: Duration) -> bool {
        if self.quiet {
            return false;
        }

        let Some(last_output_at) = self.last_output_at else {
            return false;
        };

        if now.duration_since(last_output_at) < quiet_after {
            return false;
        }

        self.quiet = true;
        true
    }

    fn is_quiet(&self) -> bool {
        self.quiet
    }
}

pub struct PtyPane {
    id: PaneId,
    generation: u64,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    parser: Parser,
    cwd: PathBuf,
    rows: u16,
    cols: u16,
    response_scan_tail: Vec<u8>,
    output_activity: OutputActivity,
    pub active: bool,
    pub exited: bool,
}

impl PtyPane {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        id: PaneId,
        generation: u64,
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
        command_builder.env("TERM", "xterm-256color");
        command_builder.env("COLORTERM", "truecolor");

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
        spawn_reader(id, generation, event_tx, reader);

        Ok(Self {
            id,
            generation,
            master: pair.master,
            child,
            writer,
            parser: Parser::new(24, 80, 10_000),
            cwd: cwd.to_path_buf(),
            rows: 24,
            cols: 80,
            response_scan_tail: Vec::new(),
            output_activity: OutputActivity::default(),
            active: false,
            exited: false,
        })
    }

    pub fn id(&self) -> PaneId {
        self.id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn screen(&self) -> &Screen {
        self.parser.screen()
    }

    pub fn process_output(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.active = true;
        self.output_activity.record_output(Instant::now());
        self.answer_terminal_queries(bytes);
    }

    pub fn output_quiet(&self) -> bool {
        !self.exited && self.output_activity.is_quiet()
    }

    pub fn refresh_output_activity(&mut self, now: Instant, quiet_after: Duration) -> bool {
        if self.exited {
            return false;
        }

        self.output_activity.refresh(now, quiet_after)
    }

    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().expect("PTY writer lock poisoned");
        writer.write_all(bytes).context("failed to write to PTY")?;
        writer.flush().context("failed to flush PTY")
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if self.rows == rows && self.cols == cols {
            return Ok(());
        }

        self.parser.screen_mut().set_size(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to resize PTY")?;
        self.rows = rows;
        self.cols = cols;
        Ok(())
    }

    pub fn poll_exit(&mut self) -> bool {
        if self.exited {
            return false;
        }

        if matches!(self.child.try_wait(), Ok(Some(_))) {
            self.exited = true;
            return true;
        }

        false
    }

    fn answer_terminal_queries(&mut self, bytes: &[u8]) {
        let mut scan = Vec::with_capacity(self.response_scan_tail.len() + bytes.len());
        scan.extend_from_slice(&self.response_scan_tail);
        scan.extend_from_slice(bytes);

        if contains_sequence(&scan, b"\x1b[5n") {
            let _ = self.write(b"\x1b[0n");
        }

        let cursor_position_requests = count_sequence(&scan, b"\x1b[6n");
        if cursor_position_requests > 0 {
            let (row, column) = self.parser.screen().cursor_position();
            let response = format!(
                "\x1b[{};{}R",
                row.saturating_add(1),
                column.saturating_add(1)
            );
            for _ in 0..cursor_position_requests {
                let _ = self.write(response.as_bytes());
            }
        }

        if contains_sequence(&scan, b"\x1b[c") || contains_sequence(&scan, b"\x1b[0c") {
            let _ = self.write(b"\x1b[?1;2c");
        }

        const MAX_QUERY_LEN: usize = 5;
        let tail_start = scan.len().saturating_sub(MAX_QUERY_LEN - 1);
        self.response_scan_tail = scan[tail_start..].to_vec();
    }

    pub fn terminate(&mut self) {
        if self.exited {
            return;
        }

        let _ = self.child.kill();
        self.exited = true;
    }
}

impl Drop for PtyPane {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn spawn_reader(
    pane: PaneId,
    generation: u64,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    mut reader: Box<dyn Read + Send>,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = event_tx.send(PtyEvent::Exited { pane, generation });
                    break;
                }
                Ok(n) => {
                    let _ = event_tx.send(PtyEvent::Output {
                        pane,
                        generation,
                        bytes: buffer[..n].to_vec(),
                    });
                }
                Err(_) => {
                    let _ = event_tx.send(PtyEvent::Exited { pane, generation });
                    break;
                }
            }
        }
    });
}

fn contains_sequence(buffer: &[u8], sequence: &[u8]) -> bool {
    count_sequence(buffer, sequence) > 0
}

fn count_sequence(buffer: &[u8], sequence: &[u8]) -> usize {
    if sequence.is_empty() || buffer.len() < sequence.len() {
        return 0;
    }

    buffer
        .windows(sequence.len())
        .filter(|window| *window == sequence)
        .count()
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        path::Path,
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    #[test]
    fn counts_split_terminal_query_sequences() {
        assert_eq!(count_sequence(b"\x1b[6n", b"\x1b[6n"), 1);
        assert_eq!(count_sequence(b"abc", b"\x1b[6n"), 0);
    }

    #[test]
    fn output_activity_marks_quiet_after_output_stops() {
        let start = Instant::now();
        let quiet_after = Duration::from_secs(3);
        let mut activity = OutputActivity::default();

        assert!(!activity.refresh(start + quiet_after, quiet_after));
        assert!(!activity.is_quiet());

        activity.record_output(start);
        assert!(!activity.refresh(start + Duration::from_secs(2), quiet_after));
        assert!(!activity.is_quiet());

        assert!(activity.refresh(start + quiet_after, quiet_after));
        assert!(activity.is_quiet());
        assert!(!activity.refresh(start + quiet_after + Duration::from_secs(1), quiet_after));

        activity.record_output(start + quiet_after + Duration::from_secs(2));
        assert!(!activity.is_quiet());
    }

    #[test]
    #[ignore = "Windows ConPTY smoke test requires an interactive console; run manually when debugging PTY I/O"]
    fn spawned_pty_receives_output_and_input() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let cwd = env::current_dir().expect("current dir");
        let mut pane = PtyPane::spawn(
            PaneId(0),
            0,
            Path::new("cmd.exe"),
            &[
                "/d".to_string(),
                "/q".to_string(),
                "/v:on".to_string(),
                "/c".to_string(),
                "set /p GRIDBASH_IN= & echo GRIDBASH_READY:!GRIDBASH_IN!".to_string(),
            ],
            &cwd,
            event_tx,
        )
        .expect("spawn cmd pty");

        pane.write(b"typed-input\r").expect("write input to pty");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut raw_output = Vec::new();
        while Instant::now() < deadline {
            while let Ok(event) = event_rx.try_recv() {
                match event {
                    PtyEvent::Output { bytes, .. } => {
                        pane.process_output(&bytes);
                        raw_output.extend(bytes);
                    }
                    PtyEvent::Exited { .. } => pane.exited = true,
                }
            }

            let raw_text = String::from_utf8_lossy(&raw_output);
            if raw_text.contains("GRIDBASH_READY:typed-input")
                && pane
                    .screen()
                    .contents()
                    .contains("GRIDBASH_READY:typed-input")
            {
                pane.terminate();
                return;
            }

            thread::sleep(Duration::from_millis(20));
        }

        pane.terminate();
        panic!(
            "PTY did not round-trip output/input; raw output was: {:?}; screen was: {:?}",
            String::from_utf8_lossy(&raw_output),
            pane.screen().contents()
        );
    }
}
