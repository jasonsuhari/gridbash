use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;
use vt100::{Parser, Screen};

use crate::layout::PaneId;

const MAX_INPUT_HISTORY: usize = 200;
const MAX_INPUT_LINE_CHARS: usize = 4096;
const MAX_OUTPUT_TAIL_CHARS: usize = 40_000;
const MAX_REPLAY_OUTPUT_CHARS: usize = 18_000;

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
    input_history: Vec<String>,
    pending_input: String,
    output_tail: String,
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
            input_history: Vec::new(),
            pending_input: String::new(),
            output_tail: String::new(),
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
        self.append_plain_output(bytes);
        self.active = true;
        self.answer_terminal_queries(bytes);
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
        }
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

    fn append_plain_output(&mut self, bytes: &[u8]) {
        let plain = plain_terminal_text(bytes);
        if plain.is_empty() {
            return;
        }

        self.output_tail.push_str(&plain);
        trim_string_tail(&mut self.output_tail, MAX_OUTPUT_TAIL_CHARS);
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

fn plain_terminal_text(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let mut plain = String::new();
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        if ch == '\x1b' {
            index = skip_escape_chars(&chars, index);
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
        index += 1;
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

fn skip_escape_chars(chars: &[char], start: usize) -> usize {
    let mut index = start.saturating_add(1);
    if index >= chars.len() {
        return index;
    }

    match chars[index] {
        '[' => {
            index += 1;
            while index < chars.len() {
                let ch = chars[index];
                index += 1;
                if ('\u{40}'..='\u{7e}').contains(&ch) {
                    break;
                }
            }
        }
        'O' => {
            index += 1;
            if index < chars.len() {
                index += 1;
            }
        }
        ']' => {
            index += 1;
            while index < chars.len() {
                if chars[index] == '\x07' {
                    index += 1;
                    break;
                }
                if chars[index] == '\x1b' && index + 1 < chars.len() && chars[index + 1] == '\\' {
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
