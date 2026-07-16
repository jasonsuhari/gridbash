use std::{
    collections::BTreeSet,
    env,
    io::{self, BufRead, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::mpsc::{self, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    cli::{CtlAction, CtlArgs},
    control_discovery::{self, DiscoveryLease, DiscoveryRecord},
    layout::PaneId,
};

const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const CONTROL_READ_TIMEOUT: Duration = Duration::from_secs(8);
const CONTROL_REQUEST_LIMIT_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
pub struct ControlHandle {
    id: String,
    endpoint: String,
    token: String,
    _discovery: DiscoveryLease,
}

impl ControlHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PaneTarget {
    Number(usize),
    Stable { pane_id: usize, generation: u64 },
}

impl PaneTarget {
    pub fn parse(value: &str) -> Result<Self> {
        if let Ok(number) = value.parse::<usize>() {
            if number == 0 {
                bail!("pane numbers are 1-based");
            }
            return Ok(Self::Number(number));
        }

        let Some(value) = value.strip_prefix("pane-") else {
            bail!("invalid pane target '{value}'; use a pane number or pane-<id>-gen-<generation>");
        };
        let Some((pane_id, generation)) = value.split_once("-gen-") else {
            bail!("invalid stable pane target; expected pane-<id>-gen-<generation>");
        };
        Ok(Self::Stable {
            pane_id: pane_id
                .parse()
                .with_context(|| format!("invalid pane id '{pane_id}'"))?,
            generation: generation
                .parse()
                .with_context(|| format!("invalid pane generation '{generation}'"))?,
        })
    }

    pub fn stable_label(pane: PaneId, generation: u64) -> String {
        format!("pane-{}-gen-{generation}", pane.0)
    }
}

impl From<usize> for PaneTarget {
    fn from(number: usize) -> Self {
        Self::Number(number)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneIdentity {
    pub index: usize,
    pub pane: PaneId,
    pub generation: u64,
}

pub fn resolve_pane_targets(
    targets: &[PaneTarget],
    identities: &[PaneIdentity],
) -> Result<Vec<usize>> {
    if targets.is_empty() {
        bail!("at least one target pane is required");
    }

    let mut resolved = BTreeSet::new();
    for target in targets {
        let index = match target {
            PaneTarget::Number(number) => identities
                .iter()
                .find(|identity| identity.index + 1 == *number)
                .map(|identity| identity.index)
                .ok_or_else(|| anyhow!("pane {number} is not available"))?,
            PaneTarget::Stable {
                pane_id,
                generation,
            } => {
                let identity = identities
                    .iter()
                    .find(|identity| identity.pane.0 == *pane_id)
                    .ok_or_else(|| anyhow!("pane-{pane_id} is not available in this session"))?;
                if identity.generation != *generation {
                    bail!(
                        "stale pane identity pane-{pane_id}-gen-{generation}; current generation is {}",
                        identity.generation
                    );
                }
                identity.index
            }
        };
        resolved.insert(index);
    }
    Ok(resolved.into_iter().collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlCommand {
    Ping,
    Describe,
    SetStatus {
        message: String,
    },
    SendCommand {
        panes: Vec<PaneTarget>,
        command: String,
        submit: bool,
    },
    ShowImage {
        path: PathBuf,
        title: Option<String>,
    },
    CaptureOutput {
        panes: Vec<PaneTarget>,
        directory: Option<PathBuf>,
    },
    StartLogging {
        panes: Vec<PaneTarget>,
        directory: Option<PathBuf>,
    },
    StopLogging {
        panes: Vec<PaneTarget>,
    },
    Focus {
        pane: PaneTarget,
    },
}

impl ControlCommand {
    fn requires_token(&self) -> bool {
        !matches!(self, Self::Ping | Self::Describe)
    }
}

#[derive(Debug)]
pub struct ControlEnvelope {
    pub command: ControlCommand,
    pub response_tx: Sender<ControlResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlResponse {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ControlResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(message: impl Into<String>, data: Value) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ControlWireRequest {
    #[serde(default)]
    token: Option<String>,
    command: ControlCommand,
}

pub fn start_control_server(
    port: u16,
    command_tx: Sender<ControlEnvelope>,
) -> Result<ControlHandle> {
    let listener = TcpListener::bind(("127.0.0.1", port)).context("failed to bind agent API")?;
    let endpoint = listener
        .local_addr()
        .context("failed to read agent API address")?
        .to_string();
    let token = new_token()?;
    let id = new_instance_id()?;
    let discovery = DiscoveryLease::publish(&DiscoveryRecord::new(id.clone(), endpoint.clone()))?;
    let server_token = token.clone();
    let server_id = id.clone();

    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => handle_control_stream(stream, &server_id, &server_token, &command_tx),
                Err(error) => eprintln!("gridbash agent API accept failed: {error}"),
            }
        }
    });

    Ok(ControlHandle {
        id,
        endpoint,
        token,
        _discovery: discovery,
    })
}

fn handle_control_stream(
    mut stream: TcpStream,
    id: &str,
    token: &str,
    command_tx: &Sender<ControlEnvelope>,
) {
    let _ = stream.set_read_timeout(Some(CONTROL_READ_TIMEOUT));
    let response = read_control_request(&mut stream).and_then(|request| {
        if !request_authorized(&request, token) {
            return Ok(ControlResponse::error("invalid GridBash control token"));
        }

        if matches!(&request.command, ControlCommand::Ping) {
            return Ok(ControlResponse::with_data(
                "GridBash control session is live",
                json!({ "id": id }),
            ));
        }

        let (response_tx, response_rx) = mpsc::channel();
        command_tx
            .send(ControlEnvelope {
                command: request.command,
                response_tx,
            })
            .context("GridBash app is not accepting control commands")?;
        response_rx
            .recv_timeout(CONTROL_READ_TIMEOUT)
            .context("GridBash app did not answer the control command")
    });

    let response = response.unwrap_or_else(|error| ControlResponse::error(format!("{error:#}")));
    let _ = serde_json::to_writer(&mut stream, &response);
    let _ = stream.flush();
}

fn request_authorized(request: &ControlWireRequest, expected_token: &str) -> bool {
    !request.command.requires_token() || request.token.as_deref() == Some(expected_token)
}

fn read_control_request(stream: &mut TcpStream) -> Result<ControlWireRequest> {
    let mut body = String::new();
    stream
        .take(CONTROL_REQUEST_LIMIT_BYTES)
        .read_to_string(&mut body)
        .context("failed to read control request")?;
    serde_json::from_str(&body).context("invalid control request JSON")
}

fn new_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow!("failed to create agent API token: {error}"))?;
    Ok(hex_encode(&bytes))
}

fn new_instance_id() -> Result<String> {
    let mut random = [0_u8; 6];
    getrandom::fill(&mut random)
        .map_err(|error| anyhow!("failed to create control session id: {error}"))?;
    Ok(format!(
        "{}-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        std::process::id(),
        hex_encode(&random)
    ))
}

fn hex_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn run_mcp_server() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line.context("failed to read MCP input")?;
        if line.trim().is_empty() {
            continue;
        }

        let response = handle_mcp_line(&line);
        if let Some(response) = response {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)
                .context("failed to write MCP response")?;
            stdout.flush().context("failed to flush MCP response")?;
        }
    }

    Ok(())
}

pub fn run_ctl(args: &CtlArgs) -> Result<()> {
    let sessions = control_discovery::discover_sessions(probe_discovered_session)?;
    if matches!(&args.action, CtlAction::List) {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&sessions).context("failed to serialize sessions")?
            );
        } else if sessions.is_empty() {
            println!("gridbash: no opted-in running sessions found");
        } else {
            println!("ID\tPID\tENDPOINT\tSTARTED");
            for session in &sessions {
                println!(
                    "{}\t{}\t{}\t{}",
                    session.id, session.pid, session.endpoint, session.started_at
                );
            }
        }
        return Ok(());
    }

    let requested_session = args
        .session
        .clone()
        .or_else(|| env::var("GRIDBASH_CONTROL_SESSION").ok());
    let session = select_discovered_session(&sessions, requested_session.as_deref())?;
    let (command, token) = match &args.action {
        CtlAction::List => unreachable!("list returned above"),
        CtlAction::Panes => (ControlCommand::Describe, None),
        CtlAction::Send {
            panes,
            command,
            no_submit,
        } => (
            ControlCommand::SendCommand {
                panes: parse_pane_targets(panes)?,
                command: command.clone(),
                submit: !no_submit,
            },
            Some(ctl_token(args)?),
        ),
        CtlAction::Capture { panes, directory } => (
            ControlCommand::CaptureOutput {
                panes: parse_pane_targets(panes)?,
                directory: directory.clone().map(absolute_tool_path).transpose()?,
            },
            Some(ctl_token(args)?),
        ),
        CtlAction::Status { message } => (
            ControlCommand::SetStatus {
                message: message.clone(),
            },
            Some(ctl_token(args)?),
        ),
        CtlAction::Focus { pane } => (
            ControlCommand::Focus {
                pane: PaneTarget::parse(pane)?,
            },
            Some(ctl_token(args)?),
        ),
    };

    let response = call_control(&session.endpoint, token.as_deref(), command)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response)
                .context("failed to serialize control response")?
        );
    } else if matches!(&args.action, CtlAction::Panes) && response.ok {
        print_panes(response.data.as_ref());
    } else {
        println!("{}", response.message);
    }
    if !response.ok {
        bail!(response.message);
    }
    Ok(())
}

fn probe_discovered_session(session: &DiscoveryRecord) -> bool {
    call_control_with_timeout(
        &session.endpoint,
        None,
        ControlCommand::Ping,
        Duration::from_millis(500),
    )
    .ok()
    .filter(|response| response.ok)
    .and_then(|response| response.data)
    .and_then(|data| data.get("id").and_then(Value::as_str).map(str::to_owned))
    .is_some_and(|id| id == session.id)
}

fn select_discovered_session<'a>(
    sessions: &'a [DiscoveryRecord],
    query: Option<&str>,
) -> Result<&'a DiscoveryRecord> {
    if let Some(query) = query {
        if let Some(exact) = sessions.iter().find(|session| session.id == query) {
            return Ok(exact);
        }
        let matches = sessions
            .iter()
            .filter(|session| session.id.starts_with(query))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [] => Err(anyhow!("no running session matches '{query}'")),
            [session] => Ok(*session),
            _ => Err(anyhow!("running session prefix '{query}' is ambiguous")),
        };
    }

    match sessions {
        [] => Err(anyhow!("no opted-in running GridBash sessions found")),
        [session] => Ok(session),
        _ => Err(anyhow!(
            "multiple GridBash sessions are running; pass --session <id-or-prefix>"
        )),
    }
}

fn ctl_token(args: &CtlArgs) -> Result<String> {
    args.token
        .clone()
        .or_else(|| env::var("GRIDBASH_CONTROL_TOKEN").ok())
        .ok_or_else(|| {
            anyhow!(
                "this operation requires --token or GRIDBASH_CONTROL_TOKEN from the target session"
            )
        })
}

fn parse_pane_targets(values: &[String]) -> Result<Vec<PaneTarget>> {
    values
        .iter()
        .map(|value| PaneTarget::parse(value))
        .collect()
}

fn print_panes(data: Option<&Value>) {
    println!("NUMBER\tID\tSTATE\tLABEL\tCWD");
    let panes = data
        .and_then(|data| data.get("panes"))
        .and_then(Value::as_array);
    for pane in panes.into_iter().flatten() {
        let mut states = Vec::new();
        for (field, label) in [
            ("focused", "focused"),
            ("selected", "selected"),
            ("sleeping", "sleeping"),
            ("logging", "logging"),
            ("exited", "exited"),
        ] {
            if pane.get(field).and_then(Value::as_bool) == Some(true) {
                states.push(label);
            }
        }
        let state = if states.is_empty() {
            "running".to_string()
        } else {
            states.join(",")
        };
        println!(
            "{}\t{}\t{}\t{}\t{}",
            pane.get("number").and_then(Value::as_u64).unwrap_or(0),
            pane.get("id").and_then(Value::as_str).unwrap_or("unknown"),
            state,
            pane.get("label").and_then(Value::as_str).unwrap_or(""),
            pane.get("cwd").and_then(Value::as_str).unwrap_or("")
        );
    }
}

fn handle_mcp_line(line: &str) -> Option<Value> {
    let value = match serde_json::from_str::<Value>(line) {
        Ok(value) => value,
        Err(error) => {
            return Some(rpc_error(
                Value::Null,
                -32700,
                format!("Parse error: {error}"),
            ));
        }
    };

    let id = value.get("id").cloned();
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return id.map(|id| rpc_error(id, -32600, "Invalid request"));
    };

    match method {
        "notifications/initialized" => None,
        "initialize" => id.map(|id| rpc_result(id, initialize_result())),
        "ping" => id.map(|id| rpc_result(id, json!({}))),
        "tools/list" => id.map(|id| rpc_result(id, tools_list_result())),
        "tools/call" => id.map(|id| {
            let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
            match handle_tool_call(params) {
                Ok(result) => rpc_result(id, result),
                Err(error) => rpc_error(id, -32602, format!("{error:#}")),
            }
        }),
        _ => id.map(|id| rpc_error(id, -32601, format!("Method not found: {method}"))),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "gridbash",
            "title": "GridBash Agent Control",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Use these tools only against the current GridBash session. Mutating tools send input into live panes."
    })
}

fn tools_list_result() -> Value {
    json!({
        "tools": [
            {
                "name": "gridbash_show_image",
                "title": "Show Image",
                "description": "Display a local image path as an overlay in the running GridBash session.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Local filesystem path to a png, jpg, gif, or webp image."
                        },
                        "title": {
                            "type": "string",
                            "description": "Optional overlay title."
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "gridbash_send_command",
                "title": "Send Command",
                "description": "Send command text to one or more 1-based GridBash panes.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "panes": {
                            "type": "array",
                            "description": "1-based pane numbers to receive the command.",
                            "items": {
                                "type": "integer",
                                "minimum": 1
                            },
                            "minItems": 1
                        },
                        "command": {
                            "type": "string",
                            "description": "Text to write into each target pane."
                        },
                        "submit": {
                            "type": "boolean",
                            "description": "When true, append Enter after the command.",
                            "default": true
                        }
                    },
                    "required": ["panes", "command"]
                }
            },
            {
                "name": "gridbash_set_status",
                "title": "Set Status",
                "description": "Set the GridBash status bar text for the current session.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "Short status message to show in the GridBash status bar."
                        }
                    },
                    "required": ["message"]
                }
            },
            {
                "name": "gridbash_capture_output",
                "title": "Capture Pane Output",
                "description": "Save bounded recent plain-text output from one or more GridBash panes.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "panes": {
                            "type": "array",
                            "description": "1-based pane numbers to capture.",
                            "items": { "type": "integer", "minimum": 1 },
                            "minItems": 1
                        },
                        "directory": {
                            "type": "string",
                            "description": "Optional output directory. GridBash local data storage is used by default."
                        }
                    },
                    "required": ["panes"]
                }
            },
            {
                "name": "gridbash_start_logging",
                "title": "Start Pane Logging",
                "description": "Continuously append new plain-text output from one or more GridBash panes to separate files.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "panes": {
                            "type": "array",
                            "description": "1-based pane numbers to log.",
                            "items": { "type": "integer", "minimum": 1 },
                            "minItems": 1
                        },
                        "directory": {
                            "type": "string",
                            "description": "Optional output directory. GridBash local data storage is used by default."
                        }
                    },
                    "required": ["panes"]
                }
            },
            {
                "name": "gridbash_stop_logging",
                "title": "Stop Pane Logging",
                "description": "Stop and flush continuous output logs for one or more GridBash panes.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "panes": {
                            "type": "array",
                            "description": "1-based pane numbers to stop logging.",
                            "items": { "type": "integer", "minimum": 1 },
                            "minItems": 1
                        }
                    },
                    "required": ["panes"]
                }
            }
        ]
    })
}

fn handle_tool_call(params: Value) -> Result<Value> {
    let tool_name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing tool name"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let command = tool_arguments_to_command(tool_name, arguments)?;
    let response = call_gridbash_control(command)?;
    Ok(tool_response(response.ok, response.message, response.data))
}

fn tool_arguments_to_command(tool_name: &str, arguments: Value) -> Result<ControlCommand> {
    match tool_name {
        "gridbash_show_image" => {
            #[derive(Deserialize)]
            struct Args {
                path: PathBuf,
                title: Option<String>,
            }

            let args: Args = serde_json::from_value(arguments).context("invalid image args")?;
            Ok(ControlCommand::ShowImage {
                path: absolute_tool_path(args.path)?,
                title: args.title,
            })
        }
        "gridbash_send_command" => {
            #[derive(Deserialize)]
            struct Args {
                panes: Vec<usize>,
                command: String,
                submit: Option<bool>,
            }

            let args: Args = serde_json::from_value(arguments).context("invalid command args")?;
            if args.panes.is_empty() {
                return Err(anyhow!("at least one pane is required"));
            }
            Ok(ControlCommand::SendCommand {
                panes: args.panes.into_iter().map(Into::into).collect(),
                command: args.command,
                submit: args.submit.unwrap_or(true),
            })
        }
        "gridbash_set_status" => {
            #[derive(Deserialize)]
            struct Args {
                message: String,
            }

            let args: Args = serde_json::from_value(arguments).context("invalid status args")?;
            Ok(ControlCommand::SetStatus {
                message: args.message,
            })
        }
        "gridbash_capture_output" | "gridbash_start_logging" => {
            #[derive(Deserialize)]
            struct Args {
                panes: Vec<usize>,
                directory: Option<PathBuf>,
            }

            let args: Args = serde_json::from_value(arguments).context("invalid output args")?;
            if args.panes.is_empty() {
                return Err(anyhow!("at least one pane is required"));
            }
            let directory = args.directory.map(absolute_tool_path).transpose()?;
            if tool_name == "gridbash_capture_output" {
                Ok(ControlCommand::CaptureOutput {
                    panes: args.panes.into_iter().map(Into::into).collect(),
                    directory,
                })
            } else {
                Ok(ControlCommand::StartLogging {
                    panes: args.panes.into_iter().map(Into::into).collect(),
                    directory,
                })
            }
        }
        "gridbash_stop_logging" => {
            #[derive(Deserialize)]
            struct Args {
                panes: Vec<usize>,
            }

            let args: Args = serde_json::from_value(arguments).context("invalid logging args")?;
            if args.panes.is_empty() {
                return Err(anyhow!("at least one pane is required"));
            }
            Ok(ControlCommand::StopLogging {
                panes: args.panes.into_iter().map(Into::into).collect(),
            })
        }
        _ => Err(anyhow!("unknown GridBash tool: {tool_name}")),
    }
}

fn absolute_tool_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }

    Ok(env::current_dir()
        .context("failed to resolve MCP server current directory")?
        .join(path))
}

fn call_gridbash_control(command: ControlCommand) -> Result<ControlResponse> {
    let endpoint = env::var("GRIDBASH_CONTROL_ADDR")
        .context("GRIDBASH_CONTROL_ADDR is not set; start GridBash with --agent-api")?;
    let token = env::var("GRIDBASH_CONTROL_TOKEN")
        .context("GRIDBASH_CONTROL_TOKEN is not set; start GridBash with --agent-api")?;
    call_control(&endpoint, Some(&token), command)
}

fn call_control(
    endpoint: &str,
    token: Option<&str>,
    command: ControlCommand,
) -> Result<ControlResponse> {
    call_control_with_timeout(endpoint, token, command, CONTROL_READ_TIMEOUT)
}

fn call_control_with_timeout(
    endpoint: &str,
    token: Option<&str>,
    command: ControlCommand,
    timeout: Duration,
) -> Result<ControlResponse> {
    let address = endpoint
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid GridBash control endpoint '{endpoint}'"))?;
    let mut stream = TcpStream::connect_timeout(&address, timeout.min(Duration::from_millis(750)))
        .with_context(|| format!("failed to connect to GridBash control API at {endpoint}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .context("failed to set GridBash control read timeout")?;
    stream
        .set_write_timeout(Some(timeout))
        .context("failed to set GridBash control write timeout")?;

    serde_json::to_writer(&mut stream, &json!({ "token": token, "command": command }))
        .context("failed to send GridBash control request")?;
    stream
        .shutdown(Shutdown::Write)
        .context("failed to finish GridBash control request")?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("failed to read GridBash control response")?;
    serde_json::from_str(&response).context("invalid GridBash control response")
}

fn tool_response(ok: bool, message: String, data: Option<Value>) -> Value {
    let text = if let Some(data) = data {
        format!(
            "{message}\n{}",
            serde_json::to_string_pretty(&data).unwrap_or(data.to_string())
        )
    } else {
        message
    };

    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "isError": !ok
    })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message.into()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_lists_the_gridbash_control_tools() {
        let response =
            handle_mcp_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).expect("response");
        let names = response["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("name"))
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "gridbash_show_image",
                "gridbash_send_command",
                "gridbash_set_status",
                "gridbash_capture_output",
                "gridbash_start_logging",
                "gridbash_stop_logging"
            ]
        );
    }

    #[test]
    fn send_command_defaults_to_submit() {
        let command = tool_arguments_to_command(
            "gridbash_send_command",
            json!({
                "panes": [2],
                "command": "cargo test"
            }),
        )
        .expect("command");

        assert!(matches!(
            command,
            ControlCommand::SendCommand {
                panes,
                command,
                submit: true
            } if panes == vec![PaneTarget::Number(2)] && command == "cargo test"
        ));
    }

    #[test]
    fn output_tools_parse_targets_and_optional_directories() {
        let capture = tool_arguments_to_command(
            "gridbash_capture_output",
            json!({ "panes": [1, 3], "directory": "captures" }),
        )
        .expect("capture command");
        assert!(matches!(
            capture,
            ControlCommand::CaptureOutput { panes, directory: Some(path) }
                if panes == vec![PaneTarget::Number(1), PaneTarget::Number(3)]
                    && path.is_absolute() && path.ends_with("captures")
        ));

        let stop = tool_arguments_to_command("gridbash_stop_logging", json!({ "panes": [2] }))
            .expect("stop command");
        assert!(matches!(
            stop,
            ControlCommand::StopLogging { panes } if panes == vec![PaneTarget::Number(2)]
        ));
    }

    #[test]
    fn read_only_inspection_is_tokenless_but_mutations_require_authentication() {
        let inspect = ControlWireRequest {
            token: None,
            command: ControlCommand::Describe,
        };
        let unauthenticated_write = ControlWireRequest {
            token: None,
            command: ControlCommand::SetStatus {
                message: "working".into(),
            },
        };
        let authenticated_write = ControlWireRequest {
            token: Some("secret".into()),
            command: ControlCommand::SetStatus {
                message: "working".into(),
            },
        };

        assert!(request_authorized(&inspect, "secret"));
        assert!(!request_authorized(&unauthenticated_write, "secret"));
        assert!(request_authorized(&authenticated_write, "secret"));
    }

    #[test]
    fn stable_pane_targets_reject_stale_generations() {
        let identities = [
            PaneIdentity {
                index: 0,
                pane: PaneId(7),
                generation: 3,
            },
            PaneIdentity {
                index: 1,
                pane: PaneId(9),
                generation: 1,
            },
        ];

        assert_eq!(
            resolve_pane_targets(
                &[
                    PaneTarget::Number(2),
                    PaneTarget::Stable {
                        pane_id: 7,
                        generation: 3,
                    },
                ],
                &identities,
            )
            .expect("resolve targets"),
            vec![0, 1]
        );
        let error = resolve_pane_targets(
            &[PaneTarget::Stable {
                pane_id: 7,
                generation: 2,
            }],
            &identities,
        )
        .expect_err("stale generation");
        assert!(error.to_string().contains("stale pane identity"));
    }

    #[test]
    fn session_selection_rejects_ambiguity_without_an_explicit_prefix() {
        let sessions = [
            DiscoveryRecord::new("alpha-one".into(), "127.0.0.1:1".into()),
            DiscoveryRecord::new("alpha-two".into(), "127.0.0.1:2".into()),
        ];

        assert!(select_discovered_session(&sessions, None).is_err());
        assert!(select_discovered_session(&sessions, Some("alpha")).is_err());
        assert_eq!(
            select_discovered_session(&sessions, Some("alpha-one"))
                .expect("exact session")
                .id,
            "alpha-one"
        );
    }

    #[test]
    fn discovery_json_is_machine_readable_and_contains_no_token() {
        let sessions = vec![DiscoveryRecord::new(
            "runtime".into(),
            "127.0.0.1:4321".into(),
        )];
        let raw = serde_json::to_string(&sessions).expect("session json");
        let decoded: Value = serde_json::from_str(&raw).expect("decode json");

        assert_eq!(decoded[0]["id"], "runtime");
        assert_eq!(decoded[0]["endpoint"], "127.0.0.1:4321");
        assert!(decoded[0].get("token").is_none());
    }
}
