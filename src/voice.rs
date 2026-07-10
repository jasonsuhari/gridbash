use std::{
    io::Read,
    process::{Child, Command, ExitStatus},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::Duration,
};

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::process::Stdio;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{env, path::PathBuf};

#[cfg(target_os = "linux")]
use crate::voice_model;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(50);
const NO_SPEECH_EXIT_CODE: i32 = 2;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
const RECOGNIZE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$recognizer = $null
try {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    $null = [Windows.Media.SpeechRecognition.SpeechRecognizer, Windows.Media.SpeechRecognition, ContentType=WindowsRuntime]

    $asTaskMethod = [System.WindowsRuntimeSystemExtensions].GetMethods() |
        Where-Object {
            $_.Name -eq 'AsTask' -and
            $_.IsGenericMethod -and
            $_.GetParameters().Count -eq 1 -and
            $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
        } |
        Select-Object -First 1
    if ($null -eq $asTaskMethod) {
        throw 'could not access the Windows Runtime async bridge'
    }

    function Wait-WindowsRuntimeOperation($operation, [Type] $resultType) {
        $task = $asTaskMethod.MakeGenericMethod($resultType).Invoke($null, @($operation))
        $task.GetAwaiter().GetResult()
    }

    $recognizer = [Windows.Media.SpeechRecognition.SpeechRecognizer]::new()
    $recognizer.Timeouts.InitialSilenceTimeout = [TimeSpan]::FromSeconds(15)
    $recognizer.Timeouts.EndSilenceTimeout = [TimeSpan]::FromMilliseconds(1500)
    $recognizer.Timeouts.BabbleTimeout = [TimeSpan]::FromSeconds(15)

    $compileResult = Wait-WindowsRuntimeOperation `
        $recognizer.CompileConstraintsAsync() `
        ([Windows.Media.SpeechRecognition.SpeechRecognitionCompilationResult])
    if ($compileResult.Status -ne 'Success') {
        throw "Windows dictation grammar failed to compile: $($compileResult.Status)"
    }

    $result = Wait-WindowsRuntimeOperation `
        $recognizer.RecognizeAsync() `
        ([Windows.Media.SpeechRecognition.SpeechRecognitionResult])
    if ($result.Status -eq 'UserCanceled' -or [string]::IsNullOrWhiteSpace($result.Text)) {
        exit 2
    }
    if ($result.Status -ne 'Success') {
        throw "Windows dictation failed: $($result.Status)"
    }

    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    [Console]::Out.Write($result.Text)
} catch {
    $exception = $_.Exception
    if ($null -ne $exception.InnerException) {
        $exception = $exception.InnerException
    }
    $errorCode = '{0:X8}' -f ($exception.HResult -band 0xffffffffL)
    $message = switch ($errorCode) {
        '80045509' { 'online speech recognition is off; enable Windows Settings > Privacy & security > Speech > Online speech recognition' }
        '80070005' { 'microphone access is denied; allow desktop apps under Windows Settings > Privacy & security > Microphone' }
        'C00DABE0' { 'no microphone is available' }
        default { "modern Windows dictation failed: $($exception.Message)" }
    }
    [Console]::Error.Write($message)
    exit 1
} finally {
    if ($null -ne $recognizer) {
        try { $recognizer.Dispose() } catch {}
    }
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceOutcome {
    Transcript(String),
    NoSpeech,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceStart {
    Listening,
    DownloadApprovalRequired(&'static str),
    DownloadingModel(&'static str),
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
    model_download_armed: bool,
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
            model_download_armed: false,
        }
    }

    pub fn start(&mut self) -> VoiceStart {
        if self.is_listening() {
            return VoiceStart::Listening;
        }

        #[cfg(target_os = "linux")]
        let needs_model_download = !voice_model::model_ready();
        #[cfg(not(target_os = "linux"))]
        let needs_model_download = false;

        if needs_model_download && !self.model_download_armed {
            self.model_download_armed = true;
            return VoiceStart::DownloadApprovalRequired(voice_model_display_size());
        }
        self.model_download_armed = false;

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
        if needs_model_download {
            VoiceStart::DownloadingModel(voice_model_display_size())
        } else {
            VoiceStart::Listening
        }
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
    #[cfg(target_os = "linux")]
    if let Err(error) = voice_model::ensure_model(&cancel_rx) {
        return Some(VoiceOutcome::Error(error.to_string()));
    }

    let mut command = match recognition_command() {
        Ok(command) => command,
        Err(error) => return Some(VoiceOutcome::Error(error)),
    };

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Some(VoiceOutcome::Error(format!(
                "could not start speech recognition: {error}"
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

#[cfg(windows)]
fn recognition_command() -> Result<Command, String> {
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
    command.creation_flags(CREATE_NO_WINDOW);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn recognition_command() -> Result<Command, String> {
    let helper = macos_speech_helper().ok_or_else(|| {
        "macOS speech helper is missing; reinstall the packaged GridBash application".to_string()
    })?;
    let mut command = Command::new(helper);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

#[cfg(target_os = "macos")]
fn macos_speech_helper() -> Option<PathBuf> {
    if let Some(path) = env::var_os("GRIDBASH_SPEECH_HELPER").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }

    let executable = env::current_exe().ok()?;
    let sibling = executable.with_file_name("gridbash-speech");
    if sibling.is_file() {
        return Some(sibling);
    }

    let contents = executable.parent()?.parent()?;
    let bundled = contents
        .join("Helpers")
        .join("GridBashSpeech.app")
        .join("Contents")
        .join("MacOS")
        .join("gridbash-speech");
    bundled.is_file().then_some(bundled)
}

#[cfg(target_os = "linux")]
fn recognition_command() -> Result<Command, String> {
    let helper = linux_speech_helper().ok_or_else(|| {
        "Linux speech helper is missing; reinstall the GridBash package".to_string()
    })?;
    let model = voice_model::configured_model_path().map_err(|error| error.to_string())?;
    let mut command = Command::new(helper);
    command
        .arg("--model")
        .arg(model)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

#[cfg(target_os = "linux")]
fn linux_speech_helper() -> Option<PathBuf> {
    if let Some(path) = env::var_os("GRIDBASH_SPEECH_HELPER").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let helper = env::current_exe().ok()?.with_file_name("gridbash-voice");
    helper.is_file().then_some(helper)
}

#[cfg(target_os = "linux")]
fn voice_model_display_size() -> &'static str {
    voice_model::MODEL_DISPLAY_SIZE
}

#[cfg(not(target_os = "linux"))]
fn voice_model_display_size() -> &'static str {
    ""
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn recognition_command() -> Result<Command, String> {
    Err("voice input is not supported on this platform yet".into())
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
        VoiceOutcome::Error(format!("speech recognition exited with {status}"))
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
