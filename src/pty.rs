use std::{
    collections::BTreeMap,
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{SyncSender, TrySendError, sync_channel},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;
use vt100::{Parser, Screen};

use crate::{
    config::{PaneProcessPriority, PaneWorkloadPolicy},
    layout::PaneId,
    process_priority::{PaneWorkloadClass, PaneWorkloadController},
};

const DEVICE_STATUS_QUERY: &[u8] = b"\x1b[5n";
const CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";
const PRIMARY_DEVICE_ATTRIBUTES_QUERY: &[u8] = b"\x1b[c";
const PRIMARY_DEVICE_ATTRIBUTES_ZERO_QUERY: &[u8] = b"\x1b[0c";
const MAX_TERMINAL_QUERY_LEN: usize = 4;
const MAX_INPUT_HISTORY: usize = 200;
const MAX_INPUT_LINE_CHARS: usize = 4096;
const MAX_OUTPUT_TAIL_CHARS: usize = 40_000;
const OUTPUT_TAIL_TRIM_AT_CHARS: usize = 48_000;
const MAX_REPLAY_OUTPUT_CHARS: usize = 18_000;
const MAX_OSC_SCAN: usize = 4096;
const PTY_READ_BUFFER_BYTES: usize = 32 * 1024;
const PTY_WRITE_QUEUE_MESSAGES: usize = 256;

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
    WriteFailed {
        pane: PaneId,
        generation: u64,
        error: String,
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
    writer: SyncSender<Vec<u8>>,
    workload: PaneWorkloadController,
    workload_error: Option<String>,
    parser: Parser,
    screen_revision: u64,
    cwd: PathBuf,
    rows: u16,
    cols: u16,
    response_scan_tail: Vec<u8>,
    output_activity: OutputActivity,
    osc_scan_tail: Vec<u8>,
    input_history: Vec<String>,
    pending_input: String,
    output_tail: String,
    output_tail_chars: usize,
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
        env: &BTreeMap<String, String>,
        cwd: &Path,
        extra_env: &[(String, String)],
        process_priority: PaneProcessPriority,
        workload_policy: PaneWorkloadPolicy,
        event_tx: mpsc::Sender<PtyEvent>,
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
        configure_cwd_reporting(profile_name, &mut command_builder)?;
        let args = args_with_cwd_reporting(profile_name, args);
        for arg in &args {
            command_builder.arg(arg);
        }
        command_builder.cwd(cwd);
        command_builder.env("TERM", "xterm-256color");
        command_builder.env("COLORTERM", "truecolor");
        for (key, value) in env {
            command_builder.env(key, value);
        }
        for (key, value) in extra_env {
            command_builder.env(key, value);
        }

        let child = pair
            .slave
            .spawn_command(command_builder)
            .with_context(|| format!("failed to spawn {}", command.display()))?;
        let (workload, workload_error) = match child.process_id() {
            Some(process_id) => {
                match PaneWorkloadController::attach(process_id, process_priority, workload_policy)
                {
                    Ok(controller) => (controller, None),
                    Err(error) => (PaneWorkloadController::unmanaged(), Some(error.to_string())),
                }
            }
            None => (
                PaneWorkloadController::unmanaged(),
                Some("child process ID is unavailable".into()),
            ),
        };
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .context("failed to clone PTY reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("failed to open PTY writer")?;
        let writer = spawn_writer(id, generation, event_tx.clone(), writer);
        spawn_reader(id, generation, event_tx, reader);

        Ok(Self {
            id,
            generation,
            master: pair.master,
            child,
            writer,
            workload,
            workload_error,
            parser: Parser::new(24, 80, 10_000),
            screen_revision: 0,
            cwd: cwd.to_path_buf(),
            rows: 24,
            cols: 80,
            response_scan_tail: Vec::new(),
            output_activity: OutputActivity::default(),
            osc_scan_tail: Vec::new(),
            input_history: Vec::new(),
            pending_input: String::new(),
            output_tail: String::new(),
            output_tail_chars: 0,
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

    pub fn screen_revision(&self) -> u64 {
        self.screen_revision
    }

    pub fn scroll_view(&mut self, rows: isize) -> bool {
        let changed = scroll_screen(self.parser.screen_mut(), rows);
        if changed {
            self.screen_revision = self.screen_revision.wrapping_add(1);
        }
        changed
    }

    pub fn reset_view(&mut self) -> bool {
        let screen = self.parser.screen_mut();
        let changed = screen.scrollback() > 0;
        screen.set_scrollback(0);
        if changed {
            self.screen_revision = self.screen_revision.wrapping_add(1);
        }
        changed
    }

    pub fn process_output(&mut self, bytes: &[u8]) -> String {
        self.update_cwd_from_osc7(bytes);
        self.parser.process(bytes);
        let plain = plain_terminal_text(bytes);
        self.append_plain_output(&plain);
        self.active = true;
        self.screen_revision = self.screen_revision.wrapping_add(1);
        self.output_activity.record_output(Instant::now());
        self.answer_terminal_queries(bytes);
        plain
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

    pub fn record_input(&mut self, bytes: &[u8]) {
        record_input_bytes(bytes, &mut self.pending_input, &mut self.input_history);
    }

    pub fn input_history(&self) -> &[String] {
        &self.input_history
    }

    pub fn output_tail(&self) -> &str {
        &self.output_tail
    }

    pub fn restore_history_display(&mut self, output_tail: &str, input_history: &[String]) {
        self.output_tail = output_tail.to_string();
        trim_string_tail(&mut self.output_tail, MAX_OUTPUT_TAIL_CHARS);
        self.output_tail_chars = self.output_tail.chars().count();
        self.input_history = input_history
            .iter()
            .filter(|line| !line.trim().is_empty())
            .rev()
            .take(MAX_INPUT_HISTORY)
            .cloned()
            .collect::<Vec<_>>();
        self.input_history.reverse();

        let replay = history_replay_text(&self.output_tail, &self.input_history);
        if !replay.is_empty() {
            self.parser.process(replay.as_bytes());
            self.screen_revision = self.screen_revision.wrapping_add(1);
        }
    }

    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        match self.writer.try_send(bytes.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(anyhow!("PTY input queue is full")),
            Err(TrySendError::Disconnected(_)) => Err(anyhow!("PTY writer has stopped")),
        }
    }

    pub fn apply_workload(
        &self,
        policy: PaneWorkloadPolicy,
        class: PaneWorkloadClass,
    ) -> Result<()> {
        if let Some(error) = &self.workload_error {
            return Err(anyhow!(error.clone()));
        }
        self.workload
            .apply(policy, class)
            .context("failed to update pane workload policy")
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
        self.screen_revision = self.screen_revision.wrapping_add(1);
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
        if self.response_scan_tail.is_empty() && !bytes.contains(&0x1b) {
            return;
        }
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
        if self.osc_scan_tail.is_empty() && !bytes.contains(&0x1b) {
            return;
        }
        let mut scan = Vec::with_capacity(self.osc_scan_tail.len() + bytes.len());
        scan.extend_from_slice(&self.osc_scan_tail);
        scan.extend_from_slice(bytes);

        for payload in osc_payloads(&scan) {
            if let Some(path) = cwd_from_osc7_payload(payload) {
                self.cwd = path;
            }
        }

        self.osc_scan_tail = incomplete_osc_tail(&scan);
    }

    pub fn terminate(&mut self) {
        if self.exited {
            return;
        }

        let _ = self.child.kill();
        self.exited = true;
    }

    fn append_plain_output(&mut self, plain: &str) {
        if plain.is_empty() {
            return;
        }

        self.output_tail.push_str(plain);
        self.output_tail_chars += plain.chars().count();
        if self.output_tail_chars > OUTPUT_TAIL_TRIM_AT_CHARS {
            trim_string_tail(&mut self.output_tail, MAX_OUTPUT_TAIL_CHARS);
            self.output_tail_chars = self.output_tail.chars().count();
        }
    }
}

fn scroll_screen(screen: &mut Screen, rows: isize) -> bool {
    let before = screen.scrollback();
    let next = if rows.is_negative() {
        before.saturating_sub(rows.unsigned_abs())
    } else {
        before.saturating_add(rows as usize)
    };
    screen.set_scrollback(next);
    screen.scrollback() != before
}

impl Drop for PtyPane {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn configure_cwd_reporting(profile_name: &str, command_builder: &mut CommandBuilder) -> Result<()> {
    match profile_name {
        "git-bash" | "bash" => configure_bash_cwd_reporting(command_builder),
        "cmd" => command_builder.env("PROMPT", "$E]7;file:///$P$E\\$P$G"),
        "zsh" => configure_zsh_cwd_reporting(command_builder)?,
        _ => {}
    }
    Ok(())
}

fn configure_bash_cwd_reporting(command_builder: &mut CommandBuilder) {
    let hook = "printf '\\033]7;file://%s%s\\007' \"${HOSTNAME:-localhost}\" \"$PWD\"";
    let prompt_command = match env::var("PROMPT_COMMAND") {
        Ok(existing) if !existing.trim().is_empty() => format!("{hook}; {existing}"),
        _ => hook.to_string(),
    };
    command_builder.env("PROMPT_COMMAND", prompt_command);
}

fn configure_zsh_cwd_reporting(command_builder: &mut CommandBuilder) -> Result<()> {
    const ZSHRC: &str = r#"if [[ -n "$GRIDBASH_ORIGINAL_ZDOTDIR" && -r "$GRIDBASH_ORIGINAL_ZDOTDIR/.zshrc" ]]; then
  _gridbash_zdotdir="$ZDOTDIR"
  ZDOTDIR="$GRIDBASH_ORIGINAL_ZDOTDIR"
  source "$GRIDBASH_ORIGINAL_ZDOTDIR/.zshrc"
  ZDOTDIR="$_gridbash_zdotdir"
  unset _gridbash_zdotdir
fi

function _gridbash_report_cwd() {
  printf '\033]7;file://%s%s\007' "${HOST:-localhost}" "$PWD"
}
autoload -Uz add-zsh-hook
add-zsh-hook precmd _gridbash_report_cwd
"#;

    let integration_dir = env::temp_dir().join("gridbash-zsh-integration-v1");
    fs::create_dir_all(&integration_dir)
        .context("failed to create GridBash zsh integration directory")?;
    fs::write(integration_dir.join(".zshrc"), ZSHRC)
        .context("failed to write GridBash zsh integration")?;

    let original_zdotdir = env::var_os("ZDOTDIR")
        .or_else(|| env::var_os("HOME"))
        .unwrap_or_default();
    command_builder.env("GRIDBASH_ORIGINAL_ZDOTDIR", original_zdotdir);
    command_builder.env("ZDOTDIR", integration_dir);
    Ok(())
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
    event_tx: mpsc::Sender<PtyEvent>,
    mut reader: Box<dyn Read + Send>,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; PTY_READ_BUFFER_BYTES];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = event_tx.blocking_send(PtyEvent::Exited { pane, generation });
                    break;
                }
                Ok(n) => {
                    let _ = event_tx.blocking_send(PtyEvent::Output {
                        pane,
                        generation,
                        bytes: buffer[..n].to_vec(),
                    });
                }
                Err(_) => {
                    let _ = event_tx.blocking_send(PtyEvent::Exited { pane, generation });
                    break;
                }
            }
        }
    });
}

fn spawn_writer(
    pane: PaneId,
    generation: u64,
    event_tx: mpsc::Sender<PtyEvent>,
    mut writer: Box<dyn Write + Send>,
) -> SyncSender<Vec<u8>> {
    let (tx, rx) = sync_channel::<Vec<u8>>(PTY_WRITE_QUEUE_MESSAGES);
    thread::spawn(move || {
        while let Ok(bytes) = rx.recv() {
            let result = writer
                .write_all(&bytes)
                .and_then(|()| writer.flush())
                .context("failed to write to PTY");
            if let Err(error) = result {
                let _ = event_tx.blocking_send(PtyEvent::WriteFailed {
                    pane,
                    generation,
                    error: format!("{error:#}"),
                });
                break;
            }
        }
    });
    tx
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
    #[cfg(windows)]
    let decoded = windows_path_from_uri_path(&decoded);
    Some(PathBuf::from(decoded))
}

#[cfg(windows)]
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

fn record_input_bytes(bytes: &[u8], pending_input: &mut String, input_history: &mut Vec<String>) {
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == 0x1b {
            index = skip_escape_sequence(bytes, index);
            continue;
        }

        match byte {
            b'\r' | b'\n' => finish_pending_input(pending_input, input_history),
            0x08 | 0x7f => {
                pending_input.pop();
            }
            b'\t' => push_pending_input(pending_input, '\t'),
            0x20..=0x7e => push_pending_input(pending_input, byte as char),
            _ => {}
        }
        index += 1;
    }
}

fn push_pending_input(pending_input: &mut String, ch: char) {
    if pending_input.chars().count() < MAX_INPUT_LINE_CHARS {
        pending_input.push(ch);
    }
}

fn finish_pending_input(pending_input: &mut String, input_history: &mut Vec<String>) {
    let line = pending_input.trim().to_string();
    pending_input.clear();
    if line.is_empty() {
        return;
    }

    input_history.push(line);
    if input_history.len() > MAX_INPUT_HISTORY {
        let extra = input_history.len() - MAX_INPUT_HISTORY;
        input_history.drain(..extra);
    }
}

fn history_replay_text(output_tail: &str, input_history: &[String]) -> String {
    if output_tail.trim().is_empty() && input_history.is_empty() {
        return String::new();
    }

    let mut replay = String::from(
        "\x1b[90mGridBash resumed pane history. Commands were not replayed.\x1b[0m\r\n",
    );

    if !input_history.is_empty() {
        replay.push_str("\x1b[36mprevious commands\x1b[0m\r\n");
        for line in input_history
            .iter()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            replay.push_str("> ");
            replay.push_str(line);
            replay.push_str("\r\n");
        }
    }

    let output = tail_chars(output_tail, MAX_REPLAY_OUTPUT_CHARS);
    if !output.trim().is_empty() {
        if !input_history.is_empty() {
            replay.push_str("\r\n");
        }
        replay.push_str("\x1b[36mlast output\x1b[0m\r\n");
        replay.push_str(&output.replace('\n', "\r\n"));
        replay.push_str("\r\n");
    }

    replay
}

pub(crate) fn plain_terminal_text(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let mut plain = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            skip_escape_chars(&mut chars);
            continue;
        }

        match ch {
            '\r' => {
                if !plain.ends_with('\n') {
                    plain.push('\n');
                }
            }
            '\n' => {
                if !plain.ends_with('\n') {
                    plain.push('\n');
                }
            }
            '\t' => plain.push(ch),
            '\x08' => {
                plain.pop();
            }
            ch if ch.is_control() => {}
            ch => plain.push(ch),
        }
    }

    plain
}

fn skip_escape_sequence(bytes: &[u8], start: usize) -> usize {
    let mut index = start.saturating_add(1);
    if index >= bytes.len() {
        return index;
    }

    match bytes[index] {
        b'[' => {
            index += 1;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        }
        b'O' => {
            index += 1;
            if index < bytes.len() {
                index += 1;
            }
        }
        b']' => {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == 0x07 {
                    index += 1;
                    break;
                }
                if bytes[index] == 0x1b && index + 1 < bytes.len() && bytes[index + 1] == b'\\' {
                    index += 2;
                    break;
                }
                index += 1;
            }
        }
        _ => index += 1,
    }

    index
}

fn skip_escape_chars(chars: &mut std::iter::Peekable<impl Iterator<Item = char>>) {
    let Some(kind) = chars.next() else {
        return;
    };
    match kind {
        '[' => {
            for ch in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&ch) {
                    break;
                }
            }
        }
        'O' => {
            let _ = chars.next();
        }
        ']' => {
            while let Some(ch) = chars.next() {
                if ch == '\x07' {
                    break;
                }
                if ch == '\x1b' && chars.next_if_eq(&'\\').is_some() {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn incomplete_osc_tail(buffer: &[u8]) -> Vec<u8> {
    let start = buffer.len().saturating_sub(MAX_OSC_SCAN);
    for index in (start..buffer.len()).rev() {
        if buffer[index] != 0x1b {
            continue;
        }
        if index + 1 == buffer.len() {
            return buffer[index..].to_vec();
        }
        if buffer[index + 1] != b']' {
            continue;
        }

        let payload = &buffer[index + 2..];
        let complete =
            payload.contains(&0x07) || payload.windows(2).any(|window| window == [0x1b, b'\\']);
        return if complete {
            Vec::new()
        } else {
            buffer[index..].to_vec()
        };
    }
    Vec::new()
}

fn trim_string_tail(value: &mut String, max_chars: usize) {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return;
    }

    let keep_from = char_count - max_chars;
    let byte_index = value
        .char_indices()
        .nth(keep_from)
        .map(|(index, _)| index)
        .unwrap_or(0);
    value.drain(..byte_index);
}

fn tail_chars(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }

    let keep_from = char_count - max_chars;
    let byte_index = value
        .char_indices()
        .nth(keep_from)
        .map(|(index, _)| index)
        .unwrap_or(0);
    value[byte_index..].to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        env, io,
        path::Path,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("shared writer")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn writer_worker_preserves_enqueued_input_order() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let (event_tx, _event_rx) = mpsc::channel(4);
        let writer = spawn_writer(
            PaneId(3),
            7,
            event_tx,
            Box::new(SharedWriter(output.clone())),
        );
        writer.send(b"first".to_vec()).expect("first write");
        writer.send(b"-second".to_vec()).expect("second write");

        let deadline = Instant::now() + Duration::from_secs(1);
        while output.lock().expect("output").len() < 12 && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(&*output.lock().expect("output"), b"first-second");
    }

    #[test]
    fn writer_worker_reports_asynchronous_failures() {
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let writer = spawn_writer(PaneId(4), 2, event_tx, Box::new(FailingWriter));
        writer.send(b"input".to_vec()).expect("queue input");

        let event = event_rx.blocking_recv().expect("write failure event");
        assert!(matches!(
            event,
            PtyEvent::WriteFailed {
                pane: PaneId(4),
                generation: 2,
                ..
            }
        ));
    }

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
    fn osc_tail_keeps_only_incomplete_sequences() {
        assert!(incomplete_osc_tail(b"ordinary output").is_empty());
        assert!(incomplete_osc_tail(b"\x1b]7;file:///tmp\x07done").is_empty());
        assert_eq!(
            incomplete_osc_tail(b"output\x1b]7;file:///tmp"),
            b"\x1b]7;file:///tmp"
        );
        assert_eq!(incomplete_osc_tail(b"output\x1b"), b"\x1b");
    }

    #[cfg(windows)]
    #[test]
    fn parses_osc7_windows_cwd() {
        let payload = b"7;file://localhost/C:/Users/Jason/My%20Repo";
        assert_eq!(
            cwd_from_osc7_payload(payload),
            Some(PathBuf::from("C:/Users/Jason/My Repo"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn parses_osc7_msys_cwd() {
        let payload = b"7;file://host/c/Users/Jason/gridbash";
        assert_eq!(
            cwd_from_osc7_payload(payload),
            Some(PathBuf::from("C:/Users/Jason/gridbash"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn parses_osc7_unix_cwd() {
        let payload = b"7;file://localhost/Users/Jason/My%20Repo";
        assert_eq!(
            cwd_from_osc7_payload(payload),
            Some(PathBuf::from("/Users/Jason/My Repo"))
        );
    }

    #[test]
    fn finds_osc_payloads_with_bel_and_st_terminators() {
        let payloads = osc_payloads(b"a\x1b]7;file://localhost/C:/one\x07b\x1b]0;title\x1b\\c");
        assert_eq!(
            payloads,
            vec![&b"7;file://localhost/C:/one"[..], &b"0;title"[..],]
        );
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
    fn scroll_screen_moves_through_scrollback_and_back_to_live_output() {
        let mut parser = Parser::new(3, 20, 100);
        let mut other_parser = Parser::new(3, 20, 100);
        parser.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        other_parser.process(b"alpha\r\nbeta\r\ngamma\r\ndelta");

        assert_eq!(parser.screen().scrollback(), 0);
        assert!(scroll_screen(parser.screen_mut(), 3));
        assert!(parser.screen().scrollback() > 0);
        assert!(parser.screen().contents().contains("one"));
        assert_eq!(other_parser.screen().scrollback(), 0);

        assert!(scroll_screen(parser.screen_mut(), -3));
        assert_eq!(parser.screen().scrollback(), 0);
        assert!(parser.screen().contents().contains("five"));
    }

    #[test]
    fn records_entered_command_lines() {
        let mut pending = String::new();
        let mut history = Vec::new();

        record_input_bytes(b"cargo test\r", &mut pending, &mut history);
        record_input_bytes(b"\x1b[A", &mut pending, &mut history);
        record_input_bytes(b"\x1bOP", &mut pending, &mut history);
        record_input_bytes(b"git status\x7f\x7f\r", &mut pending, &mut history);

        assert_eq!(history, ["cargo test", "git stat"]);
    }

    #[test]
    fn strips_escape_sequences_from_output_history() {
        let plain = plain_terminal_text(b"\x1b[31mred\x1b[0m\r\nok\x1b]0;title\x07");

        assert_eq!(plain, "red\nok");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Windows ConPTY smoke test requires an interactive console; run manually when debugging PTY I/O"]
    fn spawned_pty_receives_output_and_input() {
        let (event_tx, mut event_rx) = mpsc::channel(64);
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
            &BTreeMap::new(),
            &cwd,
            &[],
            PaneProcessPriority::BelowNormal,
            PaneWorkloadPolicy::Adaptive,
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
                    PtyEvent::WriteFailed { error, .. } => panic!("PTY write failed: {error}"),
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

    #[cfg(unix)]
    #[test]
    fn spawned_unix_pty_receives_output_and_input() {
        let (event_tx, mut event_rx) = mpsc::channel(64);
        let cwd = env::current_dir().expect("current dir");
        let mut pane = PtyPane::spawn(
            "sh",
            PaneId(0),
            0,
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                "read value; printf 'GRIDBASH_READY:%s\\n' \"$value\"".into(),
            ],
            &BTreeMap::new(),
            &cwd,
            &[],
            PaneProcessPriority::BelowNormal,
            PaneWorkloadPolicy::Adaptive,
            event_tx,
        )
        .expect("spawn Unix PTY");

        pane.write(b"typed-input\n").expect("write input to PTY");

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
                    PtyEvent::WriteFailed { error, .. } => panic!("PTY write failed: {error}"),
                }
            }

            if String::from_utf8_lossy(&raw_output).contains("GRIDBASH_READY:typed-input")
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
        panic!("Unix PTY did not round-trip input/output");
    }
}
