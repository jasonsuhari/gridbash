use std::{
    borrow::Cow,
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;
use vt100::{Parser, Screen};

use crate::{
    codex_sqlite::CodexSqliteLease,
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
const CHILD_REAP_GRACE: Duration = Duration::from_millis(100);
const CHILD_REAP_AFTER_FORCE: Duration = Duration::from_millis(400);
const CHILD_REAP_POLL_INTERVAL: Duration = Duration::from_millis(10);

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
        token: Option<PtyWriteToken>,
        error: String,
    },
    WriteSucceeded {
        pane: PaneId,
        generation: u64,
        token: PtyWriteToken,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PtyWriteToken(pub u128);

#[derive(Debug)]
struct PtyWrite {
    bytes: Vec<u8>,
    token: Option<PtyWriteToken>,
}

impl PtyWrite {
    fn untracked(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            token: None,
        }
    }

    fn tracked(bytes: &[u8], token: PtyWriteToken) -> Self {
        Self {
            bytes: bytes.to_vec(),
            token: Some(token),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PtyWriterStatus {
    Open,
    Failed,
}

#[derive(Debug)]
struct PtyWriterQueue {
    sender: SyncSender<PtyWrite>,
    status: Arc<Mutex<PtyWriterStatus>>,
}

impl PtyWriterQueue {
    fn try_send(&self, write: PtyWrite) -> std::result::Result<(), TrySendError<PtyWrite>> {
        let status = self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *status == PtyWriterStatus::Failed {
            return Err(TrySendError::Disconnected(write));
        }
        self.sender.try_send(write)
    }
}

struct SpawnGuard<L = CodexSqliteLease> {
    child: Box<dyn Child + Send>,
    _lease: Option<L>,
    armed: bool,
}

impl<L> SpawnGuard<L> {
    fn new(child: Box<dyn Child + Send>, lease: Option<L>) -> Self {
        Self {
            child,
            _lease: lease,
            armed: true,
        }
    }

    fn child(&self) -> &(dyn Child + Send) {
        self.child.as_ref()
    }

    fn child_mut(&mut self) -> &mut (dyn Child + Send) {
        self.child.as_mut()
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<L> Drop for SpawnGuard<L> {
    fn drop(&mut self) {
        if self.armed {
            terminate_and_reap_spawn_failure(self.child.as_mut());
        }
    }
}

fn terminate_and_reap_spawn_failure(child: &mut (dyn Child + Send)) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }

    let _ = child.kill();
    if reap_spawn_failure_for(child, CHILD_REAP_GRACE) {
        return;
    }

    #[cfg(unix)]
    if let Some(process_id) = child.process_id() {
        // SAFETY: the PID comes from the owned child process. SIGKILL is the
        // fallback when portable-pty's SIGHUP did not stop it.
        let _ = unsafe { libc::kill(process_id as i32, libc::SIGKILL) };
    }

    if !reap_spawn_failure_for(child, CHILD_REAP_AFTER_FORCE) {
        // This cleanup runs only while spawn construction is failing. Waiting
        // here keeps the SQLite lease alive until the child has been reaped.
        let _ = child.wait();
    }
}

fn reap_spawn_failure_for(child: &mut (dyn Child + Send), timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep(CHILD_REAP_POLL_INTERVAL.min(deadline - now));
    }
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
    child: SpawnGuard,
    writer: PtyWriterQueue,
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
    input_revision: u64,
    output_tail: String,
    output_tail_chars: usize,
    pub active: bool,
    pub exited: bool,
    child_reaped: bool,
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
        extra_env: &[(OsString, OsString)],
        codex_sqlite_lease: Option<CodexSqliteLease>,
        scrollback_rows: usize,
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
        let child = SpawnGuard::new(child, codex_sqlite_lease);
        let (workload, workload_error) = match child.child().process_id() {
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

        let mut pane = Self {
            id,
            generation,
            master: pair.master,
            child,
            writer,
            workload,
            workload_error,
            parser: Parser::new(24, 80, scrollback_rows.clamp(1_000, 50_000)),
            screen_revision: 0,
            cwd: cwd.to_path_buf(),
            rows: 24,
            cols: 80,
            response_scan_tail: Vec::new(),
            output_activity: OutputActivity::default(),
            osc_scan_tail: Vec::new(),
            input_history: Vec::new(),
            pending_input: String::new(),
            input_revision: 0,
            output_tail: String::new(),
            output_tail_chars: 0,
            active: false,
            exited: false,
            child_reaped: false,
        };
        pane.child.disarm();
        Ok(pane)
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
        let (plain, plain_chars) = plain_terminal_text_with_char_count(bytes);
        self.append_plain_output(&plain, plain_chars);
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
        self.record_input_activity(bytes);
        record_input_bytes(bytes, &mut self.pending_input, &mut self.input_history);
    }

    pub fn record_input_activity(&mut self, bytes: &[u8]) {
        advance_input_revision(&mut self.input_revision, bytes);
    }

    pub fn input_revision(&self) -> u64 {
        self.input_revision
    }

    pub fn has_pending_input(&self) -> bool {
        !self.pending_input.is_empty()
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
        self.enqueue_write(PtyWrite::untracked(bytes))
    }

    pub fn write_tracked(&self, bytes: &[u8], token: PtyWriteToken) -> Result<()> {
        self.enqueue_write(PtyWrite::tracked(bytes, token))
    }

    fn enqueue_write(&self, write: PtyWrite) -> Result<()> {
        match self.writer.try_send(write) {
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

        if self.reap_child_now() {
            return true;
        }

        false
    }

    fn answer_terminal_queries(&mut self, bytes: &[u8]) {
        if self.response_scan_tail.is_empty() && !bytes.contains(&0x1b) {
            return;
        }
        let scan = if self.response_scan_tail.is_empty() {
            Cow::Borrowed(bytes)
        } else {
            let mut scan = Vec::with_capacity(self.response_scan_tail.len() + bytes.len());
            scan.extend_from_slice(&self.response_scan_tail);
            scan.extend_from_slice(bytes);
            Cow::Owned(scan)
        };

        if contains_sequence(scan.as_ref(), DEVICE_STATUS_QUERY) {
            let _ = self.write(b"\x1b[0n");
        }

        let cursor_position_requests = count_sequence(scan.as_ref(), CURSOR_POSITION_QUERY);
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

        if contains_sequence(scan.as_ref(), PRIMARY_DEVICE_ATTRIBUTES_QUERY)
            || contains_sequence(scan.as_ref(), PRIMARY_DEVICE_ATTRIBUTES_ZERO_QUERY)
        {
            let _ = self.write(b"\x1b[?1;2c");
        }

        self.response_scan_tail = terminal_query_scan_tail(scan.as_ref());
    }

    fn update_cwd_from_osc7(&mut self, bytes: &[u8]) {
        if self.osc_scan_tail.is_empty() && !bytes.contains(&0x1b) {
            return;
        }
        let scan = if self.osc_scan_tail.is_empty() {
            Cow::Borrowed(bytes)
        } else {
            let mut scan = Vec::with_capacity(self.osc_scan_tail.len() + bytes.len());
            scan.extend_from_slice(&self.osc_scan_tail);
            scan.extend_from_slice(bytes);
            Cow::Owned(scan)
        };

        if find_sequence(scan.as_ref(), b"\x1b]").is_some() {
            for payload in osc_payloads(scan.as_ref()) {
                if let Some(path) = cwd_from_osc7_payload(payload) {
                    self.cwd = path;
                }
            }
        }

        self.osc_scan_tail = incomplete_osc_tail(scan.as_ref());
    }

    pub fn terminate(&mut self) {
        if self.child_reaped {
            return;
        }

        if self.reap_child_now() {
            return;
        }

        let _ = self.child.child_mut().kill();
        if self.reap_child_for(CHILD_REAP_GRACE) {
            return;
        }

        #[cfg(unix)]
        if let Some(process_id) = self.child.child().process_id() {
            // SAFETY: the PID comes from the owned child process. SIGKILL is the
            // bounded fallback after portable-pty's SIGHUP did not reap it.
            let _ = unsafe { libc::kill(process_id as i32, libc::SIGKILL) };
        }

        let _ = self.reap_child_for(CHILD_REAP_AFTER_FORCE);
        self.exited = true;
    }

    fn reap_child_now(&mut self) -> bool {
        if self.child_reaped {
            return true;
        }
        if matches!(self.child.child_mut().try_wait(), Ok(Some(_))) {
            self.child_reaped = true;
            self.exited = true;
            return true;
        }
        false
    }

    fn reap_child_for(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.reap_child_now() {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            thread::sleep(CHILD_REAP_POLL_INTERVAL.min(deadline - now));
        }
    }

    fn append_plain_output(&mut self, plain: &str, plain_chars: usize) {
        if plain.is_empty() {
            return;
        }

        self.output_tail.push_str(plain);
        self.output_tail_chars += plain_chars;
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
) -> PtyWriterQueue {
    let (tx, rx) = sync_channel::<PtyWrite>(PTY_WRITE_QUEUE_MESSAGES);
    let status = Arc::new(Mutex::new(PtyWriterStatus::Open));
    let worker_status = status.clone();
    thread::spawn(move || {
        while let Ok(write) = rx.recv() {
            let result = writer
                .write_all(&write.bytes)
                .and_then(|()| writer.flush())
                .context("failed to write to PTY");
            match result {
                Ok(()) => {
                    if let Some(token) = write.token {
                        let _ = event_tx.blocking_send(PtyEvent::WriteSucceeded {
                            pane,
                            generation,
                            token,
                        });
                    }
                }
                Err(error) => {
                    let error = format!("{error:#}");
                    let queued = fail_writer_queue(&worker_status, &rx);
                    let _ = event_tx.blocking_send(PtyEvent::WriteFailed {
                        pane,
                        generation,
                        token: write.token,
                        error: error.clone(),
                    });
                    for queued in queued.into_iter().filter(|queued| queued.token.is_some()) {
                        let _ = event_tx.blocking_send(PtyEvent::WriteFailed {
                            pane,
                            generation,
                            token: queued.token,
                            error: format!(
                                "PTY writer stopped before queued input was written: {error}"
                            ),
                        });
                    }
                    break;
                }
            }
        }
    });
    PtyWriterQueue { sender: tx, status }
}

fn fail_writer_queue(
    status: &Mutex<PtyWriterStatus>,
    receiver: &Receiver<PtyWrite>,
) -> Vec<PtyWrite> {
    let mut status = status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *status = PtyWriterStatus::Failed;
    receiver.try_iter().collect()
}

fn advance_input_revision(revision: &mut u64, bytes: &[u8]) {
    if !bytes.is_empty() {
        *revision = revision.wrapping_add(1);
    }
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

#[cfg(test)]
fn plain_terminal_text(bytes: &[u8]) -> String {
    plain_terminal_text_with_char_count(bytes).0
}

fn plain_terminal_text_with_char_count(bytes: &[u8]) -> (String, usize) {
    let raw = String::from_utf8_lossy(bytes);
    let bytes = raw.as_bytes();
    let mut plain = String::with_capacity(bytes.len());
    let mut plain_chars = 0;
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            0x1b => index = skip_escape_sequence(bytes, index),
            b'\r' | b'\n' => {
                if !plain.ends_with('\n') {
                    plain.push('\n');
                    plain_chars += 1;
                }
                index += 1;
            }
            b'\t' => {
                plain.push('\t');
                plain_chars += 1;
                index += 1;
            }
            0x08 => {
                if plain.pop().is_some() {
                    plain_chars -= 1;
                }
                index += 1;
            }
            byte if byte.is_ascii_control() => index += 1,
            byte if byte.is_ascii() => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && bytes[index].is_ascii()
                    && !bytes[index].is_ascii_control()
                {
                    index += 1;
                }
                plain.push_str(&raw[start..index]);
                plain_chars += index - start;
            }
            _ => {
                let ch = raw[index..]
                    .chars()
                    .next()
                    .expect("index remains on a UTF-8 character boundary");
                if !ch.is_control() {
                    plain.push(ch);
                    plain_chars += 1;
                }
                index += ch.len_utf8();
            }
        }
    }

    (plain, plain_chars)
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
        collections::BTreeSet,
        env, io,
        path::Path,
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    #[test]
    #[ignore = "manual performance benchmark"]
    fn benchmark_plain_terminal_text() {
        use std::hint::black_box;

        const ITERATIONS: usize = 10_000;
        let payload = (0..128)
            .map(|index| {
                format!(
                    "\x1b[38;5;{}mGridBash output {index:03}: compile passed in 1.23s — 東京\x1b[0m\r\n",
                    32 + index % 160
                )
            })
            .collect::<String>();
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(plain_terminal_text(black_box(payload.as_bytes())));
        }
        let elapsed = start.elapsed();
        eprintln!(
            "plain terminal text: {ITERATIONS} iterations in {elapsed:?} ({:?}/iteration)",
            elapsed / ITERATIONS as u32
        );
    }

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

    #[derive(Debug)]
    struct GuardTestChild {
        events: Arc<Mutex<Vec<&'static str>>>,
        exited: bool,
    }

    impl portable_pty::ChildKiller for GuardTestChild {
        fn kill(&mut self) -> io::Result<()> {
            self.events.lock().expect("guard events").push("kill");
            self.exited = true;
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(GuardTestKiller)
        }
    }

    impl Child for GuardTestChild {
        fn try_wait(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
            let event = if self.exited { "reaped" } else { "running" };
            self.events.lock().expect("guard events").push(event);
            Ok(self
                .exited
                .then(|| portable_pty::ExitStatus::with_exit_code(0)))
        }

        fn wait(&mut self) -> io::Result<portable_pty::ExitStatus> {
            self.events.lock().expect("guard events").push("waited");
            self.exited = true;
            Ok(portable_pty::ExitStatus::with_exit_code(0))
        }

        fn process_id(&self) -> Option<u32> {
            None
        }

        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }

    impl Drop for GuardTestChild {
        fn drop(&mut self) {
            self.events.lock().expect("guard events").push("child_drop");
        }
    }

    #[derive(Debug)]
    struct GuardTestKiller;

    impl portable_pty::ChildKiller for GuardTestKiller {
        fn kill(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(Self)
        }
    }

    struct GuardTestLease(Arc<Mutex<Vec<&'static str>>>);

    impl Drop for GuardTestLease {
        fn drop(&mut self) {
            self.0.lock().expect("guard events").push("lease_drop");
        }
    }

    #[test]
    fn spawn_guard_reaps_child_before_releasing_lease() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let child = Box::new(GuardTestChild {
            events: Arc::clone(&events),
            exited: false,
        });

        drop(SpawnGuard::new(
            child,
            Some(GuardTestLease(Arc::clone(&events))),
        ));

        assert_eq!(
            *events.lock().expect("guard events"),
            ["running", "kill", "reaped", "child_drop", "lease_drop"]
        );
    }

    #[test]
    fn disarmed_spawn_guard_leaves_child_cleanup_to_pane() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let child = Box::new(GuardTestChild {
            events: Arc::clone(&events),
            exited: false,
        });
        let mut guard = SpawnGuard::new(child, Some(GuardTestLease(Arc::clone(&events))));

        guard.disarm();
        drop(guard);

        assert_eq!(
            *events.lock().expect("guard events"),
            ["child_drop", "lease_drop"]
        );
    }

    struct GatedFailingWriter {
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }

    impl Write for GatedFailingWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            self.entered.send(()).expect("signal writer entry");
            self.release.recv().expect("release failing writer");
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
        writer
            .try_send(PtyWrite::untracked(b"first"))
            .expect("first write");
        writer
            .try_send(PtyWrite::untracked(b"-second"))
            .expect("second write");

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
        let token = PtyWriteToken(42);
        writer
            .try_send(PtyWrite::tracked(b"input", token))
            .expect("queue input");

        let event = event_rx.blocking_recv().expect("write failure event");
        assert!(matches!(
            event,
            PtyEvent::WriteFailed {
                pane: PaneId(4),
                generation: 2,
                token: Some(value),
                ..
            } if value == token
        ));
    }

    #[test]
    fn writer_failure_fails_all_accepted_tokens_and_closes_enqueue_race() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let writer = spawn_writer(
            PaneId(6),
            4,
            event_tx,
            Box::new(GatedFailingWriter {
                entered: entered_tx,
                release: release_rx,
            }),
        );
        let first = PtyWriteToken(101);
        let queued = PtyWriteToken(102);
        writer
            .try_send(PtyWrite::tracked(b"first", first))
            .expect("queue failing write");
        entered_rx.recv().expect("writer entered write");
        writer
            .try_send(PtyWrite::tracked(b"queued", queued))
            .expect("queue tracked write before shutdown");
        release_tx.send(()).expect("release writer failure");

        let mut failed = BTreeSet::new();
        for _ in 0..2 {
            let event = event_rx.blocking_recv().expect("tracked failure event");
            let PtyEvent::WriteFailed {
                pane: PaneId(6),
                generation: 4,
                token: Some(token),
                ..
            } = event
            else {
                panic!("expected tracked write failure, got {event:?}");
            };
            failed.insert(token);
        }
        assert_eq!(failed, BTreeSet::from([first, queued]));

        let after_failure = PtyWriteToken(103);
        assert!(matches!(
            writer.try_send(PtyWrite::tracked(b"too late", after_failure)),
            Err(TrySendError::Disconnected(PtyWrite {
                token: Some(token),
                ..
            })) if token == after_failure
        ));
    }

    #[test]
    fn writer_worker_acknowledges_tracked_input_after_flush() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let writer = spawn_writer(
            PaneId(5),
            3,
            event_tx,
            Box::new(SharedWriter(output.clone())),
        );
        let token = PtyWriteToken(99);
        writer
            .try_send(PtyWrite::tracked(b"tracked", token))
            .expect("queue tracked input");

        let event = event_rx.blocking_recv().expect("write acknowledgement");
        assert!(matches!(
            event,
            PtyEvent::WriteSucceeded {
                pane: PaneId(5),
                generation: 3,
                token: value,
            } if value == token
        ));
        assert_eq!(&*output.lock().expect("output"), b"tracked");
    }

    #[test]
    fn input_activity_revision_can_skip_command_history() {
        let mut revision = 7;
        advance_input_revision(&mut revision, b"\x1b[<65;10;4M");
        assert_eq!(revision, 8);

        advance_input_revision(&mut revision, b"");
        assert_eq!(revision, 8);
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

    #[test]
    fn plain_output_character_count_matches_filtered_unicode() {
        for input in [
            "plain ASCII output",
            "Tokyo 東京 — ready",
            "erase 東京\u{8}\u{8}京",
            "\u{1b}[32mgreen\u{1b}[0m\r\nnext",
            "visible\u{85}control",
        ] {
            let (plain, count) = plain_terminal_text_with_char_count(input.as_bytes());
            assert_eq!(count, plain.chars().count(), "input: {input:?}");
        }
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
            None,
            10_000,
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
                    PtyEvent::WriteSucceeded { .. } => {}
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
            None,
            10_000,
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
                    PtyEvent::WriteSucceeded { .. } => {}
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
