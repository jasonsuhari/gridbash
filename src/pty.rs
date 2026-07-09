use std::{
    env,
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

const DEVICE_STATUS_QUERY: &[u8] = b"\x1b[5n";
const CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";
const PRIMARY_DEVICE_ATTRIBUTES_QUERY: &[u8] = b"\x1b[c";
const PRIMARY_DEVICE_ATTRIBUTES_ZERO_QUERY: &[u8] = b"\x1b[0c";
const MAX_TERMINAL_QUERY_LEN: usize = 4;
const MAX_OSC_SCAN: usize = 4096;
const PTY_READ_BUFFER_BYTES: usize = 32 * 1024;

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
    osc_scan_tail: Vec<u8>,
    pub active: bool,
    pub exited: bool,
}

impl PtyPane {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        profile_name: &str,
        id: PaneId,
        generation: u64,
        command: &Path,
        args: &[String],
        cwd: &Path,
        extra_env: &[(String, String)],
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
        configure_cwd_reporting(profile_name, &mut command_builder);
        let args = args_with_cwd_reporting(profile_name, args);
        for arg in &args {
            command_builder.arg(arg);
        }
        command_builder.cwd(cwd);
        command_builder.env("TERM", "xterm-256color");
        command_builder.env("COLORTERM", "truecolor");
        for (key, value) in extra_env {
            command_builder.env(key, value);
        }

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
            osc_scan_tail: Vec::new(),
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
        self.update_cwd_from_osc7(bytes);
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

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<bool> {
        if self.rows == rows && self.cols == cols {
            return Ok(false);
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
        Ok(true)
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

        if contains_sequence(&scan, DEVICE_STATUS_QUERY) {
            let _ = self.write(b"\x1b[0n");
        }

        let cursor_position_requests = count_sequence(&scan, CURSOR_POSITION_QUERY);
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

        if contains_sequence(&scan, PRIMARY_DEVICE_ATTRIBUTES_QUERY)
            || contains_sequence(&scan, PRIMARY_DEVICE_ATTRIBUTES_ZERO_QUERY)
        {
            let _ = self.write(b"\x1b[?1;2c");
        }

        self.response_scan_tail = terminal_query_scan_tail(&scan);
    }

    fn update_cwd_from_osc7(&mut self, bytes: &[u8]) {
        let mut scan = Vec::with_capacity(self.osc_scan_tail.len() + bytes.len());
        scan.extend_from_slice(&self.osc_scan_tail);
        scan.extend_from_slice(bytes);

        for payload in osc_payloads(&scan) {
            if let Some(path) = cwd_from_osc7_payload(payload) {
                self.cwd = path;
            }
        }

        let tail_start = scan.len().saturating_sub(MAX_OSC_SCAN);
        self.osc_scan_tail = scan[tail_start..].to_vec();
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

fn configure_cwd_reporting(profile_name: &str, command_builder: &mut CommandBuilder) {
    match profile_name {
        "git-bash" => configure_bash_cwd_reporting(command_builder),
        "cmd" => command_builder.env("PROMPT", "$E]7;file:///$P$E\\$P$G"),
        _ => {}
    }
}

fn configure_bash_cwd_reporting(command_builder: &mut CommandBuilder) {
    let hook = "printf '\\033]7;file://%s%s\\007' \"${HOSTNAME:-localhost}\" \"$PWD\"";
    let prompt_command = match env::var("PROMPT_COMMAND") {
        Ok(existing) if !existing.trim().is_empty() => format!("{hook}; {existing}"),
        _ => hook.to_string(),
    };
    command_builder.env("PROMPT_COMMAND", prompt_command);
}

fn args_with_cwd_reporting(profile_name: &str, args: &[String]) -> Vec<String> {
    let mut args = args.to_vec();
    if matches!(profile_name, "pwsh" | "powershell") && !has_powershell_entrypoint(&args) {
        args.push("-NoExit".into());
        args.push("-Command".into());
        args.push(powershell_cwd_hook().into());
    }
    args
}

fn has_powershell_entrypoint(args: &[String]) -> bool {
    args.iter().any(|arg| {
        let normalized = arg.trim_start_matches('/').trim_start_matches('-');
        matches!(
            normalized.to_ascii_lowercase().as_str(),
            "command" | "c" | "file" | "f" | "encodedcommand" | "e" | "ec"
        )
    })
}

fn powershell_cwd_hook() -> &'static str {
    "$global:__GridBashPrompt = (Get-Command prompt -CommandType Function -ErrorAction SilentlyContinue).ScriptBlock; function global:prompt { $p = $ExecutionContext.SessionState.Path.CurrentLocation.ProviderPath; if ($p) { [Console]::Write(\"$([char]27)]7;$([System.Uri]::new($p).AbsoluteUri)$([char]7)\") }; if ($global:__GridBashPrompt) { & $global:__GridBashPrompt } else { \"PS $($ExecutionContext.SessionState.Path.CurrentLocation)> \" } }"
}

fn spawn_reader(
    pane: PaneId,
    generation: u64,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    mut reader: Box<dyn Read + Send>,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; PTY_READ_BUFFER_BYTES];
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

fn terminal_query_scan_tail(scan: &[u8]) -> Vec<u8> {
    let tail_len = MAX_TERMINAL_QUERY_LEN.saturating_sub(1);
    let tail_start = scan.len().saturating_sub(tail_len);
    scan[tail_start..].to_vec()
}

fn osc_payloads(buffer: &[u8]) -> Vec<&[u8]> {
    let mut payloads = Vec::new();
    let mut cursor = 0;

    while let Some(start_offset) = find_sequence(&buffer[cursor..], b"\x1b]") {
        let payload_start = cursor + start_offset + 2;
        let mut index = payload_start;
        let mut payload_end = None;
        let mut next_cursor = payload_start;

        while index < buffer.len() {
            if buffer[index] == 0x07 {
                payload_end = Some(index);
                next_cursor = index + 1;
                break;
            }
            if buffer[index] == 0x1b && buffer.get(index + 1) == Some(&b'\\') {
                payload_end = Some(index);
                next_cursor = index + 2;
                break;
            }
            index += 1;
        }

        let Some(payload_end) = payload_end else {
            break;
        };

        payloads.push(&buffer[payload_start..payload_end]);
        cursor = next_cursor;
    }

    payloads
}

fn cwd_from_osc7_payload(payload: &[u8]) -> Option<PathBuf> {
    let payload = String::from_utf8_lossy(payload);
    let body = payload.strip_prefix("7;file://")?;
    let uri_path = if body.starts_with('/') {
        body
    } else {
        let path_start = body.find('/')?;
        &body[path_start..]
    };

    let decoded = percent_decode(uri_path);
    let path = windows_path_from_uri_path(&decoded);
    Some(PathBuf::from(path))
}

fn windows_path_from_uri_path(path: &str) -> String {
    let path = path.replace('\\', "/");

    if path.len() >= 3 {
        let bytes = path.as_bytes();
        if bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && (bytes[2] == b':' || bytes[2] == b'|')
        {
            return format!("{}:{}", (bytes[1] as char).to_ascii_uppercase(), &path[3..]);
        }
    }

    if path.len() >= 3 {
        let bytes = path.as_bytes();
        if bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/' {
            return format!(
                "{}:/{}",
                (bytes[1] as char).to_ascii_uppercase(),
                &path[3..]
            );
        }
    }

    path
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
            continue;
        }

        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn find_sequence(buffer: &[u8], sequence: &[u8]) -> Option<usize> {
    if sequence.is_empty() || buffer.len() < sequence.len() {
        return None;
    }

    buffer
        .windows(sequence.len())
        .position(|window| window == sequence)
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
    fn terminal_query_scan_tail_keeps_split_queries_detectable() {
        let mut scan = terminal_query_scan_tail(b"prompt\x1b[6");
        scan.extend_from_slice(b"n");

        assert_eq!(count_sequence(&scan, CURSOR_POSITION_QUERY), 1);
    }

    #[test]
    fn terminal_query_scan_tail_does_not_replay_complete_queries() {
        let mut scan = terminal_query_scan_tail(CURSOR_POSITION_QUERY);
        scan.extend_from_slice(b"prompt");

        assert_eq!(count_sequence(&scan, CURSOR_POSITION_QUERY), 0);
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
    fn parses_osc7_windows_cwd() {
        let payload = b"7;file://localhost/C:/Users/Jason/My%20Repo";
        assert_eq!(
            cwd_from_osc7_payload(payload),
            Some(PathBuf::from("C:/Users/Jason/My Repo"))
        );
    }

    #[test]
    fn parses_osc7_windows_drive_slash_cwd() {
        let payload = b"7;file://localhost/C/Users/Jason/My%20Repo";
        assert_eq!(
            cwd_from_osc7_payload(payload),
            Some(PathBuf::from("C:/Users/Jason/My Repo"))
        );
    }

    #[test]
    fn finds_bel_and_st_terminated_osc_payloads() {
        assert_eq!(
            osc_payloads(b"before\x1b]7;file:///C:/A\x07middle\x1b]7;file:///C:/B\x1b\\"),
            vec![b"7;file:///C:/A".as_slice(), b"7;file:///C:/B".as_slice()]
        );
    }

    #[test]
    #[ignore = "Windows ConPTY smoke test requires an interactive console; run manually when debugging PTY I/O"]
    fn spawned_pty_receives_output_and_input() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let cwd = env::current_dir().expect("current dir");
        let mut pane = PtyPane::spawn(
            "cmd",
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
            &[],
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
