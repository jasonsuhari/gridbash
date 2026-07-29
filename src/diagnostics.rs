//! On-disk crash reporting.
//!
//! GridBash unwinds on panic, so a panic in the UI thread exits with status 101
//! and prints to a terminal that is usually gone a moment later: no Windows
//! Error Reporting entry, no minidump, no record of what happened. These
//! reports are the only durable evidence after the process is gone.

use std::{
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

pub fn logs_dir() -> Option<PathBuf> {
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
        previous(info);
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
        panic_message(info),
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

fn report_preamble(role: &str) -> String {
    format!(
        "role: {role}\nversion: {}\npid: {}\ntimestamp_ms: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        now_millis(),
    )
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
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

fn prune_reports(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut reports = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    let Some(excess) = reports
        .len()
        .checked_sub(MAX_REPORTS)
        .filter(|count| *count > 0)
    else {
        return;
    };
    reports.sort_by_key(|(modified, _)| *modified);
    for (_, path) in reports.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
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

        let body = fs::read_to_string(&reports[0]).expect("read report");
        assert!(body.contains("kind: panic"), "report body: {body}");
        assert!(body.contains("role: smoketest"), "report body: {body}");
        assert!(
            body.contains("gridbash crash log smoke test"),
            "report body: {body}"
        );
        assert!(
            body.contains("src\\diagnostics.rs") || body.contains("src/diagnostics.rs"),
            "report must name the panicking file: {body}"
        );

        for report in reports {
            let _ = fs::remove_file(report);
        }
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
