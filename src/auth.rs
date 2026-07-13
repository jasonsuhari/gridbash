use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const GRIDBASH_AUTH_HOME: &str = "GRIDBASH_AUTH_HOME";
const CLAUDE_PROFILES_HOME: &str = "CLAUDE_PROFILES_HOME";
const PROFILE_KIND_FILE: &str = ".profile-kind";
const CLAUDE_USAGE_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/usage";
const USAGE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_cycle: bool,
    #[serde(default, skip_serializing_if = "AuthDefaults::is_empty")]
    pub defaults: AuthDefaults,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_status: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthProfile {
    pub name: String,
    pub kind: AgentKind,
    pub dir: PathBuf,
    pub ready: bool,
    pub account_label: Option<String>,
    pub account_detail: Option<String>,
    pub usage: Option<UsageInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageInfo {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageWindow {
    pub available_percent: u8,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEnv {
    pub kind: AgentKind,
    pub name: String,
    pub dir: PathBuf,
    pub env_var: &'static str,
    pub env_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthLaunchCommand {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }

    pub fn env_var(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Codex => "CODEX_HOME",
        }
    }

    pub fn default_command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex => Self::Claude,
        }
    }
}

impl AuthConfig {
    pub fn is_empty(&self) -> bool {
        self.home.is_none()
            && !self.auto_cycle
            && self.defaults.is_empty()
            && self.usage_status.is_none()
    }

    pub fn usage_enabled(&self) -> bool {
        self.usage_status.unwrap_or(true)
    }
}

impl AuthDefaults {
    pub fn is_empty(&self) -> bool {
        self.claude.is_none() && self.codex.is_none()
    }

    pub fn get(&self, kind: AgentKind) -> Option<&str> {
        match kind {
            AgentKind::Claude => self.claude.as_deref(),
            AgentKind::Codex => self.codex.as_deref(),
        }
    }

    pub fn set(&mut self, kind: AgentKind, name: impl Into<String>) {
        match kind {
            AgentKind::Claude => self.claude = Some(name.into()),
            AgentKind::Codex => self.codex = Some(name.into()),
        }
    }
}

impl AuthProfile {
    pub fn status_label(&self) -> &'static str {
        if self.ready { "ready" } else { "login needed" }
    }
}

impl UsageInfo {
    pub fn display_label(&self) -> String {
        let five = self
            .five_hour
            .as_ref()
            .map(|value| format!("5h {}%", value.available_percent))
            .unwrap_or_else(|| "5h n/a".into());
        let seven = self
            .seven_day
            .as_ref()
            .map(|value| format!("7d {}%", value.available_percent))
            .unwrap_or_else(|| "7d n/a".into());
        format!("{five} | {seven}")
    }
}

impl AuthEnv {
    pub fn env_map(&self) -> BTreeMap<String, String> {
        BTreeMap::from([(self.env_var.to_string(), self.env_value.clone())])
    }
}

pub fn resolve_home(config: &AuthConfig) -> Result<PathBuf> {
    non_empty_env_path(GRIDBASH_AUTH_HOME)
        .or_else(|| config.home.clone())
        .or_else(|| non_empty_env_path(CLAUDE_PROFILES_HOME))
        .or_else(|| BaseDirs::new().map(|dirs| dirs.home_dir().join(".gridbash-auth")))
        .ok_or_else(|| anyhow!("failed to resolve auth profile home"))
}

pub fn discover_profiles(config: &AuthConfig) -> Result<Vec<AuthProfile>> {
    let home = resolve_home(config)?;
    if !home.exists() {
        return Ok(Vec::new());
    }

    let mut profiles = fs::read_dir(&home)
        .with_context(|| format!("failed to read auth profile home {}", home.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| is_profile_entry(entry.path(), &entry.file_name().to_string_lossy()))
        .map(|entry| {
            profile_from_dir(
                entry.file_name().to_string_lossy().to_string(),
                entry.path(),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    profiles.sort_by(|left, right| {
        left.kind
            .as_str()
            .cmp(right.kind.as_str())
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(profiles)
}

pub fn discover_profiles_with_usage(config: &AuthConfig) -> Result<Vec<AuthProfile>> {
    let mut profiles = discover_profiles(config)?;
    if !config.usage_enabled() {
        return Ok(profiles);
    }

    for profile in &mut profiles {
        profile.usage = fetch_usage(profile).unwrap_or(None);
    }
    Ok(profiles)
}

pub fn create_profile(config: &AuthConfig, kind: AgentKind, name: &str) -> Result<AuthProfile> {
    validate_profile_name(name)?;
    let home = resolve_home(config)?;
    fs::create_dir_all(&home)
        .with_context(|| format!("failed to create auth profile home {}", home.display()))?;
    let dir = home.join(name);
    if dir.exists() {
        return Err(anyhow!("auth profile already exists: {name}"));
    }

    fs::create_dir(&dir)
        .with_context(|| format!("failed to create auth profile {}", dir.display()))?;
    fs::write(dir.join(PROFILE_KIND_FILE), kind.as_str())
        .with_context(|| format!("failed to write profile kind for {}", dir.display()))?;
    fs::write(dir.join("PROFILE.md"), profile_readme(kind, name))
        .with_context(|| format!("failed to write profile readme for {}", dir.display()))?;

    profile_from_dir(name.to_string(), dir)
}

pub fn next_profile_name(config: &AuthConfig, kind: AgentKind) -> Result<String> {
    let profiles = discover_profiles(config)?;
    let names = profiles
        .into_iter()
        .map(|profile| profile.name)
        .collect::<std::collections::BTreeSet<_>>();
    for index in 1..=999 {
        let candidate = format!("{}-{index}", kind.as_str());
        if !names.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "failed to find available {} profile name",
        kind.as_str()
    ))
}

pub fn env_for_default(config: &AuthConfig, kind: AgentKind) -> Result<Option<AuthEnv>> {
    let Some(name) = config.defaults.get(kind) else {
        return Ok(None);
    };
    env_for_profile(config, kind, name).map(Some)
}

pub fn env_for_profile(config: &AuthConfig, kind: AgentKind, name: &str) -> Result<AuthEnv> {
    validate_profile_name(name)?;
    let dir = resolve_home(config)?.join(name);
    if !dir.is_dir() {
        return Err(anyhow!(
            "configured {} auth profile not found: {}",
            kind.as_str(),
            name
        ));
    }
    let actual_kind = profile_kind(&dir);
    if actual_kind != kind {
        return Err(anyhow!(
            "configured auth profile '{}' is {}, not {}",
            name,
            actual_kind.as_str(),
            kind.as_str()
        ));
    }
    Ok(AuthEnv {
        kind,
        name: name.to_string(),
        env_var: kind.env_var(),
        env_value: dir.display().to_string(),
        dir,
    })
}

pub fn login_command(profile: &AuthProfile) -> AuthLaunchCommand {
    let command = resolve_agent_executable(profile.kind)
        .unwrap_or_else(|| PathBuf::from(profile.kind.default_command()));
    let args = match profile.kind {
        AgentKind::Claude => vec!["auth".into(), "login".into()],
        AgentKind::Codex => vec!["login".into()],
    };
    AuthLaunchCommand {
        command,
        args,
        env: BTreeMap::from([(
            profile.kind.env_var().to_string(),
            profile.dir.display().to_string(),
        )]),
    }
}

pub fn resolve_agent_executable(kind: AgentKind) -> Option<PathBuf> {
    match kind {
        AgentKind::Claude => non_empty_env_path("CLAUDE_BIN")
            .filter(|path| path.exists())
            .or_else(resolve_installed_claude)
            .or_else(|| resolve_on_path("claude")),
        AgentKind::Codex => non_empty_env_path("CODEX_BIN")
            .filter(|path| path.exists())
            .or_else(resolve_installed_codex)
            .or_else(|| resolve_on_path("codex")),
    }
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).and_then(|value| {
        let path = PathBuf::from(value);
        (!path.as_os_str().is_empty()).then_some(path)
    })
}

fn resolve_installed_claude() -> Option<PathBuf> {
    let appdata = env::var_os("APPDATA")?;
    let candidate = PathBuf::from(appdata)
        .join("npm")
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("bin")
        .join("claude.exe");
    candidate.is_file().then_some(candidate)
}

fn resolve_installed_codex() -> Option<PathBuf> {
    let appdata = env::var_os("APPDATA")?;
    let candidate = PathBuf::from(appdata).join("npm").join("codex.exe");
    candidate.is_file().then_some(candidate)
}

fn resolve_on_path(command: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let pathext: Vec<String> = env::var_os("PATHEXT")
        .map(|value| {
            env::split_paths(&value)
                .filter_map(|p| p.to_str().map(|s| s.trim_start_matches('.').to_string()))
                .collect()
        })
        .unwrap_or_else(|| vec!["exe".into(), "cmd".into(), "bat".into(), "ps1".into()]);

    for dir in env::split_paths(&path) {
        for ext in &pathext {
            let candidate = dir.join(format!("{command}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        let direct = dir.join(command);
        if direct.is_file() {
            return Some(direct);
        }
    }
    None
}

fn is_profile_entry(path: PathBuf, name: &str) -> bool {
    path.is_dir() && !name.starts_with('.') && !name.ends_with(".lock")
}

fn profile_from_dir(name: String, dir: PathBuf) -> Result<AuthProfile> {
    let kind = profile_kind(&dir);
    let ready = has_auth_files(&dir, kind);
    let (account_label, account_detail) = read_account(&dir, kind);
    Ok(AuthProfile {
        name,
        kind,
        dir,
        ready,
        account_label,
        account_detail,
        usage: None,
    })
}

fn profile_kind(dir: &Path) -> AgentKind {
    match fs::read_to_string(dir.join(PROFILE_KIND_FILE)) {
        Ok(raw) if raw.trim().eq_ignore_ascii_case("codex") => AgentKind::Codex,
        _ => AgentKind::Claude,
    }
}

fn has_auth_files(dir: &Path, kind: AgentKind) -> bool {
    match kind {
        AgentKind::Claude => [".credentials.json", "accounts", ".claude.json"]
            .iter()
            .any(|name| dir.join(name).exists()),
        AgentKind::Codex => dir.join("auth.json").exists(),
    }
}

fn read_account(dir: &Path, kind: AgentKind) -> (Option<String>, Option<String>) {
    match kind {
        AgentKind::Claude => read_claude_email(dir)
            .map(|email| (Some(mask_email(&email)), None))
            .unwrap_or((None, None)),
        AgentKind::Codex => {
            let account = read_codex_account(dir);
            (
                account.email.as_deref().map(mask_email),
                account.plan.map(|plan| plan.to_ascii_lowercase()),
            )
        }
    }
}

fn read_claude_email(dir: &Path) -> Option<String> {
    let data = read_json_file(&dir.join(".claude.json"))?;
    data.pointer("/oauthAccount/emailAddress")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Default)]
struct CodexAccount {
    email: Option<String>,
    plan: Option<String>,
}

fn read_codex_account(dir: &Path) -> CodexAccount {
    let Some(auth) = read_json_file(&dir.join("auth.json")) else {
        return CodexAccount::default();
    };
    let Some(token) = auth.pointer("/tokens/id_token").and_then(Value::as_str) else {
        return CodexAccount::default();
    };
    let Some(claims) = decode_jwt_payload(token) else {
        return CodexAccount::default();
    };
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let plan = claims
        .get("https://api.openai.com/auth")
        .and_then(|inner| inner.get("chatgpt_plan_type"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    CodexAccount { email, plan }
}

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let encoded = token.split('.').nth(1)?;
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| general_purpose::URL_SAFE.decode(encoded))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn fetch_usage(profile: &AuthProfile) -> Result<Option<UsageInfo>> {
    match profile.kind {
        AgentKind::Claude => fetch_claude_usage(&profile.dir),
        AgentKind::Codex => fetch_codex_usage(&profile.dir),
    }
}

fn fetch_claude_usage(dir: &Path) -> Result<Option<UsageInfo>> {
    let Some(creds) = read_json_file(&dir.join(".credentials.json")) else {
        return Ok(None);
    };
    let Some(token) = creds
        .pointer("/claudeAiOauth/accessToken")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let body = curl_json(
        CLAUDE_USAGE_ENDPOINT,
        &[
            ("Authorization", format!("Bearer {token}")),
            ("anthropic-beta", "oauth-2025-04-20".into()),
        ],
    )?;
    Ok(parse_usage_response(&body))
}

fn fetch_codex_usage(dir: &Path) -> Result<Option<UsageInfo>> {
    let Some(auth) = read_json_file(&dir.join("auth.json")) else {
        return Ok(None);
    };
    let Some(token) = auth.pointer("/tokens/access_token").and_then(Value::as_str) else {
        return Ok(None);
    };
    let mut headers = vec![
        ("Authorization", format!("Bearer {token}")),
        ("User-Agent", "codex-cli".into()),
    ];
    if let Some(account_id) = auth.pointer("/tokens/account_id").and_then(Value::as_str) {
        headers.push(("chatgpt-account-id", account_id.to_string()));
    }
    let body = curl_json(CODEX_USAGE_ENDPOINT, &headers)?;
    Ok(parse_usage_response(&body))
}

fn curl_json(url: &str, headers: &[(&str, String)]) -> Result<Value> {
    let mut args = vec![
        "-s".to_string(),
        "-m".to_string(),
        USAGE_TIMEOUT.as_secs().to_string(),
    ];
    for (name, value) in headers {
        args.push("-H".into());
        args.push(format!("{name}: {value}"));
    }
    args.push(url.to_string());

    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    let output = Command::new(curl)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {curl} for usage status"))?;
    if !output.status.success() {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_slice(&output.stdout).unwrap_or(Value::Null))
}

fn parse_usage_response(value: &Value) -> Option<UsageInfo> {
    let five_hour = parse_usage_window(value.get("five_hour"));
    let seven_day = parse_usage_window(value.get("seven_day"));
    (five_hour.is_some() || seven_day.is_some()).then_some(UsageInfo {
        five_hour,
        seven_day,
    })
}

fn parse_usage_window(value: Option<&Value>) -> Option<UsageWindow> {
    let value = value?;
    let used_percent = value.get("utilization").and_then(Value::as_f64)?;
    let available_percent = (100.0 - used_percent).round().clamp(0.0, 100.0) as u8;
    let resets_at = value
        .get("resets_at")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(UsageWindow {
        available_percent,
        resets_at,
    })
}

fn read_json_file(path: &Path) -> Option<Value> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn mask_email(email: &str) -> String {
    let Some((local, domain)) = email.split_once('@') else {
        return email.to_string();
    };
    if local.len() > 4 {
        format!("{}...@{domain}", &local[..4])
    } else {
        format!("{local}@{domain}")
    }
}

fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        || !name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
    {
        return Err(anyhow!(
            "invalid auth profile name '{name}'; use letters, numbers, dot, underscore, or dash"
        ));
    }
    Ok(())
}

fn profile_readme(kind: AgentKind, name: &str) -> String {
    let tool = match kind {
        AgentKind::Claude => "Claude Code",
        AgentKind::Codex => "Codex",
    };
    format!(
        "# {tool} profile: {name}\n\nThis directory is selected with {}.\n{tool} stores this profile's auth, settings, sessions, and history here.\n\nDo not commit credentials from this directory.\n",
        kind.env_var()
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TempHome {
        path: PathBuf,
    }

    impl TempHome {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = env::temp_dir().join(format!("gridbash-auth-test-{nonce}"));
            fs::create_dir_all(&path).expect("temp home");
            Self { path }
        }

        fn config(&self) -> AuthConfig {
            AuthConfig {
                home: Some(self.path.clone()),
                auto_cycle: false,
                defaults: AuthDefaults::default(),
                usage_status: Some(false),
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn discovers_claude_and_codex_profiles() {
        let temp = TempHome::new();
        let claude = temp.path.join("claude-1");
        let codex = temp.path.join("codex-2");
        fs::create_dir(&claude).expect("claude dir");
        fs::create_dir(&codex).expect("codex dir");
        fs::write(
            claude.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"jason@example.com"}}"#,
        )
        .expect("claude auth");
        fs::write(codex.join(PROFILE_KIND_FILE), "codex").expect("kind");
        fs::write(codex.join("auth.json"), r#"{"tokens":{}}"#).expect("codex auth");

        let profiles = discover_profiles(&temp.config()).expect("profiles");

        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].kind, AgentKind::Claude);
        assert_eq!(
            profiles[0].account_label.as_deref(),
            Some("jaso...@example.com")
        );
        assert!(profiles[0].ready);
        assert_eq!(profiles[1].kind, AgentKind::Codex);
        assert!(profiles[1].ready);
    }

    #[test]
    fn builds_default_env_for_kind() {
        let temp = TempHome::new();
        let dir = temp.path.join("codex-1");
        fs::create_dir(&dir).expect("dir");
        fs::write(dir.join(PROFILE_KIND_FILE), "codex").expect("kind");
        let mut config = temp.config();
        config.defaults.set(AgentKind::Codex, "codex-1");

        let env = env_for_default(&config, AgentKind::Codex)
            .expect("env")
            .expect("present");

        assert_eq!(env.env_var, "CODEX_HOME");
        assert_eq!(env.name, "codex-1");
    }

    #[test]
    fn rejects_wrong_kind_default() {
        let temp = TempHome::new();
        fs::create_dir(temp.path.join("claude-1")).expect("dir");
        let mut config = temp.config();
        config.defaults.set(AgentKind::Codex, "claude-1");

        let error = env_for_default(&config, AgentKind::Codex).expect_err("wrong kind");

        assert!(error.to_string().contains("not codex"));
    }

    #[test]
    fn creates_profile_with_marker_and_readme() {
        let temp = TempHome::new();

        let profile = create_profile(&temp.config(), AgentKind::Codex, "codex-1").expect("create");

        assert_eq!(profile.kind, AgentKind::Codex);
        assert_eq!(
            fs::read_to_string(profile.dir.join(PROFILE_KIND_FILE)).expect("kind"),
            "codex"
        );
        assert!(profile.dir.join("PROFILE.md").is_file());
    }

    #[test]
    fn decodes_codex_account_without_exposing_token() {
        let temp = TempHome::new();
        let dir = temp.path.join("codex-1");
        fs::create_dir(&dir).expect("dir");
        let claims = serde_json::json!({
            "email": "longperson@example.com",
            "https://api.openai.com/auth": {"chatgpt_plan_type": "Plus"}
        });
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
        fs::write(
            dir.join("auth.json"),
            format!(r#"{{"tokens":{{"id_token":"header.{payload}.sig"}}}}"#),
        )
        .expect("auth");

        let (label, detail) = read_account(&dir, AgentKind::Codex);

        assert_eq!(label.as_deref(), Some("long...@example.com"));
        assert_eq!(detail.as_deref(), Some("plus"));
    }

    #[test]
    fn formats_usage_response_as_available_percent() {
        let value = serde_json::json!({
            "five_hour": {"utilization": 73.0, "resets_at": "2026-07-07T12:00:00Z"}
        });

        let usage = parse_usage_response(&value).expect("usage");

        assert_eq!(usage.five_hour.unwrap().available_percent, 27);
    }

    #[test]
    fn treats_low_usage_utilization_as_a_percentage() {
        for (utilization, available_percent) in [(0.5, 100), (1.0, 99)] {
            let value = serde_json::json!({"utilization": utilization});
            let usage = parse_usage_window(Some(&value)).expect("usage window");

            assert_eq!(usage.available_percent, available_percent);
        }
    }
}
