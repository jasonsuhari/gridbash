//! Adopting a terminal the user already has open into a pane.
//!
//! A running console cannot be moved into GridBash. Windows binds a process to
//! the console it was created with, and there is no supported way to hand a live
//! process to a different pseudoconsole — so the window on screen keeps its
//! process no matter what we do here.
//!
//! What can be carried across is *where* it was: a shell's window title says its
//! working directory, and that is the part people actually mean when they say
//! they want a window "in" the grid. So a pane is re-rooted at the same folder
//! and the original window is left alone, running, for the user to close when
//! they are ready.

use std::path::{Path, PathBuf};

/// A terminal window running outside this GridBash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTerminal {
    pub pid: u32,
    /// The window title, as shown in the taskbar.
    pub title: String,
    /// The folder the title reports, when it reports one that still exists.
    pub cwd: Option<PathBuf>,
}

impl ExternalTerminal {
    /// What the picker shows: the folder if we could read one, else the raw
    /// title, so a window we only half-understand is still listed rather than
    /// silently dropped.
    pub fn label(&self) -> String {
        match self.cwd.as_deref() {
            Some(cwd) => cwd.display().to_string(),
            None => self.title.clone(),
        }
    }

    /// Only a window whose folder we resolved can re-root a pane.
    pub fn is_adoptable(&self) -> bool {
        self.cwd.is_some()
    }
}

/// Window titles that mark a shell we know how to read a path out of.
const SHELL_TITLE_PREFIXES: [&str; 5] = ["MINGW64:", "MINGW32:", "MSYS:", "MSYS2:", "CYGWIN:"];

/// The working directory a shell window's title is reporting.
///
/// Git Bash titles its window `MINGW64:/c/Users/you/project`, which is a POSIX
/// path from an MSYS root rather than anything Windows can open, so the drive
/// letter has to be put back. Titles that carry no path at all — a shell that
/// has been retitled by a running program, say — resolve to nothing rather than
/// to a guess.
pub fn cwd_from_title(title: &str, home: Option<&Path>) -> Option<PathBuf> {
    let trimmed = title.trim();
    let body = SHELL_TITLE_PREFIXES
        .iter()
        .find_map(|prefix| {
            trimmed
                .len()
                .checked_sub(prefix.len())
                .and_then(|_| trimmed.get(..prefix.len()))
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .and_then(|_| trimmed.get(prefix.len()..))
        })
        .unwrap_or(trimmed)
        .trim();

    // A retitled window reads like "npm run dev" or "vim - src/main.rs". The
    // path, when there is one, is the first thing on the line.
    let candidate = body.split_whitespace().next()?;
    if candidate.is_empty() {
        return None;
    }

    posix_to_windows(candidate, home)
}

/// Turns the shell's idea of a path into one the OS can open.
fn posix_to_windows(value: &str, home: Option<&Path>) -> Option<PathBuf> {
    // Already a Windows path, from a shell that reports them that way.
    if value.len() >= 2 && value.as_bytes()[1] == b':' {
        return Some(PathBuf::from(value.replace('/', "\\")));
    }

    if let Some(rest) = value.strip_prefix('~') {
        let home = home?;
        let rest = rest.trim_start_matches(['/', '\\']);
        return Some(if rest.is_empty() {
            home.to_path_buf()
        } else {
            home.join(rest.replace('/', "\\"))
        });
    }

    // Cygwin spells the same thing with a prefix of its own.
    let value = value.strip_prefix("/cygdrive").unwrap_or(value);

    // `/c/Users/you` — a drive letter standing in for its root.
    let rest = value.strip_prefix('/')?;
    let mut parts = rest.splitn(2, '/');
    let drive = parts.next()?;
    let mut letter = drive.chars();
    let letter = letter.next()?;
    if !letter.is_ascii_alphabetic() || drive.chars().count() != 1 {
        return None;
    }

    // Built as text rather than with `PathBuf::push`, which appends a separator
    // of the *host's* kind: a non-Windows host would turn `C:\` plus `src` into
    // `C:\/src`. These are Windows paths whatever machine parses them, and the
    // tests below run on every platform GridBash ships to.
    let mut path = format!("{}:\\", letter.to_ascii_uppercase());
    if let Some(tail) = parts.next().filter(|tail| !tail.is_empty()) {
        path.push_str(&tail.replace('/', "\\"));
    }
    Some(PathBuf::from(path))
}

/// Keeps only windows whose folder still exists, so the picker never offers a
/// path that would fail the moment it was chosen.
fn resolve(pid: u32, title: String) -> ExternalTerminal {
    let cwd = cwd_from_title(&title, home_dir().as_deref()).filter(|path| path.is_dir());
    ExternalTerminal { pid, title, cwd }
}

fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

/// Every shell window on this desktop that does not belong to us.
///
/// Returns nothing off Windows: the title convention this reads is an MSYS one,
/// and elsewhere a terminal multiplexer is the right tool for the job anyway.
#[cfg(windows)]
pub fn discover() -> Vec<ExternalTerminal> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::{
        Win32::{
            Foundation::{HWND, LPARAM, TRUE},
            UI::WindowsAndMessaging::{
                EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
                IsWindowVisible,
            },
        },
        core::BOOL,
    };

    unsafe extern "system" fn collect(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: the pointer is the one handed to `EnumWindows` below, which
        // outlives the enumeration it drives.
        let found = unsafe { &mut *(lparam as *mut Vec<(u32, String)>) };
        if unsafe { IsWindowVisible(window) } == 0 {
            return TRUE;
        }

        let length = unsafe { GetWindowTextLengthW(window) };
        if length <= 0 {
            return TRUE;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
        if written <= 0 {
            return TRUE;
        }
        let title = std::ffi::OsString::from_wide(&buffer[..written as usize])
            .to_string_lossy()
            .into_owned();

        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid != 0 {
            found.push((pid, title));
        }
        TRUE
    }

    let mut found: Vec<(u32, String)> = Vec::new();
    // SAFETY: `collect` only writes through the pointer we pass, and the vector
    // it points at is alive for the whole call.
    unsafe {
        EnumWindows(Some(collect), &mut found as *mut _ as LPARAM);
    }

    let own = std::process::id();
    let mut terminals = found
        .into_iter()
        .filter(|(pid, _)| *pid != own)
        .filter(|(_, title)| looks_like_a_shell(title))
        .map(|(pid, title)| resolve(pid, title))
        .collect::<Vec<_>>();
    terminals.sort_by_key(ExternalTerminal::label);
    terminals.dedup_by(|left, right| left.pid == right.pid);
    terminals
}

#[cfg(not(windows))]
pub fn discover() -> Vec<ExternalTerminal> {
    Vec::new()
}

/// Whether a window title is one of the shells this can read a folder out of.
///
/// Deliberately narrow. Listing every console window on the desktop would put
/// the user's editor and chat client in a picker that promises to open a shell
/// where they are.
fn looks_like_a_shell(title: &str) -> bool {
    let trimmed = title.trim();
    // `get` rather than a slice: window titles are arbitrary user text, and
    // slicing one whose first character is multi-byte panics — which on this
    // path would take the whole interface down mid-frame.
    SHELL_TITLE_PREFIXES.iter().any(|prefix| {
        trimmed
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_folder_out_of_a_git_bash_title() {
        let home = PathBuf::from("C:\\Users\\Jason");
        let cwd = |title: &str| cwd_from_title(title, Some(&home));

        assert_eq!(
            cwd("MINGW64:/c/Users/Jason/Documents/GitHub"),
            Some(PathBuf::from("C:\\Users\\Jason\\Documents\\GitHub"))
        );
        // The prefix is optional, and the case of it is not ours to rely on.
        assert_eq!(cwd("/d/work"), Some(PathBuf::from("D:\\work")));
        assert_eq!(cwd("mingw32:/e/"), Some(PathBuf::from("E:\\")));
        assert_eq!(cwd("MSYS:/c/tmp"), Some(PathBuf::from("C:\\tmp")));
        assert_eq!(
            cwd("CYGWIN:/cygdrive/f/data"),
            Some(PathBuf::from("F:\\data"))
        );

        // Home-relative titles.
        assert_eq!(cwd("MINGW64:~"), Some(home.clone()));
        assert_eq!(cwd("~/projects"), Some(home.join("projects")));
        assert_eq!(cwd_from_title("~", None), None, "no home means no answer");

        // A shell that already speaks Windows.
        assert_eq!(
            cwd("C:/Users/Jason/src"),
            Some(PathBuf::from("C:\\Users\\Jason\\src"))
        );
    }

    /// A window retitled by whatever is running in it must resolve to nothing
    /// rather than to a plausible-looking wrong folder.
    #[test]
    fn a_title_without_a_path_resolves_to_nothing() {
        let home = PathBuf::from("C:\\Users\\Jason");
        let cwd = |title: &str| cwd_from_title(title, Some(&home));

        assert_eq!(cwd(""), None);
        assert_eq!(cwd("   "), None);
        assert_eq!(cwd("MINGW64:"), None);
        assert_eq!(cwd("npm run dev"), None);
        assert_eq!(cwd("Inbox - Outlook"), None);
        // A drive letter is one character; "/usr/bin" is not a Windows path.
        assert_eq!(cwd("/usr/bin"), None);
        assert_eq!(cwd("MINGW64:/usr/local"), None);
    }

    #[test]
    fn only_shell_windows_are_offered() {
        assert!(looks_like_a_shell("MINGW64:/c/src"));
        assert!(looks_like_a_shell("  mingw64:/c/src"));
        assert!(looks_like_a_shell("CYGWIN:~"));
        assert!(!looks_like_a_shell("Slack | general"));
        assert!(!looks_like_a_shell("Command Prompt"));
        assert!(!looks_like_a_shell(""));

        // Window titles are arbitrary user text. Slicing one whose first
        // character is multi-byte used to panic here, and this runs over every
        // window on the desktop.
        for hostile in ["·", "日本語のタイトル", "→", "M·NGW64:/c/x", "🙂 build"] {
            assert!(!looks_like_a_shell(hostile), "{hostile}");
        }
    }

    #[test]
    fn a_window_without_a_readable_folder_still_names_itself() {
        let unreadable = ExternalTerminal {
            pid: 42,
            title: "MINGW64:~ - npm run dev".into(),
            cwd: None,
        };
        assert_eq!(unreadable.label(), "MINGW64:~ - npm run dev");
        assert!(!unreadable.is_adoptable());

        let readable = ExternalTerminal {
            pid: 43,
            title: "MINGW64:/c/src".into(),
            cwd: Some(PathBuf::from("C:\\src")),
        };
        assert_eq!(readable.label(), "C:\\src");
        assert!(readable.is_adoptable());
    }

    /// Discovery must be safe to call anywhere, including in a test process with
    /// no desktop to enumerate.
    #[test]
    fn discovery_never_panics() {
        let _ = discover();
    }

    /// Prints what is actually on this desktop. Run with:
    /// `cargo test -- --ignored --nocapture lists_the_shell_windows`
    #[test]
    #[ignore = "inspects the live desktop"]
    fn lists_the_shell_windows_on_this_desktop() {
        for terminal in discover() {
            eprintln!(
                "pid {:>6}  adoptable={}  {}  <- {:?}",
                terminal.pid,
                terminal.is_adoptable(),
                terminal.label(),
                terminal.title
            );
        }
    }
}
