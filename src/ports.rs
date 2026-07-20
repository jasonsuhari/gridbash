use std::{
    collections::{HashMap, HashSet},
    io,
    process::Command,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProcessRoot {
    pub pid: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPort {
    pub port: u16,
    pub pid: u32,
    pub process: String,
    pub owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessInfo {
    parent_pid: u32,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListeningSocket {
    port: u16,
    pid: u32,
    process: Option<String>,
}

pub fn discover_agent_ports(roots: &[AgentProcessRoot]) -> io::Result<Vec<AgentPort>> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    let processes = process_snapshot()?;
    let listeners = listening_sockets()?;
    Ok(associate_agent_ports(roots, &processes, listeners))
}

pub fn terminate_process(pid: u32) -> io::Result<()> {
    if pid == 0 || pid == std::process::id() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to terminate an invalid or current process",
        ));
    }
    terminate_process_platform(pid)
}

fn associate_agent_ports(
    roots: &[AgentProcessRoot],
    processes: &HashMap<u32, ProcessInfo>,
    listeners: Vec<ListeningSocket>,
) -> Vec<AgentPort> {
    let roots = roots
        .iter()
        .filter(|root| {
            processes
                .get(&root.pid)
                .is_some_and(|process| is_gridbash_host(&process.name))
        })
        .map(|root| (root.pid, root.label.as_str()))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut ports = listeners
        .into_iter()
        .filter_map(|listener| {
            let owner = process_root(listener.pid, processes, &roots)?;
            seen.insert((listener.port, listener.pid))
                .then(|| AgentPort {
                    port: listener.port,
                    pid: listener.pid,
                    process: listener
                        .process
                        .filter(|name| !name.is_empty())
                        .or_else(|| {
                            processes
                                .get(&listener.pid)
                                .map(|process| process.name.clone())
                        })
                        .unwrap_or_else(|| "unknown".into()),
                    owner: owner.to_string(),
                })
        })
        .collect::<Vec<_>>();
    ports.sort_by(|left, right| {
        left.port
            .cmp(&right.port)
            .then_with(|| left.pid.cmp(&right.pid))
    });
    ports
}

fn is_gridbash_host(process_name: &str) -> bool {
    process_name
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("gridbash"))
}

fn process_root<'a>(
    pid: u32,
    processes: &HashMap<u32, ProcessInfo>,
    roots: &HashMap<u32, &'a str>,
) -> Option<&'a str> {
    let mut current = pid;
    let mut visited = HashSet::new();
    for _ in 0..64 {
        if !visited.insert(current) {
            return None;
        }
        let parent = processes.get(&current)?.parent_pid;
        if let Some(label) = roots.get(&parent) {
            return Some(*label);
        }
        if parent == 0 || parent == current {
            return None;
        }
        current = parent;
    }
    None
}

fn endpoint_port(endpoint: &str) -> Option<u16> {
    endpoint
        .trim()
        .rsplit_once(':')
        .and_then(|(_, port)| port.trim_end_matches(']').parse().ok())
}

#[cfg(windows)]
fn listening_sockets() -> io::Result<Vec<ListeningSocket>> {
    let output = Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("netstat failed while listing TCP ports"));
    }
    Ok(parse_windows_netstat(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

#[cfg(windows)]
fn parse_windows_netstat(output: &str) -> Vec<ListeningSocket> {
    output
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 5
                || !fields[0].eq_ignore_ascii_case("TCP")
                || !fields[3].eq_ignore_ascii_case("LISTENING")
            {
                return None;
            }
            Some(ListeningSocket {
                port: endpoint_port(fields[1])?,
                pid: fields[4].parse().ok()?,
                process: None,
            })
        })
        .collect()
}

#[cfg(windows)]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    use std::mem;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut entry = unsafe { mem::zeroed::<PROCESSENTRY32W>() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut processes = HashMap::new();
    let mut success = unsafe { Process32FirstW(snapshot, &mut entry) };
    while success != 0 {
        let length = entry
            .szExeFile
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
        processes.insert(
            entry.th32ProcessID,
            ProcessInfo {
                parent_pid: entry.th32ParentProcessID,
                name,
            },
        );
        success = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    unsafe { CloseHandle(snapshot) };
    Ok(processes)
}

#[cfg(windows)]
fn terminate_process_platform(pid: u32) -> io::Result<()> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
    };
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { TerminateProcess(process, 1) };
    let error = (result == 0).then(io::Error::last_os_error);
    unsafe { CloseHandle(process) };
    error.map_or(Ok(()), Err)
}

#[cfg(unix)]
fn listening_sockets() -> io::Result<Vec<ListeningSocket>> {
    let lsof = if cfg!(target_os = "macos") {
        "/usr/sbin/lsof"
    } else {
        "lsof"
    };
    match Command::new(lsof)
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"])
        .output()
    {
        Ok(output) if output.status.success() || !output.stdout.is_empty() => {
            return Ok(parse_lsof(&String::from_utf8_lossy(&output.stdout)));
        }
        _ => {}
    }
    #[cfg(target_os = "linux")]
    {
        let output = Command::new("ss").args(["-H", "-ltnp"]).output()?;
        if output.status.success() {
            return Ok(parse_linux_ss(&String::from_utf8_lossy(&output.stdout)));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "neither lsof nor a supported socket inspector is available",
    ))
}

#[cfg(unix)]
fn parse_lsof(output: &str) -> Vec<ListeningSocket> {
    let mut pid = None;
    let mut process = None::<String>;
    let mut listeners = Vec::new();
    for line in output.lines() {
        let Some(tag) = line.get(..1) else {
            continue;
        };
        let value = &line[1..];
        match tag {
            "p" => {
                pid = value.parse().ok();
                process = None;
            }
            "c" => process = Some(value.to_string()),
            "n" => {
                if let (Some(pid), Some(port)) = (pid, endpoint_port(value)) {
                    listeners.push(ListeningSocket {
                        port,
                        pid,
                        process: process.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    listeners
}

#[cfg(target_os = "linux")]
fn parse_linux_ss(output: &str) -> Vec<ListeningSocket> {
    output
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let local = *fields.get(3)?;
            let details = fields.get(5..)?.join(" ");
            let pid_start = details.find("pid=")? + 4;
            let pid_end = details[pid_start..]
                .find(|ch: char| !ch.is_ascii_digit())
                .map(|offset| pid_start + offset)
                .unwrap_or(details.len());
            let pid = details[pid_start..pid_end].parse().ok()?;
            let process = details
                .split_once("((\"")
                .and_then(|(_, tail)| tail.split_once('"'))
                .map(|(name, _)| name.to_string());
            Some(ListeningSocket {
                port: endpoint_port(local)?,
                pid,
                process,
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    let mut processes = HashMap::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(name_start) = stat.find('(') else {
            continue;
        };
        let Some(name_end) = stat.rfind(')') else {
            continue;
        };
        let Some(parent_pid) = stat[name_end + 1..]
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        processes.insert(
            pid,
            ProcessInfo {
                parent_pid,
                name: stat[name_start + 1..name_end].to_string(),
            },
        );
    }
    Ok(processes)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,comm="])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("ps failed while listing processes"));
    }
    Ok(parse_ps(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn parse_ps(output: &str) -> HashMap<u32, ProcessInfo> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent_pid = fields.next()?.parse().ok()?;
            let name = fields.next()?.rsplit('/').next()?.to_string();
            Some((pid, ProcessInfo { parent_pid, name }))
        })
        .collect()
}

#[cfg(unix)]
fn terminate_process_platform(pid: u32) -> io::Result<()> {
    let pid = i32::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "process ID is too large"))?;
    let result = unsafe { libc::kill(pid, libc::SIGTERM) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(windows, unix)))]
fn listening_sockets() -> io::Result<Vec<ListeningSocket>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "port discovery is not supported on this platform",
    ))
}

#[cfg(not(any(windows, unix)))]
fn process_snapshot() -> io::Result<HashMap<u32, ProcessInfo>> {
    Ok(HashMap::new())
}

#[cfg(not(any(windows, unix)))]
fn terminate_process_platform(_pid: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process termination is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn associates_only_descendants_of_agent_roots() {
        let roots = vec![AgentProcessRoot {
            pid: 10,
            label: "Grid 1 / Pane 2".into(),
        }];
        let processes = HashMap::from([
            (
                10,
                ProcessInfo {
                    parent_pid: 1,
                    name: "gridbash".into(),
                },
            ),
            (
                20,
                ProcessInfo {
                    parent_pid: 10,
                    name: "codex".into(),
                },
            ),
            (
                30,
                ProcessInfo {
                    parent_pid: 20,
                    name: "node".into(),
                },
            ),
            (
                40,
                ProcessInfo {
                    parent_pid: 1,
                    name: "postgres".into(),
                },
            ),
        ]);
        let listeners = vec![
            ListeningSocket {
                port: 41_000,
                pid: 10,
                process: None,
            },
            ListeningSocket {
                port: 3000,
                pid: 30,
                process: None,
            },
            ListeningSocket {
                port: 5432,
                pid: 40,
                process: None,
            },
        ];
        assert_eq!(
            associate_agent_ports(&roots, &processes, listeners),
            vec![AgentPort {
                port: 3000,
                pid: 30,
                process: "node".into(),
                owner: "Grid 1 / Pane 2".into(),
            }]
        );
    }

    #[cfg(windows)]
    #[test]
    fn parses_windows_tcp_listeners() {
        let output = r#"
  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       1234
  TCP    [::1]:8080             [::]:0                 LISTENING       4321
  TCP    127.0.0.1:5000         127.0.0.1:6000         ESTABLISHED     9999
"#;
        assert_eq!(
            parse_windows_netstat(output),
            vec![
                ListeningSocket {
                    port: 3000,
                    pid: 1234,
                    process: None,
                },
                ListeningSocket {
                    port: 8080,
                    pid: 4321,
                    process: None,
                },
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn parses_lsof_machine_output() {
        let output = "p1234\ncnode\nf19\nPIPv4\nn*:3000\nn127.0.0.1:3001\n";
        assert_eq!(
            parse_lsof(output),
            vec![
                ListeningSocket {
                    port: 3000,
                    pid: 1234,
                    process: Some("node".into()),
                },
                ListeningSocket {
                    port: 3001,
                    pid: 1234,
                    process: Some("node".into()),
                },
            ]
        );
    }
}
