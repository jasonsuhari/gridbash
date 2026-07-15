use std::{
    collections::BTreeMap,
    ffi::{OsString, OsString as PlatformString},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc as std_mpsc},
    thread,
    time::{Duration, Instant},
};
use std::{error::Error as StdError, fmt};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use vt100::Screen;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use crate::{
    codex_sqlite::{CodexSqlitePool, PaneCodexSqlite},
    config::{PaneProcessPriority, PaneWorkloadPolicy},
    layout::PaneId,
    process_priority::PaneWorkloadClass,
    pty::{PtyEvent, PtyPane as LocalPtyPane, PtyView, PtyWriteToken},
};

const HOST_START_TIMEOUT: Duration = Duration::from_secs(8);
const HOST_POLL_INTERVAL: Duration = Duration::from_millis(10);
const HOST_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const BACKGROUND_OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const PANE_HOST_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyHostRef {
    pub endpoint: String,
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HostLaunchSpec {
    token: String,
    ready_path: PathBuf,
    profile_name: String,
    pane_id: usize,
    generation: u64,
    command: PathBuf,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: PathBuf,
    extra_env: Vec<(String, String)>,
    scrollback_rows: usize,
    process_priority: PaneProcessPriority,
    workload_policy: PaneWorkloadPolicy,
    keep_running: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct HostReadyFile {
    endpoint: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostCommand {
    Hello {
        protocol_version: u16,
        token: String,
        keep_running: bool,
    },
    Write {
        data: String,
        #[serde(default)]
        token: Option<String>,
    },
    Resize {
        rows: u16,
        cols: u16,
    },
    ApplyWorkload {
        policy: PaneWorkloadPolicy,
        class: PaneWorkloadClass,
    },
    SetKeepRunning {
        keep_running: bool,
    },
    Disconnect {
        keep_running: bool,
    },
    Terminate,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostEvent {
    Ready {
        #[serde(default)]
        protocol_version: u16,
        background_output: String,
        cwd: PathBuf,
        exited: bool,
    },
    Output {
        data: String,
    },
    Exited,
    WriteFailed {
        #[serde(default)]
        token: Option<String>,
        error: String,
    },
    WriteSucceeded {
        token: String,
    },
    Error {
        error: String,
    },
    Incompatible {
        host_version: u16,
    },
    Busy,
}

#[derive(Debug)]
pub struct PaneHostBusy;

impl fmt::Display for PaneHostBusy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("pane host already has an attached GridBash client")
    }
}

impl StdError for PaneHostBusy {}

#[derive(Debug)]
pub struct PaneHostIncompatible {
    host_version: u16,
}

impl fmt::Display for PaneHostIncompatible {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "pane host protocol mismatch: client {}, host {}; reopen with the previous GridBash version or stop the old host",
            PANE_HOST_PROTOCOL_VERSION, self.host_version
        )
    }
}

impl StdError for PaneHostIncompatible {}

#[derive(Debug)]
pub struct PaneHostRejected {
    reason: String,
}

impl fmt::Display for PaneHostRejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "pane host rejected the connection: {}",
            self.reason
        )
    }
}

impl StdError for PaneHostRejected {}

pub struct PtyPane {
    id: PaneId,
    generation: u64,
    connection: Arc<Mutex<TcpStream>>,
    host: PtyHostRef,
    view: PtyView,
    keep_running: bool,
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
        extra_env: &[(OsString, OsString)],
        scrollback_rows: usize,
        process_priority: PaneProcessPriority,
        workload_policy: PaneWorkloadPolicy,
        keep_running: bool,
        event_tx: mpsc::Sender<PtyEvent>,
    ) -> Result<Self> {
        let token = random_token()?;
        let directory = pane_hosts_dir()?;
        create_private_dir_all(&directory).with_context(|| {
            format!(
                "failed to create pane host directory {}",
                directory.display()
            )
        })?;
        let spec_path = directory.join(format!("{token}.json"));
        let ready_path = directory.join(format!("{token}.ready.json"));
        let spec = HostLaunchSpec {
            token: token.clone(),
            ready_path: ready_path.clone(),
            profile_name: profile_name.to_string(),
            pane_id: id.0,
            generation,
            command: command.to_path_buf(),
            args: args.to_vec(),
            env: env.clone(),
            cwd: cwd.to_path_buf(),
            extra_env: extra_env
                .iter()
                .map(|(name, value)| {
                    (
                        name.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
                .collect(),
            scrollback_rows,
            process_priority,
            workload_policy,
            keep_running,
        };
        let raw = serde_json::to_vec(&spec).context("failed to serialize pane host launch")?;
        write_private_file(&spec_path, &raw)
            .with_context(|| format!("failed to write pane host spec {}", spec_path.display()))?;

        let mut child = match spawn_detached_host(&spec_path) {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_file(&spec_path);
                return Err(error);
            }
        };

        let deadline = Instant::now() + HOST_START_TIMEOUT;
        let ready = loop {
            if let Ok(raw) = fs::read(&ready_path)
                && let Ok(ready) = serde_json::from_slice::<HostReadyFile>(&raw)
            {
                break ready;
            }
            if let Some(status) = child
                .try_wait()
                .context("failed to inspect pane host startup")?
            {
                let _ = fs::remove_file(&spec_path);
                let _ = fs::remove_file(&ready_path);
                bail!("pane host exited during startup with {status}");
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = fs::remove_file(&spec_path);
                let _ = fs::remove_file(&ready_path);
                bail!("timed out waiting for pane host startup");
            }
            thread::sleep(Duration::from_millis(20));
        };
        let _ = fs::remove_file(&ready_path);
        thread::spawn(move || {
            let _ = child.wait();
        });

        let host = PtyHostRef {
            endpoint: ready.endpoint,
            token,
        };
        Self::connect(
            host,
            id,
            generation,
            cwd,
            scrollback_rows,
            keep_running,
            "",
            &[],
            event_tx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn attach(
        host: PtyHostRef,
        id: PaneId,
        generation: u64,
        fallback_cwd: &Path,
        scrollback_rows: usize,
        keep_running: bool,
        saved_output_tail: &str,
        saved_input_history: &[String],
        event_tx: mpsc::Sender<PtyEvent>,
    ) -> Result<Self> {
        Self::connect(
            host,
            id,
            generation,
            fallback_cwd,
            scrollback_rows,
            keep_running,
            saved_output_tail,
            saved_input_history,
            event_tx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn connect(
        host: PtyHostRef,
        id: PaneId,
        generation: u64,
        fallback_cwd: &Path,
        scrollback_rows: usize,
        keep_running: bool,
        saved_output_tail: &str,
        saved_input_history: &[String],
        event_tx: mpsc::Sender<PtyEvent>,
    ) -> Result<Self> {
        let endpoint = host
            .endpoint
            .parse::<SocketAddr>()
            .with_context(|| format!("invalid pane host endpoint {}", host.endpoint))?;
        let mut stream = TcpStream::connect_timeout(&endpoint, HANDSHAKE_TIMEOUT)
            .with_context(|| format!("failed to connect to pane host {}", host.endpoint))?;
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok();
        send_json(
            &mut stream,
            &HostCommand::Hello {
                protocol_version: PANE_HOST_PROTOCOL_VERSION,
                token: host.token.clone(),
                keep_running,
            },
        )?;

        let reader_stream = stream
            .try_clone()
            .context("failed to clone pane host connection")?;
        let mut reader = BufReader::new(reader_stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("failed to read pane host handshake")?;
        if line.is_empty() {
            bail!("pane host closed during handshake");
        }
        let ready = serde_json::from_str::<HostEvent>(&line)
            .context("failed to parse pane host handshake")?;
        let (protocol_version, background_output, cwd, exited) = match ready {
            HostEvent::Ready {
                protocol_version,
                background_output,
                cwd,
                exited,
            } => (protocol_version, background_output, cwd, exited),
            HostEvent::Error { error } => {
                return Err(PaneHostRejected { reason: error }.into());
            }
            HostEvent::Busy => return Err(PaneHostBusy.into()),
            HostEvent::Incompatible { host_version } => {
                return Err(PaneHostIncompatible { host_version }.into());
            }
            _ => bail!("pane host rejected the connection"),
        };
        if protocol_version != PANE_HOST_PROTOCOL_VERSION {
            return Err(PaneHostIncompatible {
                host_version: protocol_version,
            }
            .into());
        }

        reader.get_mut().set_read_timeout(None).ok();
        let mut view = PtyView::new(fallback_cwd.to_path_buf(), scrollback_rows);
        view.restore_history_display(saved_output_tail, saved_input_history);
        view.set_cwd(cwd);
        let background_output = BASE64
            .decode(background_output)
            .context("failed to decode pane host background output")?;
        if !background_output.is_empty() {
            view.process_output(&background_output);
        }

        spawn_client_reader(reader, id, generation, event_tx);
        Ok(Self {
            id,
            generation,
            connection: Arc::new(Mutex::new(stream)),
            host,
            view,
            keep_running,
            active: !background_output.is_empty(),
            exited,
        })
    }

    pub fn id(&self) -> PaneId {
        self.id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn host_ref(&self) -> PtyHostRef {
        self.host.clone()
    }

    pub fn cwd(&self) -> &Path {
        self.view.cwd()
    }

    pub fn screen(&self) -> &Screen {
        self.view.screen()
    }

    pub fn screen_revision(&self) -> u64 {
        self.view.screen_revision()
    }

    pub fn scroll_view(&mut self, rows: isize) -> bool {
        self.view.scroll_view(rows)
    }

    pub fn reset_view(&mut self) -> bool {
        self.view.reset_view()
    }

    pub fn process_output(&mut self, bytes: &[u8]) -> String {
        let plain = self.view.process_output(bytes);
        self.active = true;
        plain
    }

    pub fn output_quiet(&self) -> bool {
        !self.exited && self.view.output_quiet()
    }

    pub fn refresh_output_activity(&mut self, now: Instant, quiet_after: Duration) -> bool {
        if self.exited {
            return false;
        }
        self.view.refresh_output_activity(now, quiet_after)
    }

    pub fn record_input(&mut self, bytes: &[u8]) {
        self.view.record_input(bytes);
    }

    pub fn record_input_activity(&mut self, bytes: &[u8]) {
        self.view.record_input_activity(bytes);
    }

    pub fn input_revision(&self) -> u64 {
        self.view.input_revision()
    }

    pub fn input_history(&self) -> &[String] {
        self.view.input_history()
    }

    pub fn output_tail(&self) -> &str {
        self.view.output_tail()
    }

    pub fn restore_history_display(&mut self, output_tail: &str, input_history: &[String]) {
        self.view
            .restore_history_display(output_tail, input_history);
    }

    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        self.send(&HostCommand::Write {
            data: BASE64.encode(bytes),
            token: None,
        })
    }

    pub fn write_tracked(&self, bytes: &[u8], token: PtyWriteToken) -> Result<()> {
        self.send(&HostCommand::Write {
            data: BASE64.encode(bytes),
            token: Some(token.0.to_string()),
        })
    }

    pub fn apply_workload(
        &self,
        policy: PaneWorkloadPolicy,
        class: PaneWorkloadClass,
    ) -> Result<()> {
        self.send(&HostCommand::ApplyWorkload { policy, class })
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if !self.view.resize_view(rows, cols) {
            return Ok(());
        }
        self.send(&HostCommand::Resize { rows, cols })
    }

    pub fn poll_exit(&mut self) -> bool {
        false
    }

    pub fn set_keep_running(&mut self, keep_running: bool) -> Result<()> {
        self.keep_running = keep_running;
        self.send(&HostCommand::SetKeepRunning { keep_running })
    }

    fn send(&self, command: &HostCommand) -> Result<()> {
        let mut stream = self
            .connection
            .lock()
            .map_err(|_| anyhow!("pane host connection lock was poisoned"))?;
        send_json(&mut stream, command)
    }
}

impl Drop for PtyPane {
    fn drop(&mut self) {
        let command = if self.exited {
            HostCommand::Terminate
        } else {
            HostCommand::Disconnect {
                keep_running: self.keep_running,
            }
        };
        let _ = self.send(&command);
        if let Ok(stream) = self.connection.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

pub fn run_pane_host(spec_path: &Path) -> Result<()> {
    let raw = fs::read(spec_path)
        .with_context(|| format!("failed to read pane host spec {}", spec_path.display()))?;
    let spec = serde_json::from_slice::<HostLaunchSpec>(&raw)
        .context("failed to parse pane host launch spec")?;
    let _ = fs::remove_file(spec_path);

    let listener = TcpListener::bind(("127.0.0.1", 0)).context("failed to bind pane host")?;
    listener
        .set_nonblocking(true)
        .context("failed to make pane host listener nonblocking")?;
    let endpoint = listener
        .local_addr()
        .context("failed to read pane host endpoint")?;
    let ready = serde_json::to_vec(&HostReadyFile {
        endpoint: endpoint.to_string(),
    })
    .context("failed to serialize pane host readiness")?;
    write_private_file(&spec.ready_path, &ready).with_context(|| {
        format!(
            "failed to write pane host readiness {}",
            spec.ready_path.display()
        )
    })?;

    let PaneCodexSqlite {
        env: codex_env,
        lease,
    } = CodexSqlitePool::new()?.for_pane(&spec.env)?;
    let mut extra_env = spec
        .extra_env
        .iter()
        .map(|(name, value)| (PlatformString::from(name), PlatformString::from(value)))
        .collect::<Vec<_>>();
    extra_env.extend(codex_env);
    let (event_tx, mut event_rx) = mpsc::channel(256);
    let mut pane = LocalPtyPane::spawn(
        &spec.profile_name,
        PaneId(spec.pane_id),
        spec.generation,
        &spec.command,
        &spec.args,
        &spec.env,
        &spec.cwd,
        &extra_env,
        lease,
        spec.scrollback_rows,
        spec.process_priority,
        spec.workload_policy,
        event_tx,
    )?;

    let (command_tx, command_rx) = std_mpsc::channel::<HostInput>();
    let mut client = None::<HostClient>;
    let mut next_client_id = 1_u64;
    let mut keep_running = spec.keep_running;
    let mut background_output = Vec::new();
    let mut last_exit_poll = Instant::now();

    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if client.is_none() {
                    if let Some(connected) = accept_client(
                        stream,
                        next_client_id,
                        &spec.token,
                        &pane,
                        &mut background_output,
                        command_tx.clone(),
                    )? {
                        keep_running = connected.keep_running;
                        client = Some(connected.client);
                        next_client_id = next_client_id.wrapping_add(1);
                    }
                } else {
                    stream.set_nonblocking(false).ok();
                    let _ = send_json(&mut stream, &HostEvent::Busy);
                    let _ = stream.shutdown(Shutdown::Both);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error).context("failed to accept pane host client"),
        }

        while let Ok(input) = command_rx.try_recv() {
            if client.as_ref().map(|value| value.id) != Some(input.client_id) {
                continue;
            }
            match input.message {
                HostInputMessage::Command(command) => match command {
                    HostCommand::Hello { .. } => {}
                    HostCommand::Write { data, token } => {
                        let decoded = BASE64.decode(data).context("invalid pane input payload")?;
                        let tracked_token = token
                            .map(|token| {
                                token
                                    .parse::<u128>()
                                    .context("invalid tracked pane input token")
                            })
                            .transpose()?;
                        let write_result = if let Some(token) = tracked_token {
                            pane.write_tracked(&decoded, PtyWriteToken(token))
                        } else {
                            pane.write(&decoded)
                        };
                        if let Err(error) = write_result {
                            send_to_client(
                                &mut client,
                                &HostEvent::WriteFailed {
                                    token: tracked_token.map(|token| token.to_string()),
                                    error: error.to_string(),
                                },
                            );
                        }
                    }
                    HostCommand::Resize { rows, cols } => {
                        if let Err(error) = pane.resize(rows, cols) {
                            send_to_client(
                                &mut client,
                                &HostEvent::Error {
                                    error: error.to_string(),
                                },
                            );
                        }
                    }
                    HostCommand::ApplyWorkload { policy, class } => {
                        if let Err(error) = pane.apply_workload(policy, class) {
                            send_to_client(
                                &mut client,
                                &HostEvent::Error {
                                    error: error.to_string(),
                                },
                            );
                        }
                    }
                    HostCommand::SetKeepRunning {
                        keep_running: value,
                    } => keep_running = value,
                    HostCommand::Disconnect {
                        keep_running: value,
                    } => {
                        keep_running = value;
                        disconnect_client(&mut client);
                        if !keep_running {
                            pane.terminate();
                            return Ok(());
                        }
                    }
                    HostCommand::Terminate => {
                        pane.terminate();
                        send_to_client(&mut client, &HostEvent::Exited);
                        return Ok(());
                    }
                },
                HostInputMessage::Closed => {
                    disconnect_client(&mut client);
                    if !keep_running {
                        pane.terminate();
                        return Ok(());
                    }
                }
            }
        }

        while let Ok(event) = event_rx.try_recv() {
            match event {
                PtyEvent::Output { bytes, .. } => {
                    pane.process_output(&bytes);
                    if client.is_some() {
                        send_to_client(
                            &mut client,
                            &HostEvent::Output {
                                data: BASE64.encode(&bytes),
                            },
                        );
                    }
                    if client.is_none() {
                        append_background_output(&mut background_output, &bytes);
                    }
                }
                PtyEvent::Exited { .. } => {
                    pane.exited = true;
                    send_to_client(&mut client, &HostEvent::Exited);
                }
                PtyEvent::WriteFailed { token, error, .. } => send_to_client(
                    &mut client,
                    &HostEvent::WriteFailed {
                        token: token.map(|token| token.0.to_string()),
                        error,
                    },
                ),
                PtyEvent::WriteSucceeded { token, .. } => send_to_client(
                    &mut client,
                    &HostEvent::WriteSucceeded {
                        token: token.0.to_string(),
                    },
                ),
            }
        }

        if last_exit_poll.elapsed() >= HOST_EXIT_POLL_INTERVAL {
            if pane.poll_exit() {
                send_to_client(&mut client, &HostEvent::Exited);
            }
            last_exit_poll = Instant::now();
        }

        if client.is_none() && !keep_running {
            pane.terminate();
            return Ok(());
        }
        thread::sleep(HOST_POLL_INTERVAL);
    }
}

struct HostClient {
    id: u64,
    stream: TcpStream,
}

struct AcceptedClient {
    client: HostClient,
    keep_running: bool,
}

struct HostInput {
    client_id: u64,
    message: HostInputMessage,
}

enum HostInputMessage {
    Command(HostCommand),
    Closed,
}

fn accept_client(
    mut stream: TcpStream,
    client_id: u64,
    expected_token: &str,
    pane: &LocalPtyPane,
    background_output: &mut Vec<u8>,
    command_tx: std_mpsc::Sender<HostInput>,
) -> Result<Option<AcceptedClient>> {
    stream
        .set_nonblocking(false)
        .context("failed to make pane host client blocking")?;
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok();
    let reader_stream = stream
        .try_clone()
        .context("failed to clone pane host client")?;
    let mut reader = BufReader::new(reader_stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("failed to read pane host client handshake")?;
    let Ok(HostCommand::Hello {
        protocol_version,
        token,
        keep_running,
    }) = serde_json::from_str::<HostCommand>(&line)
    else {
        let _ = send_json(
            &mut stream,
            &HostEvent::Error {
                error: "expected pane host hello".into(),
            },
        );
        return Ok(None);
    };
    if protocol_version != PANE_HOST_PROTOCOL_VERSION {
        let _ = send_json(
            &mut stream,
            &HostEvent::Incompatible {
                host_version: PANE_HOST_PROTOCOL_VERSION,
            },
        );
        return Ok(None);
    }
    if token != expected_token {
        let _ = send_json(
            &mut stream,
            &HostEvent::Error {
                error: "pane host authentication failed".into(),
            },
        );
        return Ok(None);
    }

    send_json(
        &mut stream,
        &HostEvent::Ready {
            protocol_version: PANE_HOST_PROTOCOL_VERSION,
            background_output: BASE64.encode(&*background_output),
            cwd: pane.cwd().to_path_buf(),
            exited: pane.exited,
        },
    )?;
    background_output.clear();
    reader.get_mut().set_read_timeout(None).ok();
    spawn_host_reader(reader, client_id, command_tx);
    Ok(Some(AcceptedClient {
        client: HostClient {
            id: client_id,
            stream,
        },
        keep_running,
    }))
}

fn spawn_host_reader(
    mut reader: BufReader<TcpStream>,
    client_id: u64,
    sender: std_mpsc::Sender<HostInput>,
) {
    thread::spawn(move || {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => match serde_json::from_str::<HostCommand>(&line) {
                    Ok(command) => {
                        if sender
                            .send(HostInput {
                                client_id,
                                message: HostInputMessage::Command(command),
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(HostInput {
                            client_id,
                            message: HostInputMessage::Command(HostCommand::Disconnect {
                                keep_running: false,
                            }),
                        });
                        let _ = error;
                        return;
                    }
                },
                Err(_) => break,
            }
        }
        let _ = sender.send(HostInput {
            client_id,
            message: HostInputMessage::Closed,
        });
    });
}

fn spawn_client_reader(
    mut reader: BufReader<TcpStream>,
    pane: PaneId,
    generation: u64,
    event_tx: mpsc::Sender<PtyEvent>,
) {
    thread::spawn(move || {
        loop {
            let mut line = String::new();
            let event = match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => serde_json::from_str::<HostEvent>(&line),
                Err(_) => break,
            };
            let Ok(event) = event else {
                continue;
            };
            let event = match event {
                HostEvent::Ready { .. } => continue,
                HostEvent::Output { data } => match BASE64.decode(data) {
                    Ok(bytes) => PtyEvent::Output {
                        pane,
                        generation,
                        bytes,
                    },
                    Err(error) => PtyEvent::WriteFailed {
                        pane,
                        generation,
                        token: None,
                        error: format!("invalid pane host output: {error}"),
                    },
                },
                HostEvent::Exited => PtyEvent::Exited { pane, generation },
                HostEvent::WriteFailed { token, error } => PtyEvent::WriteFailed {
                    pane,
                    generation,
                    token: token
                        .and_then(|token| token.parse().ok())
                        .map(PtyWriteToken),
                    error,
                },
                HostEvent::WriteSucceeded { token } => {
                    let Ok(token) = token.parse() else {
                        continue;
                    };
                    PtyEvent::WriteSucceeded {
                        pane,
                        generation,
                        token: PtyWriteToken(token),
                    }
                }
                HostEvent::Error { error } => PtyEvent::WriteFailed {
                    pane,
                    generation,
                    token: None,
                    error,
                },
                HostEvent::Incompatible { .. } => continue,
                HostEvent::Busy => continue,
            };
            if event_tx.blocking_send(event).is_err() {
                return;
            }
        }
        let _ = event_tx.blocking_send(PtyEvent::Exited { pane, generation });
    });
}

fn send_to_client(client: &mut Option<HostClient>, event: &HostEvent) {
    let failed = client
        .as_mut()
        .is_some_and(|client| send_json(&mut client.stream, event).is_err());
    if failed {
        disconnect_client(client);
    }
}

fn disconnect_client(client: &mut Option<HostClient>) {
    if let Some(client) = client.take() {
        let _ = client.stream.shutdown(Shutdown::Both);
    }
}

fn append_background_output(buffer: &mut Vec<u8>, bytes: &[u8]) {
    buffer.extend_from_slice(bytes);
    if buffer.len() > BACKGROUND_OUTPUT_LIMIT {
        let excess = buffer.len() - BACKGROUND_OUTPUT_LIMIT;
        buffer.drain(..excess);
    }
}

fn send_json(stream: &mut TcpStream, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *stream, value).context("failed to encode pane host message")?;
    stream
        .write_all(b"\n")
        .context("failed to write pane host message")?;
    stream.flush().context("failed to flush pane host message")
}

fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow!("failed to generate pane host token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn pane_hosts_dir() -> Result<PathBuf> {
    ProjectDirs::from("", "", "GridBash")
        .map(|dirs| dirs.data_local_dir().join("pane-hosts"))
        .ok_or_else(|| anyhow!("failed to resolve GridBash pane host directory"))
}

fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn spawn_detached_host(spec_path: &Path) -> Result<std::process::Child> {
    let executable = std::env::current_exe().context("failed to resolve GridBash executable")?;
    let mut command = Command::new(executable);
    command
        .arg("--pane-host")
        .arg(spec_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and runs before the child execs.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    command.spawn().context("failed to start pane host")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHostFiles {
        directory: PathBuf,
        spec: PathBuf,
        ready: PathBuf,
    }

    impl TestHostFiles {
        fn new() -> Self {
            let token = random_token().expect("test token");
            let directory = std::env::temp_dir().join(format!("gridbash-pane-host-test-{token}"));
            fs::create_dir_all(&directory).expect("create test host directory");
            Self {
                spec: directory.join("host.json"),
                ready: directory.join("host.ready.json"),
                directory,
            }
        }
    }

    impl Drop for TestHostFiles {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn host_protocol_round_trips_binary_output() {
        let event = HostEvent::Output {
            data: BASE64.encode(b"hello\0\x1b[31m"),
        };
        let raw = serde_json::to_string(&event).expect("serialize event");
        let HostEvent::Output { data } =
            serde_json::from_str::<HostEvent>(&raw).expect("parse event")
        else {
            panic!("wrong event kind");
        };
        assert_eq!(
            BASE64.decode(data).expect("decode output"),
            b"hello\0\x1b[31m"
        );
    }

    #[test]
    fn background_output_keeps_a_bounded_tail() {
        let mut output = vec![b'a'; BACKGROUND_OUTPUT_LIMIT];
        append_background_output(&mut output, b"latest");
        assert_eq!(output.len(), BACKGROUND_OUTPUT_LIMIT);
        assert!(output.ends_with(b"latest"));
    }

    #[test]
    fn pane_host_survives_disconnect_and_accepts_reattach() {
        let files = TestHostFiles::new();
        let host_token = random_token().expect("host token");
        #[cfg(windows)]
        let (profile_name, command, args, line_ending) = (
            "cmd",
            PathBuf::from("cmd.exe"),
            vec!["/d".to_string(), "/q".to_string()],
            "\r",
        );
        #[cfg(unix)]
        let (profile_name, command, args, line_ending) =
            ("sh", PathBuf::from("/bin/sh"), vec!["-i".to_string()], "\n");
        let spec = HostLaunchSpec {
            token: host_token.clone(),
            ready_path: files.ready.clone(),
            profile_name: profile_name.into(),
            pane_id: 7,
            generation: 0,
            command,
            args,
            env: BTreeMap::new(),
            cwd: std::env::current_dir().expect("current directory"),
            extra_env: Vec::new(),
            scrollback_rows: 1_000,
            process_priority: PaneProcessPriority::Normal,
            workload_policy: PaneWorkloadPolicy::Unrestricted,
            keep_running: true,
        };
        fs::write(
            &files.spec,
            serde_json::to_vec(&spec).expect("serialize host spec"),
        )
        .expect("write host spec");
        let spec_path = files.spec.clone();
        let host_thread = thread::spawn(move || run_pane_host(&spec_path));

        let ready = wait_for_ready(&files.ready);
        let host = PtyHostRef {
            endpoint: ready.endpoint,
            token: host_token,
        };
        let (first_tx, mut first_rx) = mpsc::channel(32);
        let first = PtyPane::attach(
            host.clone(),
            PaneId(7),
            0,
            &spec.cwd,
            1_000,
            true,
            "",
            &[],
            first_tx,
        )
        .expect("attach first client");
        first
            .write(format!("echo before-detach{line_ending}").as_bytes())
            .expect("write before detach");
        assert_output_contains(&mut first_rx, "before-detach");

        let (busy_tx, _) = mpsc::channel(1);
        let busy = match PtyPane::attach(
            host.clone(),
            PaneId(7),
            0,
            &spec.cwd,
            1_000,
            true,
            "",
            &[],
            busy_tx,
        ) {
            Ok(_) => panic!("second simultaneous client should be rejected"),
            Err(error) => error,
        };
        assert!(busy.is::<PaneHostBusy>());
        drop(first);

        thread::sleep(Duration::from_millis(100));
        let (second_tx, mut second_rx) = mpsc::channel(32);
        let mut second = PtyPane::attach(
            host,
            PaneId(7),
            0,
            &spec.cwd,
            1_000,
            true,
            "",
            &[],
            second_tx,
        )
        .expect("reattach second client");
        second
            .write(format!("echo after-reattach{line_ending}").as_bytes())
            .expect("write after reattach");
        assert_output_contains(&mut second_rx, "after-reattach");
        second
            .set_keep_running(false)
            .expect("disable host persistence");
        drop(second);

        host_thread
            .join()
            .expect("join pane host")
            .expect("pane host exits cleanly");
    }

    fn wait_for_ready(path: &Path) -> HostReadyFile {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(raw) = fs::read(path)
                && let Ok(ready) = serde_json::from_slice(&raw)
            {
                return ready;
            }
            assert!(Instant::now() < deadline, "pane host did not become ready");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_output_contains(receiver: &mut mpsc::Receiver<PtyEvent>, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = String::new();
        while Instant::now() < deadline {
            match receiver.try_recv() {
                Ok(PtyEvent::Output { bytes, .. }) => {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                    if output.contains(expected) {
                        return;
                    }
                }
                Ok(PtyEvent::WriteFailed { error, .. }) => panic!("pane write failed: {error}"),
                Ok(_) | Err(mpsc::error::TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        panic!("expected output {expected:?}, got {output:?}");
    }
}
