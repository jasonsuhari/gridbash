use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(50);
const NO_SPEECH_EXIT_CODE: i32 = 2;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const RECOGNIZE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$recognizer = $null
try {
    Add-Type -AssemblyName System.Speech
    $recognizer = New-Object System.Speech.Recognition.SpeechRecognitionEngine
    $recognizer.LoadGrammar((New-Object System.Speech.Recognition.DictationGrammar))
    $recognizer.SetInputToDefaultAudioDevice()
    $result = $recognizer.Recognize([TimeSpan]::FromSeconds(15))
    if ($null -eq $result) { exit 2 }
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    [Console]::Out.Write($result.Text)
} catch {
    [Console]::Error.Write($_.Exception.Message)
    exit 1
} finally {
    if ($null -ne $recognizer) { $recognizer.Dispose() }
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceOutcome {
    Transcript(String),
    NoSpeech,
    Error(String),
}

#[derive(Debug)]
struct VoiceEvent {
    request_id: u64,
    outcome: VoiceOutcome,
}

pub struct VoiceInput {
    event_tx: Sender<VoiceEvent>,
    event_rx: Receiver<VoiceEvent>,
    cancel_tx: Option<Sender<()>>,
    active_request: Option<u64>,
    next_request: u64,
}

impl VoiceInput {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            event_tx,
            event_rx,
            cancel_tx: None,
            active_request: None,
            next_request: 0,
        }
    }

    pub fn start(&mut self) {
        if self.is_listening() {
            return;
        }

        let request_id = self.next_request;
        self.next_request = self.next_request.wrapping_add(1);
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let event_tx = self.event_tx.clone();

        thread::spawn(move || {
            if let Some(outcome) = recognize(cancel_rx) {
                let _ = event_tx.send(VoiceEvent {
                    request_id,
                    outcome,
                });
            }
        });

        self.cancel_tx = Some(cancel_tx);
        self.active_request = Some(request_id);
    }

    pub fn cancel(&mut self) -> bool {
        let Some(cancel_tx) = self.cancel_tx.take() else {
            return false;
        };
        let _ = cancel_tx.send(());
        self.active_request = None;
        true
    }

    pub fn is_listening(&self) -> bool {
        self.active_request.is_some()
    }

    pub fn poll(&mut self) -> Option<VoiceOutcome> {
        while let Ok(event) = self.event_rx.try_recv() {
            if self.active_request == Some(event.request_id) {
                self.active_request = None;
                self.cancel_tx = None;
                return Some(event.outcome);
            }
        }
        None
    }
}

impl Default for VoiceInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for VoiceInput {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn recognize(cancel_rx: Receiver<()>) -> Option<VoiceOutcome> {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            RECOGNIZE_SCRIPT,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Some(VoiceOutcome::Error(format!(
                "could not start Windows speech recognition: {error}"
            )));
        }
    };

    loop {
        match cancel_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => {
                terminate(&mut child);
                return None;
            }
            Err(TryRecvError::Empty) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => return Some(read_outcome(&mut child, status)),
            Ok(None) => thread::sleep(PROCESS_POLL_INTERVAL),
            Err(error) => {
                terminate(&mut child);
                return Some(VoiceOutcome::Error(format!(
                    "speech recognition failed: {error}"
                )));
            }
        }
    }
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn read_outcome(child: &mut Child, status: ExitStatus) -> VoiceOutcome {
    let stdout = read_pipe(child.stdout.take());
    let stderr = read_pipe(child.stderr.take());

    if status.success() {
        let transcript = normalize_transcript(&stdout);
        return if transcript.is_empty() {
            VoiceOutcome::NoSpeech
        } else {
            VoiceOutcome::Transcript(transcript)
        };
    }

    if status.code() == Some(NO_SPEECH_EXIT_CODE) {
        return VoiceOutcome::NoSpeech;
    }

    let detail = normalize_transcript(&stderr);
    if detail.is_empty() {
        VoiceOutcome::Error(format!("Windows speech recognition exited with {status}"))
    } else {
        VoiceOutcome::Error(detail)
    }
}

fn read_pipe<R: Read>(pipe: Option<R>) -> String {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn normalize_transcript(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_normalization_removes_transport_whitespace() {
        assert_eq!(
            normalize_transcript("  hello\r\n   voice   mode  "),
            "hello voice mode"
        );
    }

    #[test]
    fn completed_request_returns_one_result_and_becomes_idle() {
        let mut input = VoiceInput::new();
        input.active_request = Some(7);
        input
            .event_tx
            .send(VoiceEvent {
                request_id: 7,
                outcome: VoiceOutcome::Transcript("hello".into()),
            })
            .expect("voice event");

        assert!(input.is_listening());
        assert_eq!(input.poll(), Some(VoiceOutcome::Transcript("hello".into())));
        assert!(!input.is_listening());
        assert_eq!(input.poll(), None);
    }
}
