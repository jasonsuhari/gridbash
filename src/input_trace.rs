use std::{
    fmt::Display,
    fs::{self, OpenOptions},
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::diagnostics;

const FILE_NAME: &str = "input-trace.txt";
const ROTATED_NAME: &str = "input-trace.1.txt";
const MAX_BYTES: u64 = 512 * 1024;

/// Callers pass the shape of an event, never the text a user typed.
pub fn record(event: impl Display) {
    let Some(directory) = diagnostics::logs_dir() else {
        return;
    };
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let path = directory.join(FILE_NAME);
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let _ = fs::rename(&path, directory.join(ROTATED_NAME));
    }
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or_default();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{millis} pid={} {event}", std::process::id());
    }
}
