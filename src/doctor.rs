//! Reports whether this terminal can deliver the Alt shortcuts, and offers to
//! fix the setting that stops it.
//!
//! Every GridBash shortcut is an Alt chord, and macOS terminals spend Option on
//! character composition by default: Option+C types `ç` and Option+N starts a
//! dead key, so the chord never reaches the application at all. The leader key
//! keeps GridBash usable regardless, but the setting is one line to change and
//! most people would rather press the shortcut the interface names.

use std::{
    env, fs,
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

// Only the macOS probes and fixes shell out; elsewhere Option is not special
// and there is nothing to read or change.
#[cfg(target_os = "macos")]
use std::process::Command;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};

use crate::{
    config::Config,
    keybindings::KeyBindings,
    profiles::{default_profile_name, profile_diagnostics},
};

/// What the host terminal does with the Option/Alt key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaState {
    /// Alt reaches GridBash, so every shortcut works as labelled.
    Sends,
    /// Option is spent on character composition and no Alt chord arrives.
    Composes,
    /// The terminal is not one this knows how to interrogate.
    Unknown,
}

/// A terminal this knows how to check, and in most cases configure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostTerminal {
    AppleTerminal,
    ITerm2,
    Ghostty,
    Kitty,
    WezTerm,
    Alacritty,
    VsCode,
    Other(String),
}

impl HostTerminal {
    pub fn detect() -> Self {
        // A terminal's own marker beats TERM, which a multiplexer or a remote
        // shell may have rewritten to something generic.
        if env::var_os("KITTY_WINDOW_ID").is_some() || term_is("xterm-kitty") {
            return Self::Kitty;
        }
        if env::var_os("ALACRITTY_WINDOW_ID").is_some() || term_is("alacritty") {
            return Self::Alacritty;
        }
        match env::var("TERM_PROGRAM").unwrap_or_default().as_str() {
            "Apple_Terminal" => Self::AppleTerminal,
            "iTerm.app" => Self::ITerm2,
            "ghostty" => Self::Ghostty,
            "WezTerm" => Self::WezTerm,
            "vscode" => Self::VsCode,
            "" => Self::Other("unknown".into()),
            other => Self::Other(other.to_string()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::AppleTerminal => "Apple Terminal".into(),
            Self::ITerm2 => "iTerm2".into(),
            Self::Ghostty => "Ghostty".into(),
            Self::Kitty => "kitty".into(),
            Self::WezTerm => "WezTerm".into(),
            Self::Alacritty => "Alacritty".into(),
            Self::VsCode => "Visual Studio Code".into(),
            Self::Other(name) => name.clone(),
        }
    }

    /// How to turn Option into Alt by hand, for when the fix cannot be applied
    /// or the user would rather do it themselves.
    fn manual_instructions(&self) -> &'static str {
        match self {
            Self::AppleTerminal => {
                "Terminal > Settings > Profiles > Keyboard > tick \"Use Option as Meta Key\""
            }
            Self::ITerm2 => {
                "iTerm2 > Settings > Profiles > Keys > General > set \"Left Option key\" to \"Esc+\""
            }
            Self::Ghostty => "add `macos-option-as-alt = true` to the Ghostty config",
            Self::Kitty => "add `macos_option_as_alt yes` to kitty.conf",
            Self::WezTerm => {
                "set `send_composed_key_when_left_alt_is_pressed = false` in the WezTerm config"
            }
            Self::Alacritty => "set `[keyboard] option_as_alt = \"Both\"` in the Alacritty config",
            Self::VsCode => {
                "VS Code's terminal sends Option as Alt when `terminal.integrated.macOptionIsMeta` is true"
            }
            Self::Other(_) => {
                "look for an \"Option as Meta\", \"Option as Alt\", or \"Esc+\" setting"
            }
        }
    }
}

fn term_is(name: &str) -> bool {
    env::var("TERM").is_ok_and(|term| term == name)
}

/// A configuration file whose presence or absence answers the question.
struct ConfigFileProbe {
    path: PathBuf,
    /// The setting that turns Option into Alt, written as the file wants it.
    line: &'static str,
    /// The setting's key, so an existing entry can be recognized.
    key: &'static str,
    /// Values of that key which mean Option already arrives as Alt.
    enabled: &'static [&'static str],
    /// Whether the terminal already behaves this way with no setting at all.
    default_sends: bool,
}

impl HostTerminal {
    fn config_probe(&self) -> Option<ConfigFileProbe> {
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        match self {
            Self::Ghostty => Some(ConfigFileProbe {
                path: config_home.join("ghostty").join("config"),
                line: "macos-option-as-alt = true",
                key: "macos-option-as-alt",
                enabled: &["true", "left", "right"],
                default_sends: false,
            }),
            Self::Kitty => Some(ConfigFileProbe {
                path: config_home.join("kitty").join("kitty.conf"),
                line: "macos_option_as_alt yes",
                key: "macos_option_as_alt",
                enabled: &["yes", "y", "true", "both", "left", "right"],
                default_sends: false,
            }),
            Self::Alacritty => Some(ConfigFileProbe {
                path: config_home.join("alacritty").join("alacritty.toml"),
                line: "option_as_alt = \"Both\"",
                key: "option_as_alt",
                enabled: &["\"both\"", "\"onlyleft\"", "\"onlyright\""],
                default_sends: false,
            }),
            // WezTerm composes only when explicitly told to, so an untouched
            // install already sends Alt.
            Self::WezTerm => Some(ConfigFileProbe {
                path: config_home.join("wezterm").join("wezterm.lua"),
                line: "send_composed_key_when_left_alt_is_pressed = false",
                key: "send_composed_key_when_left_alt_is_pressed",
                enabled: &["false"],
                default_sends: true,
            }),
            _ => None,
        }
    }
}

fn probe_config_file(probe: &ConfigFileProbe) -> MetaState {
    let Ok(contents) = fs::read_to_string(&probe.path) else {
        return if probe.default_sends {
            MetaState::Sends
        } else {
            MetaState::Composes
        };
    };

    let mut setting = None;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('#') || !line.contains(probe.key) {
            continue;
        }
        // `key = value`, `key value`, and `key: value` all appear across these
        // formats, so the value is whatever follows the key.
        let Some((_, value)) = line.split_once(probe.key) else {
            continue;
        };
        let value = value
            .trim()
            .trim_start_matches(['=', ':'])
            .trim()
            .trim_end_matches(',')
            .trim()
            .to_ascii_lowercase();
        setting = Some(value);
    }

    match setting {
        Some(value) => {
            if probe.enabled.iter().any(|allowed| *allowed == value) {
                MetaState::Sends
            } else {
                MetaState::Composes
            }
        }
        None if probe.default_sends => MetaState::Sends,
        None => MetaState::Composes,
    }
}

#[cfg(target_os = "macos")]
const APPLE_TERMINAL_DOMAIN: &str = "com.apple.Terminal";

/// The Apple Terminal profile whose settings a new window would use.
#[cfg(target_os = "macos")]
fn apple_terminal_profile() -> Option<String> {
    for key in ["Startup Window Settings", "Default Window Settings"] {
        let output = Command::new("defaults")
            .args(["read", APPLE_TERMINAL_DOMAIN, key])
            .output()
            .ok()?;
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn apple_terminal_plist() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Preferences")
            .join("com.apple.Terminal.plist"),
    )
}

#[cfg(target_os = "macos")]
fn plist_buddy(plist: &PathBuf, command: &str) -> Option<String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(command)
        .arg(plist)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Every Apple Terminal profile, and whether Option is Meta in it.
///
/// The setting is per-profile, and the window in front of the user need not be
/// using the profile new windows are opened with. Listing them all is what
/// makes "the default says yes but my window says no" diagnosable instead of
/// baffling.
#[cfg(target_os = "macos")]
fn apple_terminal_profiles() -> Vec<(String, bool)> {
    let Some(plist) = apple_terminal_plist().filter(|plist| plist.is_file()) else {
        return Vec::new();
    };
    let Some(listing) = plist_buddy(&plist, "Print :'Window Settings'") else {
        return Vec::new();
    };

    // PlistBuddy prints a nested dictionary; a profile is a key at one indent
    // level, and its settings are indented under it.
    let mut profiles = Vec::new();
    let mut current: Option<(String, bool)> = None;
    for line in listing.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if indent == 4 && trimmed.ends_with("= Dict {") {
            if let Some(profile) = current.take() {
                profiles.push(profile);
            }
            let name = trimmed.trim_end_matches("= Dict {").trim().to_string();
            current = Some((name, false));
        } else if let Some((_, meta)) = current.as_mut()
            && trimmed.starts_with("useOptionAsMetaKey")
        {
            *meta = trimmed.ends_with("true");
        }
    }
    profiles.extend(current);
    profiles
}

/// Whether Apple Terminal is running, which decides whether writing its
/// preferences is worth attempting.
#[cfg(target_os = "macos")]
fn apple_terminal_is_running() -> bool {
    // `pgrep -x Terminal` does not match, because the process reports its full
    // bundle path rather than a bare name.
    Command::new("pgrep")
        .args(["-f", "MacOS/Terminal"])
        .output()
        .is_ok_and(|output| !output.stdout.is_empty())
}

#[cfg(target_os = "macos")]
fn apple_terminal_state() -> MetaState {
    let (Some(plist), Some(profile)) = (apple_terminal_plist(), apple_terminal_profile()) else {
        return MetaState::Unknown;
    };
    // Without the file there is no answer to read. Reporting "composes" here
    // would relabel every shortcut on the word of a preferences file that a
    // relocated HOME simply put somewhere else.
    if !plist.is_file() {
        return MetaState::Unknown;
    }
    match plist_buddy(
        &plist,
        &format!("Print :'Window Settings':'{profile}':useOptionAsMetaKey"),
    ) {
        Some(value) => {
            if value.eq_ignore_ascii_case("true") {
                MetaState::Sends
            } else {
                MetaState::Composes
            }
        }
        // The key is absent until it is first switched on.
        None => MetaState::Composes,
    }
}

/// iTerm2 writes `Option Key Sends` per profile: 0 normal, 1 Meta, 2 Esc+.
#[cfg(target_os = "macos")]
fn iterm2_state() -> MetaState {
    let Some(plist) = env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Preferences")
            .join("com.googlecode.iterm2.plist")
    }) else {
        return MetaState::Unknown;
    };
    match plist_buddy(&plist, "Print :'New Bookmarks':0:'Option Key Sends'") {
        Some(value) => match value.trim() {
            "1" | "2" => MetaState::Sends,
            _ => MetaState::Composes,
        },
        None => MetaState::Unknown,
    }
}

/// What this terminal currently does with Option.
pub fn meta_state(terminal: &HostTerminal) -> MetaState {
    if !cfg!(target_os = "macos") {
        // Option is only special on macOS; every other platform's Alt arrives
        // as a modifier.
        return MetaState::Sends;
    }

    #[cfg(target_os = "macos")]
    match terminal {
        HostTerminal::AppleTerminal => return apple_terminal_state(),
        HostTerminal::ITerm2 => return iterm2_state(),
        HostTerminal::VsCode => return MetaState::Unknown,
        _ => {}
    }

    terminal
        .config_probe()
        .map_or(MetaState::Unknown, |probe| probe_config_file(&probe))
}

/// Every profile the terminal knows about, where that is a thing it has.
fn terminal_profiles(terminal: &HostTerminal) -> Vec<(String, bool)> {
    #[cfg(target_os = "macos")]
    if *terminal == HostTerminal::AppleTerminal {
        return apple_terminal_profiles();
    }
    let _ = terminal;
    Vec::new()
}

/// Where an Option answer was read from, when it was read at all.
fn meta_source(terminal: &HostTerminal) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }

    #[cfg(target_os = "macos")]
    match terminal {
        HostTerminal::AppleTerminal => {
            return apple_terminal_profile()
                .map(|profile| format!("the Terminal profile \"{profile}\""));
        }
        HostTerminal::ITerm2 => return Some("iTerm2's first profile".into()),
        _ => {}
    }

    terminal
        .config_probe()
        .filter(|probe| probe.path.is_file())
        .map(|probe| probe.path.display().to_string())
}

/// Everything the doctor found, so the printing and the fixing agree.
pub struct Report {
    pub terminal: HostTerminal,
    pub meta: MetaState,
    /// Where the Option answer came from, when it was read rather than measured.
    pub meta_source: Option<String>,
    /// Every profile the terminal has, and whether Option is Meta in each.
    pub profiles: Vec<(String, bool)>,
    pub leader: Option<String>,
    pub shell_profile: String,
    pub shell_available: bool,
}

impl Report {
    pub fn detect(config: &Config) -> Self {
        let terminal = HostTerminal::detect();
        let meta = meta_state(&terminal);
        let meta_source = meta_source(&terminal);
        let profiles = terminal_profiles(&terminal);
        let leader = KeyBindings::from_overrides(&config.keys)
            .ok()
            .and_then(|bindings| bindings.leader_label());
        let shell_profile = config
            .defaults
            .profile
            .clone()
            .unwrap_or_else(|| default_profile_name().to_string());
        let shell_available = profile_diagnostics(config)
            .into_iter()
            .any(|profile| profile.name == shell_profile && profile.executable.is_some());

        Self {
            terminal,
            meta,
            meta_source,
            profiles,
            leader,
            shell_profile,
            shell_available,
        }
    }

    fn print(&self, output: &mut impl Write) -> Result<()> {
        writeln!(output, "Terminal    {}", self.terminal.label())?;
        writeln!(
            output,
            "Shell       {} ({})",
            self.shell_profile,
            if self.shell_available {
                "available"
            } else {
                "not found on PATH"
            }
        )?;

        match self.meta {
            MetaState::Sends => {
                writeln!(output, "Option key  sends Alt — every shortcut works")?;
            }
            MetaState::Composes => {
                writeln!(
                    output,
                    "Option key  composes characters — Alt chords cannot reach GridBash"
                )?;
            }
            MetaState::Unknown => {
                writeln!(
                    output,
                    "Option key  could not be determined for this terminal"
                )?;
            }
        }

        // The answer above is read out of a preferences file, not measured, and
        // the file describes the profile new windows use — which need not be the
        // profile of the window this is running in. Saying where it came from is
        // what makes a wrong answer recognisable as one.
        if let Some(source) = &self.meta_source {
            writeln!(output, "            read from {source}")?;
            writeln!(
                output,
                "            `gridbash doctor --keys` measures this window instead"
            )?;
        }

        match (&self.leader, self.meta) {
            (Some(leader), MetaState::Sends) => writeln!(
                output,
                "Shortcuts   Alt+N opens a tab. {leader} then N does the same."
            )?,
            (Some(leader), _) => writeln!(
                output,
                "Shortcuts   {leader} then N opens a tab, and works right now."
            )?,
            (None, _) => writeln!(output, "Shortcuts   Alt+N opens a tab.")?,
        }

        // A per-profile setting read from one profile explains nothing on its
        // own. Seeing every profile is what turns "it says yes but behaves like
        // no" into "this window is using a different profile".
        if !self.profiles.is_empty() {
            writeln!(output)?;
            writeln!(output, "Option as Meta, per profile:")?;
            let width = self
                .profiles
                .iter()
                .map(|(name, _)| name.chars().count())
                .max()
                .unwrap_or(0);
            for (name, meta) in &self.profiles {
                writeln!(
                    output,
                    "  {name:<width$}  {}",
                    if *meta { "on" } else { "off" }
                )?;
            }
            if self.profiles.iter().any(|(_, meta)| *meta)
                && self.profiles.iter().any(|(_, meta)| !*meta)
            {
                writeln!(
                    output,
                    "  Shortcuts work only in the profiles marked on. Check which one this"
                )?;
                writeln!(
                    output,
                    "  window uses under Terminal > Settings > Profiles."
                )?;
            }
        }

        if self.meta != MetaState::Sends {
            writeln!(output)?;
            writeln!(output, "To make Option send Alt:")?;
            writeln!(output, "  {}", self.terminal.manual_instructions())?;
        }

        Ok(())
    }

    /// Whether there is a setting worth offering to change.
    fn fixable(&self) -> bool {
        self.meta == MetaState::Composes && self.fix().is_some()
    }

    fn fix(&self) -> Option<Fix> {
        #[cfg(target_os = "macos")]
        match self.terminal {
            HostTerminal::AppleTerminal => return Fix::apple_terminal(),
            HostTerminal::ITerm2 => return Fix::iterm2(),
            _ => {}
        }
        self.terminal.config_probe().map(Fix::ConfigFile)
    }
}

/// A change the doctor knows how to make, and can describe before making it.
enum Fix {
    /// Tick "Use Option as Meta Key" for one Apple Terminal profile.
    #[cfg(target_os = "macos")]
    AppleTerminal { plist: PathBuf, profile: String },
    /// Set "Option Key Sends" to Esc+ for iTerm2's first profile.
    #[cfg(target_os = "macos")]
    ITerm2 { plist: PathBuf },
    /// Append a setting line to a terminal's own config file.
    ConfigFile(ConfigFileProbe),
}

impl Fix {
    #[cfg(target_os = "macos")]
    fn apple_terminal() -> Option<Self> {
        // Terminal keeps its settings in memory and writes them out when it
        // quits, so anything written underneath a running Terminal is
        // overwritten the moment the user closes it. Offering a fix that
        // silently undoes itself is worse than offering none.
        if apple_terminal_is_running() {
            return None;
        }
        Some(Self::AppleTerminal {
            plist: apple_terminal_plist()?,
            profile: apple_terminal_profile()?,
        })
    }

    #[cfg(target_os = "macos")]
    fn iterm2() -> Option<Self> {
        let plist = env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Preferences")
                .join("com.googlecode.iterm2.plist")
        })?;
        plist.is_file().then_some(Self::ITerm2 { plist })
    }

    /// What this will change, in the user's own terms, before it happens.
    fn describe(&self) -> String {
        match self {
            #[cfg(target_os = "macos")]
            Self::AppleTerminal { profile, .. } => {
                format!("tick \"Use Option as Meta Key\" for the Terminal profile \"{profile}\"")
            }
            #[cfg(target_os = "macos")]
            Self::ITerm2 { .. } => {
                "set iTerm2's \"Left Option key\" to \"Esc+\" for its first profile".into()
            }
            Self::ConfigFile(probe) => {
                format!("add `{}` to {}", probe.line, probe.path.display())
            }
        }
    }

    /// What has to happen before the change takes effect.
    fn restart_note(&self) -> &'static str {
        match self {
            #[cfg(target_os = "macos")]
            Self::AppleTerminal { .. } => {
                "Quit Terminal completely (Cmd+Q) and reopen it — Terminal rewrites its preferences on quit, so a change made while it runs is lost otherwise."
            }
            #[cfg(target_os = "macos")]
            Self::ITerm2 { .. } => "Restart iTerm2 for the profile change to load.",
            Self::ConfigFile(_) => "Reload the terminal's config or open a new window.",
        }
    }

    fn apply(&self) -> Result<()> {
        match self {
            #[cfg(target_os = "macos")]
            Self::AppleTerminal { plist, profile } => {
                let entry = format!("Print :'Window Settings':'{profile}':useOptionAsMetaKey");
                let write = if plist_buddy(plist, &entry).is_some() {
                    format!("Set :'Window Settings':'{profile}':useOptionAsMetaKey true")
                } else {
                    format!("Add :'Window Settings':'{profile}':useOptionAsMetaKey bool true")
                };
                run_plist_buddy(plist, &write)?;
                reload_preferences();
                Ok(())
            }
            #[cfg(target_os = "macos")]
            Self::ITerm2 { plist } => {
                let entry = "Print :'New Bookmarks':0:'Option Key Sends'";
                let write = if plist_buddy(plist, entry).is_some() {
                    "Set :'New Bookmarks':0:'Option Key Sends' 2".to_string()
                } else {
                    "Add :'New Bookmarks':0:'Option Key Sends' integer 2".to_string()
                };
                run_plist_buddy(plist, &write)?;
                reload_preferences();
                Ok(())
            }
            Self::ConfigFile(probe) => {
                if let Some(parent) = probe.path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("failed to create {} for the config", parent.display())
                    })?;
                }
                let existing = fs::read_to_string(&probe.path).unwrap_or_default();
                let mut updated = existing.clone();
                if !updated.is_empty() && !updated.ends_with('\n') {
                    updated.push('\n');
                }
                updated
                    .push_str("\n# added by `gridbash doctor` so Option reaches GridBash as Alt\n");
                updated.push_str(probe.line);
                updated.push('\n');
                fs::write(&probe.path, updated)
                    .with_context(|| format!("failed to write {}", probe.path.display()))?;
                Ok(())
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn run_plist_buddy(plist: &PathBuf, command: &str) -> Result<()> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(command)
        .arg(plist)
        .output()
        .with_context(|| format!("failed to run PlistBuddy on {}", plist.display()))?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "PlistBuddy could not apply the change: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

/// Drops the preference daemon's cache so the edited file is what is read next.
#[cfg(target_os = "macos")]
fn reload_preferences() {
    let _ = Command::new("killall").arg("cfprefsd").output();
}

/// Reads keys straight from the terminal and says what each one arrived as.
///
/// Everything else here infers the answer from a preferences file, which is a
/// guess: it reads the profile new windows are opened with, and the window the
/// user is actually sitting in may have been opened with another one, or may
/// belong to a different terminal altogether. This measures instead.
fn probe_keys() -> Result<()> {
    println!();
    println!("Press the shortcuts you want to check. Ctrl+C stops.");
    println!("Try Option+K, Option+Shift+C, and Option+Left.");
    println!();

    enable_raw_mode().context("failed to put the terminal in raw mode")?;
    let outcome = read_probe_keys();
    let restored = disable_raw_mode();
    outcome?;
    restored.context("failed to restore the terminal")?;

    println!();
    Ok(())
}

fn read_probe_keys() -> Result<()> {
    // A terminal that cannot report Alt sends it as a leading Esc, so an Alt
    // chord arrives here in pieces. They are gathered up and reported as the
    // one chord they stand for, which is also what GridBash itself does with
    // them.
    let mut split: Option<SplitChord> = None;

    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }

        match split.take() {
            None => {
                if key.code == KeyCode::Esc && key.modifiers.is_empty() {
                    // Nothing is reported yet: what comes next decides whether
                    // this was the Esc key or the start of an Alt chord.
                    split = Some(SplitChord::Escape);
                    continue;
                }
                let (arrived, verdict) = describe_key(key, false);
                report_key(&arrived, &verdict);
            }
            Some(SplitChord::Escape) => match key.code {
                KeyCode::Char(byte @ ('[' | 'O')) => {
                    split = Some(SplitChord::Sequence {
                        introducer: byte,
                        params: String::new(),
                    });
                }
                _ => {
                    report_key("Esc", "the Esc key, on its own");
                    let (arrived, verdict) = describe_key(key, true);
                    report_key(&arrived, &verdict);
                }
            },
            Some(SplitChord::Sequence { introducer, params }) => {
                let KeyCode::Char(byte) = key.code else {
                    report_key("Esc", "the Esc key, on its own");
                    let (arrived, verdict) = describe_key(key, false);
                    report_key(&arrived, &verdict);
                    continue;
                };
                if byte.is_ascii_digit() || byte == ';' {
                    let mut params = params;
                    params.push(byte);
                    split = Some(SplitChord::Sequence { introducer, params });
                    continue;
                }
                match crate::app::meta_escape_key(introducer, byte, &params) {
                    Some(chord) => report_key(
                        &format!("{:?}{}", chord.code, modifier_suffix(chord.modifiers)),
                        "Alt, split across an Esc - GridBash rejoins these",
                    ),
                    None => report_key(
                        &format!("Esc then '{introducer}{params}{byte}'"),
                        "not a key sequence - passed through to the pane",
                    ),
                }
            }
        }
    }
}

/// How much of a split Alt chord has arrived so far.
enum SplitChord {
    /// A lone Esc that may yet turn out to be an Alt prefix.
    Escape,
    /// An Esc followed by a sequence introducer, and its parameters so far.
    Sequence { introducer: char, params: String },
}

/// Raw mode means this is the only thing moving the cursor, so every line has
/// to return it to column zero itself.
fn report_key(arrived: &str, verdict: &str) {
    print!("  {arrived:<28}  {verdict}\r\n");
    io::stdout().flush().ok();
}

/// What a key arrived as, and whether that is any use as a shortcut.
fn describe_key(key: KeyEvent, after_escape: bool) -> (String, String) {
    let arrived = format!("{:?}{}", key.code, modifier_suffix(key.modifiers));

    if key.modifiers.contains(KeyModifiers::ALT) {
        return (arrived, "Alt arrived — shortcuts work".into());
    }
    if after_escape {
        return (
            arrived,
            "Alt, split across an Esc — GridBash rejoins these".into(),
        );
    }
    if key.code == KeyCode::Esc && key.modifiers.is_empty() {
        return (arrived, "Esc, or the start of a split Alt chord".into());
    }
    if let KeyCode::Char(ch) = key.code
        && !ch.is_ascii()
    {
        return (
            arrived,
            format!("Option composed '{ch}' — no Alt, this types into a pane"),
        );
    }
    (arrived, "no Alt — this types into a pane".into())
}

fn modifier_suffix(modifiers: KeyModifiers) -> String {
    let mut held = Vec::new();
    for (flag, name) in [
        (KeyModifiers::CONTROL, "Ctrl"),
        (KeyModifiers::ALT, "Alt"),
        (KeyModifiers::SHIFT, "Shift"),
        (KeyModifiers::SUPER, "Super"),
    ] {
        if modifiers.contains(flag) {
            held.push(name);
        }
    }
    if held.is_empty() {
        String::new()
    } else {
        format!("  +{}", held.join("+"))
    }
}

/// Runs `gridbash doctor`.
///
/// `fix` applies the change without asking, for a shell that has no one sitting
/// in front of it. Interactively the change is always described and confirmed
/// first, because it edits an application's own preferences.
pub fn run(config: &Config, fix: bool, keys: bool) -> Result<()> {
    let report = Report::detect(config);
    if keys {
        let mut stdout = io::stdout();
        report.print(&mut stdout)?;
        stdout.flush()?;
        return probe_keys();
    }
    let mut stdout = io::stdout();
    report.print(&mut stdout)?;

    if !report.fixable() {
        stdout.flush()?;
        return Ok(());
    }
    let Some(action) = report.fix() else {
        stdout.flush()?;
        return Ok(());
    };

    writeln!(stdout)?;
    writeln!(stdout, "GridBash can {}.", action.describe())?;

    if !fix {
        if !io::stdin().is_terminal() {
            writeln!(stdout, "Run `gridbash doctor --fix` to apply it.")?;
            stdout.flush()?;
            return Ok(());
        }
        write!(stdout, "Apply it now? [y/N] ")?;
        stdout.flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            writeln!(stdout, "Left unchanged.")?;
            stdout.flush()?;
            return Ok(());
        }
    }

    action.apply()?;
    writeln!(stdout, "Applied. {}", action.restart_note())?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(contents: &str, key: &'static str, enabled: &'static [&'static str]) -> MetaState {
        let path = env::temp_dir().join(format!("gridbash-doctor-{key}-{}.conf", contents.len()));
        fs::write(&path, contents).expect("probe config");
        let state = probe_config_file(&ConfigFileProbe {
            path: path.clone(),
            line: "",
            key,
            enabled,
            default_sends: false,
        });
        let _ = fs::remove_file(&path);
        state
    }

    /// The same question is spelled differently by every terminal, so the value
    /// is whatever follows the key however it is separated.
    #[test]
    fn config_probes_read_each_terminals_spelling() {
        assert_eq!(
            probe(
                "macos-option-as-alt = true\n",
                "macos-option-as-alt",
                &["true"]
            ),
            MetaState::Sends
        );
        assert_eq!(
            probe("macos_option_as_alt yes\n", "macos_option_as_alt", &["yes"]),
            MetaState::Sends
        );
        assert_eq!(
            probe(
                "option_as_alt = \"Both\",\n",
                "option_as_alt",
                &["\"both\""]
            ),
            MetaState::Sends
        );
    }

    /// A commented-out setting is not a setting, and the last one written wins.
    #[test]
    fn config_probes_ignore_comments_and_take_the_last_setting() {
        assert_eq!(
            probe(
                "# macos-option-as-alt = true\n",
                "macos-option-as-alt",
                &["true"]
            ),
            MetaState::Composes
        );
        assert_eq!(
            probe(
                "macos-option-as-alt = true\nmacos-option-as-alt = false\n",
                "macos-option-as-alt",
                &["true"]
            ),
            MetaState::Composes
        );
    }

    /// A terminal that is missing the file entirely still gets an answer, and
    /// which answer depends on what it does untouched.
    #[test]
    fn a_missing_config_falls_back_to_the_terminals_own_default() {
        let missing = env::temp_dir().join("gridbash-doctor-absent.conf");
        let _ = fs::remove_file(&missing);
        for (default_sends, expected) in [(true, MetaState::Sends), (false, MetaState::Composes)] {
            assert_eq!(
                probe_config_file(&ConfigFileProbe {
                    path: missing.clone(),
                    line: "",
                    key: "whatever",
                    enabled: &["true"],
                    default_sends,
                }),
                expected
            );
        }
    }

    /// Option is only ever special on macOS.
    #[test]
    fn other_platforms_always_report_a_working_alt() {
        if !cfg!(target_os = "macos") {
            assert_eq!(meta_state(&HostTerminal::AppleTerminal), MetaState::Sends);
        }
    }

    /// Every terminal names a way to change the setting by hand, because the
    /// automatic path is not always available.
    #[test]
    fn every_terminal_can_be_fixed_by_hand() {
        for terminal in [
            HostTerminal::AppleTerminal,
            HostTerminal::ITerm2,
            HostTerminal::Ghostty,
            HostTerminal::Kitty,
            HostTerminal::WezTerm,
            HostTerminal::Alacritty,
            HostTerminal::VsCode,
            HostTerminal::Other("something".into()),
        ] {
            assert!(!terminal.manual_instructions().is_empty());
            assert!(!terminal.label().is_empty());
        }
    }
}
