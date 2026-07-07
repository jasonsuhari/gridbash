use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::Sender,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

const CLAUDE_USAGE_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/usage";
const OPENAI_COSTS_ENDPOINT: &str = "https://api.openai.com/v1/organization/costs";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageTarget {
    pub profile_name: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UsageEvent {
    Profile {
        profile_name: String,
        label: Option<String>,
    },
    ApiSpend {
        label: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthKind {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageSource {
    profile_name: String,
    dir: PathBuf,
    kind: AuthKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct UsageBlock {
    used_percent: f64,
}

pub fn spawn_usage_monitor(targets: Vec<UsageTarget>, tx: Sender<UsageEvent>) {
    thread::spawn(move || {
        let targets = resolve_usage_sources(&targets);
        loop {
            let mut disconnected = false;

            for source in &targets {
                let label = profile_usage_label(source);
                if tx
                    .send(UsageEvent::Profile {
                        profile_name: source.profile_name.clone(),
                        label,
                    })
                    .is_err()
                {
                    disconnected = true;
                    break;
                }
            }

            if disconnected {
                break;
            }

            if tx
                .send(UsageEvent::ApiSpend {
                    label: api_spend_label(),
                })
                .is_err()
            {
                break;
            }

            thread::sleep(REFRESH_INTERVAL);
        }
    });
}

fn resolve_usage_sources(targets: &[UsageTarget]) -> Vec<UsageSource> {
    let mut sources = BTreeMap::new();
    for target in targets {
        if let Some(source) = resolve_usage_source(target) {
            sources.entry(source.profile_name.clone()).or_insert(source);
        }
    }
    sources.into_values().collect()
}

fn resolve_usage_source(target: &UsageTarget) -> Option<UsageSource> {
    let vibe_dir = profiles_home().join(&target.profile_name);
    if vibe_dir.is_dir() {
        let kind = profile_kind(&vibe_dir)
            .or_else(|| infer_kind(&target.profile_name))
            .unwrap_or(AuthKind::Claude);
        return Some(UsageSource {
            profile_name: target.profile_name.clone(),
            dir: vibe_dir,
            kind,
        });
    }

    let command_kind = infer_kind(&target.command).or_else(|| infer_kind(&target.profile_name))?;
    let dir = match command_kind {
        AuthKind::Claude => home_dir()?.join(".claude"),
        AuthKind::Codex => home_dir()?.join(".codex"),
    };

    dir.is_dir().then(|| UsageSource {
        profile_name: target.profile_name.clone(),
        dir,
        kind: command_kind,
    })
}

fn profile_kind(dir: &Path) -> Option<AuthKind> {
    let raw = fs::read_to_string(dir.join(".profile-kind")).ok()?;
    match raw.trim() {
        "claude" => Some(AuthKind::Claude),
        "codex" => Some(AuthKind::Codex),
        _ => None,
    }
}

fn infer_kind(value: &str) -> Option<AuthKind> {
    let name = Path::new(value)
        .file_stem()
        .and_then(|part| part.to_str())
        .unwrap_or(value)
        .to_ascii_lowercase();

    if name.starts_with("codex") || name.contains("codex") {
        Some(AuthKind::Codex)
    } else if name.starts_with("claude") || name.contains("claude") {
        Some(AuthKind::Claude)
    } else {
        None
    }
}

fn profile_usage_label(source: &UsageSource) -> Option<String> {
    let usage = match source.kind {
        AuthKind::Claude => fetch_claude_usage(&source.dir),
        AuthKind::Codex => {
            fetch_codex_usage(&source.dir).or_else(|| codex_usage_snapshot(&source.dir))
        }
    }?;
    usage_label(usage.five, usage.seven)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct UsageWindows {
    five: Option<UsageBlock>,
    seven: Option<UsageBlock>,
}

fn usage_label(five: Option<UsageBlock>, seven: Option<UsageBlock>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(block) = five {
        parts.push(format!(
            "5h {}% left",
            available_percent(block.used_percent)
        ));
    }
    if let Some(block) = seven {
        parts.push(format!(
            "7d {}% left",
            available_percent(block.used_percent)
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" / "))
}

fn available_percent(used_percent: f64) -> u8 {
    (100.0 - used_percent).round().clamp(0.0, 100.0) as u8
}

fn fetch_claude_usage(dir: &Path) -> Option<UsageWindows> {
    let token = read_claude_access_token(dir)?;
    let output = run_curl(&[
        "-s",
        "-m",
        "3",
        "-H",
        &format!("Authorization: Bearer {token}"),
        "-H",
        "anthropic-beta: oauth-2025-04-20",
        CLAUDE_USAGE_ENDPOINT,
    ])?;

    parse_claude_usage(&output)
}

fn fetch_codex_usage(dir: &Path) -> Option<UsageWindows> {
    let auth = read_codex_auth(dir)?;
    let mut args = vec![
        "-s".to_string(),
        "-m".to_string(),
        "3".to_string(),
        "-H".to_string(),
        format!("Authorization: Bearer {}", auth.access_token),
        "-H".to_string(),
        "User-Agent: codex-cli".to_string(),
    ];
    if let Some(account_id) = auth.account_id {
        args.push("-H".to_string());
        args.push(format!("chatgpt-account-id: {account_id}"));
    }
    args.push(CODEX_USAGE_ENDPOINT.to_string());

    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run_curl(&refs)?;
    parse_codex_usage(&output)
}

fn api_spend_label() -> Option<String> {
    let token = env::var("OPENAI_ADMIN_KEY")
        .ok()
        .filter(|value| !value.is_empty())?;
    let now = unix_now()?;
    let start = now.saturating_sub(24 * 60 * 60);
    let url = format!("{OPENAI_COSTS_ENDPOINT}?start_time={start}&end_time={now}&limit=2");
    let output = run_curl(&[
        "-s",
        "-m",
        "3",
        "-H",
        &format!("Authorization: Bearer {token}"),
        "-H",
        "Content-Type: application/json",
        &url,
    ])?;
    let spend = parse_openai_costs(&output)?;
    Some(format_api_spend(spend.value, spend.currency.as_deref()))
}

fn format_api_spend(value: f64, currency: Option<&str>) -> String {
    match currency.unwrap_or("usd") {
        "usd" => format!("API ${value:.2} 24h"),
        other => format!("API {value:.2} {} 24h", other.to_ascii_uppercase()),
    }
}

fn run_curl(args: &[&str]) -> Option<String> {
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    let output = Command::new(curl).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn read_claude_access_token(dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(dir.join(".credentials.json")).ok()?;
    let creds: ClaudeCredentials = serde_json::from_str(&raw).ok()?;
    creds.claude_ai_oauth?.access_token
}

#[derive(Debug, Deserialize)]
struct ClaudeCredentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeOauth>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOauth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
}

fn read_codex_auth(dir: &Path) -> Option<CodexAuth> {
    let raw = fs::read_to_string(dir.join("auth.json")).ok()?;
    let auth: CodexAuthFile = serde_json::from_str(&raw).ok()?;
    let tokens = auth.tokens?;
    let access_token = tokens.access_token.filter(|value| !value.is_empty())?;
    Some(CodexAuth {
        access_token,
        account_id: tokens.account_id,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexAuth {
    access_token: String,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexAuthFile {
    tokens: Option<CodexTokens>,
}

#[derive(Debug, Deserialize)]
struct CodexTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

fn parse_claude_usage(raw: &str) -> Option<UsageWindows> {
    let data: ClaudeUsageResponse = serde_json::from_str(raw).ok()?;
    Some(UsageWindows {
        five: data.five_hour.and_then(|block| block.utilization),
        seven: data.seven_day.and_then(|block| block.utilization),
    })
}

#[derive(Debug, Deserialize)]
struct ClaudeUsageResponse {
    five_hour: Option<ClaudeUsageBlock>,
    seven_day: Option<ClaudeUsageBlock>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsageBlock {
    #[serde(default, deserialize_with = "optional_usage_block")]
    utilization: Option<UsageBlock>,
}

fn parse_codex_usage(raw: &str) -> Option<UsageWindows> {
    let data: CodexUsageResponse = serde_json::from_str(raw).ok()?;
    let rate_limit = data.rate_limit?;
    Some(UsageWindows {
        five: rate_limit
            .primary_window
            .and_then(|block| block.used_percent),
        seven: rate_limit
            .secondary_window
            .and_then(|block| block.used_percent),
    })
}

#[derive(Debug, Deserialize)]
struct CodexUsageResponse {
    rate_limit: Option<CodexRateLimit>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimit {
    primary_window: Option<CodexRateLimitWindow>,
    secondary_window: Option<CodexRateLimitWindow>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimitWindow {
    #[serde(default, deserialize_with = "optional_usage_block")]
    used_percent: Option<UsageBlock>,
}

fn optional_usage_block<'de, D>(deserializer: D) -> Result<Option<UsageBlock>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<f64>::deserialize(deserializer)?;
    Ok(value.map(|used_percent| UsageBlock { used_percent }))
}

fn codex_usage_snapshot(dir: &Path) -> Option<UsageWindows> {
    let sessions = dir.join("sessions");
    let mut files = Vec::new();
    collect_codex_rollouts(&sessions, &mut files);
    files.sort_by(|left, right| right.1.cmp(&left.1));

    files
        .into_iter()
        .take(8)
        .find_map(|(file, _)| extract_codex_rate_limits(&file))
}

fn collect_codex_rollouts(dir: &Path, out: &mut Vec<(PathBuf, u128)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            collect_codex_rollouts(&path, out);
        } else if file_type.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
        {
            out.push((path.clone(), modified_millis(&path)));
        }
    }
}

fn modified_millis(path: &Path) -> u128 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn extract_codex_rate_limits(file: &Path) -> Option<UsageWindows> {
    let raw = fs::read_to_string(file).ok()?;
    for line in raw.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(payload) = value.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(|value| value.as_str()) != Some("token_count") {
            continue;
        }
        let Some(rate_limits) = payload.get("rate_limits") else {
            continue;
        };
        let primary = rate_limits.get("primary").and_then(rate_limit_block);
        let secondary = rate_limits.get("secondary").and_then(rate_limit_block);
        if primary.is_some() || secondary.is_some() {
            return Some(UsageWindows {
                five: primary,
                seven: secondary,
            });
        }
    }
    None
}

fn rate_limit_block(value: &serde_json::Value) -> Option<UsageBlock> {
    value
        .get("used_percent")
        .and_then(|value| value.as_f64())
        .map(|used_percent| UsageBlock { used_percent })
}

fn parse_openai_costs(raw: &str) -> Option<ApiSpend> {
    let data: CostsResponse = serde_json::from_str(raw).ok()?;
    let mut total = 0.0;
    let mut currency = None;

    for bucket in data.data {
        for result in bucket.results {
            let Some(amount) = result.amount else {
                continue;
            };
            let Some(value) = amount.value else {
                continue;
            };
            total += value;
            if currency.is_none() {
                currency = amount.currency;
            }
        }
    }

    Some(ApiSpend {
        value: total,
        currency,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct ApiSpend {
    value: f64,
    currency: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CostsResponse {
    #[serde(default)]
    data: Vec<CostBucket>,
}

#[derive(Debug, Deserialize)]
struct CostBucket {
    #[serde(default)]
    results: Vec<CostResult>,
}

#[derive(Debug, Deserialize)]
struct CostResult {
    amount: Option<CostAmount>,
}

#[derive(Debug, Deserialize)]
struct CostAmount {
    value: Option<f64>,
    currency: Option<String>,
}

fn profiles_home() -> PathBuf {
    env::var_os("CLAUDE_PROFILES_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".claude-profiles")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
}

fn unix_now() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_available_usage_windows() {
        assert_eq!(
            usage_label(
                Some(UsageBlock { used_percent: 60.2 }),
                Some(UsageBlock { used_percent: 8.0 })
            ),
            Some("5h 40% left / 7d 92% left".into())
        );
    }

    #[test]
    fn clamps_available_percent() {
        assert_eq!(available_percent(-12.0), 100);
        assert_eq!(available_percent(101.0), 0);
    }

    #[test]
    fn parses_codex_usage_response() {
        let usage = parse_codex_usage(
            r#"{
                "rate_limit": {
                    "primary_window": {"used_percent": 61.0},
                    "secondary_window": {"used_percent": 12.4}
                }
            }"#,
        )
        .expect("usage");

        assert_eq!(
            usage_label(usage.five, usage.seven),
            Some("5h 39% left / 7d 88% left".into())
        );
    }

    #[test]
    fn parses_openai_costs_response() {
        let spend = parse_openai_costs(
            r#"{
                "data": [
                    {"results": [{"amount": {"value": 0.06, "currency": "usd"}}]},
                    {"results": [{"amount": {"value": 1.44, "currency": "usd"}}]}
                ]
            }"#,
        )
        .expect("spend");

        assert_eq!(
            format_api_spend(spend.value, spend.currency.as_deref()),
            "API $1.50 24h"
        );
    }

    #[test]
    fn extracts_codex_rate_limits_from_rollout_line() {
        let temp = env::temp_dir().join(format!(
            "gridbash-usage-test-{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::write(
            &temp,
            r#"{"type":"session_meta","payload":{"id":"abc"}}"#.to_string()
                + "\n"
                + r#"{"payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":70},"secondary":{"used_percent":20}}}}"#,
        )
        .expect("write rollout");

        let usage = extract_codex_rate_limits(&temp).expect("limits");
        let _ = fs::remove_file(temp);

        assert_eq!(
            usage_label(usage.five, usage.seven),
            Some("5h 30% left / 7d 80% left".into())
        );
    }
}
