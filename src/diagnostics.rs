//! On-disk crash reporting.
//!
//! GridBash unwinds on panic, so a panic in the UI thread exits with status 101
//! and prints to a terminal that is usually gone a moment later: no Windows
//! Error Reporting entry, no minidump, no record of what happened. These
//! reports are the only durable evidence after the process is gone.

use std::{
    cell::Cell,
    fs::{self, OpenOptions},
    io::Write,
    panic::PanicHookInfo,
    path::{Path, PathBuf},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;

/// Keep enough reports to cover a session's worth of restarts without letting
/// the directory grow without bound.
const MAX_REPORTS: usize = 50;

thread_local! {
    /// Set while this thread is about to catch and recover from a panic.
    ///
    /// A shielded panic still gets a report on disk, but nothing may print to
    /// the terminal or tear down the alternate screen: the TUI keeps running,
    /// and stderr output would land on top of it as garbage. It is per-thread
    /// because a panic hook runs on the thread that panicked, and a worker
    /// recovering from its own panic must not change what a UI-thread panic
    /// does.
    static SHIELDED: Cell<bool> = const { Cell::new(false) };
}

/// True while a panic on this thread is expected to be caught and recovered
/// rather than fatal.
pub fn panics_are_shielded() -> bool {
    SHIELDED
        .try_with(|shielded| shielded.get())
        .unwrap_or(false)
}

/// Suppresses terminal-visible panic output on this thread for as long as the
/// guard lives.
///
/// The guard drops during unwinding as well as on the happy path, so the shield
/// always lifts before the caught panic is handled.
pub struct PanicShield {
    previous: bool,
}

impl PanicShield {
    pub fn new() -> Self {
        let previous = SHIELDED
            .try_with(|shielded| shielded.replace(true))
            .unwrap_or(false);
        Self { previous }
    }
}

impl Drop for PanicShield {
    fn drop(&mut self) {
        let previous = self.previous;
        let _ = SHIELDED.try_with(|shielded| shielded.set(previous));
    }
}

/// Runs `work`, turning a panic into the error the work would have reported.
///
/// Worker threads answer the interface over a channel. A panicking worker never
/// answers at all, so whatever it was asked to do stays in flight for the rest
/// of the session, spinner and all. Reporting the panic as a failure keeps the
/// request completable and puts the reason on screen.
pub fn recovering<T>(label: &str, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let outcome = {
        let _shield = PanicShield::new();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
    };
    match outcome {
        Ok(result) => result,
        Err(payload) => {
            let detail = panic_payload_message(payload.as_ref());
            record_recovered("tui", label, &detail);
            Err(format!("{label} failed unexpectedly: {detail}"))
        }
    }
}

/// Directory reports are written to.
///
/// `GRIDBASH_LOGS_DIR` overrides it. A test binary redirects itself, because a
/// test that writes a report into the real directory does not just leave litter
/// there: reports are pruned to a fixed count, so a test run repeated often
/// enough evicts the user's genuine crash evidence before anyone reads it.
pub fn logs_dir() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("GRIDBASH_LOGS_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Some(configured);
    }
    if cfg!(test) {
        return Some(std::env::temp_dir().join("gridbash-test-logs"));
    }
    ProjectDirs::from("", "", "GridBash").map(|dirs| dirs.data_local_dir().join("logs"))
}

/// Record panics to disk before delegating to the previously installed hook.
///
/// The UI thread's terminal-restoring hook chains to whatever hook it replaced,
/// so installing this first means both the plain CLI paths and the full TUI
/// report panics without either needing to know about the other.
pub fn install_panic_logger(role: &'static str) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record_panic(info, role);
        // The default hook writes the message and backtrace to stderr. That is
        // what a plain CLI run wants, and exactly what a recovering TUI must
        // not have painted over its alternate screen.
        if !panics_are_shielded() {
            previous(info);
        }
    }));
}

/// Write a panic report. Called from inside a panic hook, so every failure is
/// swallowed: a second panic here would abort the process and lose the report.
pub fn record_panic(info: &PanicHookInfo<'_>, role: &str) {
    let location = info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| "unknown".to_string());
    let thread = thread::current();
    let report = format!(
        "kind: panic\n\
         {}\
         thread: {}\n\
         location: {location}\n\
         message: {}\n\
         backtrace:\n{}\n",
        report_preamble(role),
        thread.name().unwrap_or("unnamed"),
        panic_payload_message(info.payload()),
        std::backtrace::Backtrace::force_capture(),
    );
    write_report("panic", role, &report);
}

/// Record a non-zero exit so a failed startup or teardown leaves a trail too.
pub fn record_error_exit(role: &str, error: &anyhow::Error) {
    let report = format!(
        "kind: error-exit\n{}error: {error:#}\n",
        report_preamble(role)
    );
    write_report("error", role, &report);
}

/// Record a failure the process survived.
///
/// Recovered failures never reach the user's terminal, so without this they
/// would leave no trace at all beyond a status-bar line that scrolls away.
pub fn record_recovered(role: &str, context: &str, detail: &str) {
    let report = format!(
        "kind: recovered\n{}context: {context}\ndetail: {detail}\n",
        report_preamble(role)
    );
    write_report("recovered", role, &report);
}

fn report_preamble(role: &str) -> String {
    format!(
        "role: {role}\nversion: {}\npid: {}\ntimestamp_ms: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        now_millis(),
    )
}

/// Best-effort text of a panic payload, for reports and status messages.
///
/// A panic payload is `Any`, so anything other than the two shapes `panic!`
/// produces has to be described rather than read.
pub fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or_default()
}

fn write_report(kind: &str, role: &str, report: &str) {
    let Some(directory) = logs_dir() else {
        return;
    };
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let path = directory.join(format!(
        "{kind}-{role}-{}-{}.log",
        now_millis(),
        std::process::id()
    ));
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(report.as_bytes());
        let _ = file.flush();
    }
    prune_reports(&directory);
}

/// Drop the oldest reports once the directory is full, taking everything else
/// before a panic report.
///
/// A panic is the report that cannot be reconstructed: the process is gone and
/// nothing else recorded why. Recovered failures and error exits are far more
/// numerous and far less valuable, so a burst of them must not push the one
/// panic worth reading out of the directory.
fn prune_reports(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut reports = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((is_panic_report(&entry.path()), modified, entry.path()))
        })
        .collect::<Vec<_>>();
    let Some(excess) = reports
        .len()
        .checked_sub(MAX_REPORTS)
        .filter(|count| *count > 0)
    else {
        return;
    };
    reports.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    for (_, _, path) in reports.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
}

fn is_panic_report(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("panic-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_reports_name_the_role_and_location() {
        let report = format!(
            "kind: panic\n{}location: src/ui.rs:1:2\n",
            report_preamble("tui")
        );
        assert!(report.contains("role: tui"));
        assert!(report.contains(concat!("version: ", env!("CARGO_PKG_VERSION"))));
        assert!(report.contains("location: src/ui.rs:1:2"));
    }

    /// End-to-end: a real panic must leave a readable report behind, because
    /// this is the only evidence that survives the process.
    #[test]
    fn a_real_panic_writes_a_report_naming_the_message_and_location() {
        let directory = logs_dir().expect("log directory");
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|info| record_panic(info, "smoketest")));
        let result = std::panic::catch_unwind(|| panic!("gridbash crash log smoke test"));
        std::panic::set_hook(previous);
        assert!(result.is_err(), "the test panic must have unwound");

        let reports = fs::read_dir(&directory)
            .expect("read log directory")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("panic-smoketest-"))
            })
            .collect::<Vec<_>>();
        assert!(!reports.is_empty(), "panic must produce a report");

        // The panic hook is process-wide, so a panic from any other test running
        // at the same time also lands under this role. Match on the message this
        // test panicked with instead of assuming which report came first.
        let body = reports
            .iter()
            .filter_map(|report| fs::read_to_string(report).ok())
            .find(|body| body.contains("gridbash crash log smoke test"))
            .unwrap_or_else(|| {
                panic!(
                    "no report names the smoke-test panic; found {} report(s)",
                    reports.len()
                )
            });
        assert!(body.contains("kind: panic"), "report body: {body}");
        assert!(body.contains("role: smoketest"), "report body: {body}");
        assert!(
            body.contains("src\\diagnostics.rs") || body.contains("src/diagnostics.rs"),
            "report must name the panicking file: {body}"
        );

        for report in reports {
            let _ = fs::remove_file(report);
        }
    }

    /// The shield decides whether a panic tears down the alternate screen and
    /// prints to stderr. Leaving it stuck on would hide a genuinely fatal panic;
    /// leaving it stuck off would let a recovered one garble the interface.
    #[test]
    fn the_panic_shield_lifts_on_both_the_happy_path_and_an_unwind() {
        assert!(!panics_are_shielded());

        {
            let _outer = PanicShield::new();
            assert!(panics_are_shielded());
            {
                let _inner = PanicShield::new();
                assert!(panics_are_shielded());
            }
            assert!(
                panics_are_shielded(),
                "a nested guard must not lift the outer shield"
            );
        }
        assert!(!panics_are_shielded());

        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(|| {
            let _shield = PanicShield::new();
            assert!(panics_are_shielded());
            panic!("unwind past the shield");
        });
        std::panic::set_hook(previous);

        assert!(result.is_err(), "the test panic must have unwound");
        assert!(
            !panics_are_shielded(),
            "unwinding out of the guard must lift the shield"
        );
    }

    /// A panic report is the only evidence that a crash happened at all, so a
    /// flood of routine reports must not be able to push one out.
    #[test]
    fn pruning_takes_routine_reports_before_panics() {
        let directory = std::env::temp_dir().join(format!("gridbash-kinds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create log directory");
        for index in 0..5 {
            fs::write(directory.join(format!("panic-tui-{index:04}.log")), b"x")
                .expect("write panic report");
        }
        for index in 0..MAX_REPORTS + 10 {
            fs::write(
                directory.join(format!("recovered-tui-{index:04}.log")),
                b"x",
            )
            .expect("write recovered report");
        }

        prune_reports(&directory);

        let remaining = fs::read_dir(&directory)
            .expect("read log directory")
            .flatten()
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.starts_with("panic-"))
            })
            .collect::<Vec<_>>();
        assert_eq!(remaining.len(), MAX_REPORTS);
        assert_eq!(
            remaining.iter().filter(|is_panic| **is_panic).count(),
            5,
            "every panic report must survive"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    /// A test binary that writes into the real diagnostics directory prunes the
    /// user's genuine crash reports out of it.
    #[test]
    fn a_test_binary_never_writes_to_the_real_diagnostics_directory() {
        let directory = logs_dir().expect("log directory");

        let real = ProjectDirs::from("", "", "GridBash")
            .map(|dirs| dirs.data_local_dir().join("logs"))
            .expect("real log directory");
        assert_ne!(
            directory, real,
            "tests must be redirected away from {real:?}"
        );
    }

    #[test]
    fn pruning_keeps_the_newest_reports() {
        let directory = std::env::temp_dir().join(format!("gridbash-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create log directory");
        for index in 0..MAX_REPORTS + 10 {
            fs::write(directory.join(format!("panic-tui-{index:04}.log")), b"x")
                .expect("write report");
        }

        prune_reports(&directory);

        let remaining = fs::read_dir(&directory)
            .expect("read log directory")
            .flatten()
            .count();
        assert_eq!(remaining, MAX_REPORTS);
        let _ = fs::remove_dir_all(&directory);
    }
}
