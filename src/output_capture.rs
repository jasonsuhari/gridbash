use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;

use crate::layout::PaneId;

const OUTPUT_FILE_ATTEMPTS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneLogKey {
    pub pane: PaneId,
    pub generation: u64,
}

impl PaneLogKey {
    pub fn new(pane: PaneId, generation: u64) -> Self {
        Self { pane, generation }
    }
}

pub fn default_output_directory() -> Result<PathBuf> {
    ProjectDirs::from("", "", "GridBash")
        .map(|dirs| dirs.data_local_dir().join("output"))
        .ok_or_else(|| anyhow!("failed to resolve GridBash output directory"))
}

pub fn prepare_output_directory(directory: Option<&Path>) -> Result<PathBuf> {
    let directory = match directory {
        Some(path) if path.as_os_str().is_empty() => bail!("output directory cannot be empty"),
        Some(path) => path.to_path_buf(),
        None => default_output_directory()?,
    };

    if directory.exists() && !directory.is_dir() {
        bail!("output path is not a directory: {}", directory.display());
    }
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create output directory {}", directory.display()))?;
    Ok(directory)
}

pub fn capture_output(directory: &Path, pane_number: usize, plain_text: &str) -> Result<PathBuf> {
    let (path, mut file) = create_output_file(directory, "capture", pane_number, "txt")?;
    file.write_all(plain_text.as_bytes())
        .with_context(|| format!("failed to write capture {}", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush capture {}", path.display()))?;
    Ok(path)
}

struct PaneLogger {
    path: PathBuf,
    writer: Box<dyn Write + Send>,
}

impl std::fmt::Debug for PaneLogger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PaneLogger")
            .field("path", &self.path)
            .finish()
    }
}

impl PaneLogger {
    fn start(directory: &Path, pane_number: usize) -> Result<Self> {
        let (path, file) = create_output_file(directory, "log", pane_number, "log")?;
        Ok(Self {
            path,
            writer: Box::new(BufWriter::new(file)),
        })
    }

    #[cfg(test)]
    fn with_writer(path: PathBuf, writer: impl Write + Send + 'static) -> Self {
        Self {
            path,
            writer: Box::new(writer),
        }
    }

    fn append(&mut self, plain_text: &str) -> Result<()> {
        if plain_text.is_empty() {
            return Ok(());
        }
        self.writer
            .write_all(plain_text.as_bytes())
            .with_context(|| format!("failed to append log {}", self.path.display()))?;
        self.writer
            .flush()
            .with_context(|| format!("failed to flush log {}", self.path.display()))
    }

    fn finish(&mut self) -> Result<()> {
        self.writer
            .flush()
            .with_context(|| format!("failed to flush log {}", self.path.display()))
    }
}

#[derive(Debug, Default)]
pub struct OutputLogs {
    active: HashMap<PaneLogKey, PaneLogger>,
}

impl OutputLogs {
    pub fn start(
        &mut self,
        key: PaneLogKey,
        pane_number: usize,
        directory: &Path,
    ) -> Result<PathBuf> {
        if let Some(logger) = self.active.get(&key) {
            bail!(
                "pane {pane_number} is already logging to {}",
                logger.path.display()
            );
        }
        let logger = PaneLogger::start(directory, pane_number)?;
        let path = logger.path.clone();
        self.active.insert(key, logger);
        Ok(path)
    }

    pub fn stop(&mut self, key: PaneLogKey) -> Result<Option<PathBuf>> {
        let Some(mut logger) = self.active.remove(&key) else {
            return Ok(None);
        };
        let path = logger.path.clone();
        logger.finish()?;
        Ok(Some(path))
    }

    pub fn append(&mut self, key: PaneLogKey, plain_text: &str) -> Result<()> {
        let result = match self.active.get_mut(&key) {
            Some(logger) => logger.append(plain_text),
            None => return Ok(()),
        };
        if result.is_err() {
            self.active.remove(&key);
        }
        result
    }

    pub fn is_active(&self, key: PaneLogKey) -> bool {
        self.active.contains_key(&key)
    }

    pub fn path(&self, key: PaneLogKey) -> Option<&Path> {
        self.active.get(&key).map(|logger| logger.path.as_path())
    }

    #[cfg(test)]
    fn insert_logger(&mut self, key: PaneLogKey, logger: PaneLogger) {
        self.active.insert(key, logger);
    }
}

fn create_output_file(
    directory: &Path,
    kind: &str,
    pane_number: usize,
    extension: &str,
) -> Result<(PathBuf, File)> {
    let directory = prepare_output_directory(Some(directory))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();

    for attempt in 0..OUTPUT_FILE_ATTEMPTS {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let path = directory.join(format!(
            "gridbash-{kind}-{stamp}-pane-{pane_number}{suffix}.{extension}"
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create output file {}", path.display()));
            }
        }
    }

    bail!(
        "failed to allocate a unique {kind} filename in {}",
        directory.display()
    )
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = format!(
                "gridbash-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("test clock")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Clone)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("writer lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("disk unavailable"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn key(generation: u64) -> PaneLogKey {
        PaneLogKey::new(PaneId(7), generation)
    }

    #[test]
    fn output_directory_validation_rejects_empty_and_file_paths() {
        let directory = TestDirectory::new("path-validation");
        let file = directory.path().join("not-a-directory");
        fs::write(&file, "content").expect("write fixture");

        assert!(prepare_output_directory(Some(Path::new(""))).is_err());
        assert!(prepare_output_directory(Some(&file)).is_err());
        assert_eq!(
            prepare_output_directory(Some(directory.path())).expect("valid directory"),
            directory.path()
        );
    }

    #[test]
    fn captures_use_collision_safe_paths() {
        let directory = TestDirectory::new("capture-paths");
        let first = capture_output(directory.path(), 1, "first").expect("first capture");
        let second = capture_output(directory.path(), 1, "second").expect("second capture");

        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(first).expect("first text"), "first");
        assert_eq!(fs::read_to_string(second).expect("second text"), "second");
    }

    #[test]
    fn logger_appends_output_in_order() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = SharedWriter(bytes.clone());
        let mut logger = PaneLogger::with_writer(PathBuf::from("ordered.log"), writer);

        logger.append("first\n").expect("first append");
        logger.append("second\n").expect("second append");

        assert_eq!(
            String::from_utf8(bytes.lock().expect("bytes lock").clone()).expect("utf8"),
            "first\nsecond\n"
        );
    }

    #[test]
    fn logging_can_stop_and_restart_with_a_new_file() {
        let directory = TestDirectory::new("restart");
        let mut logs = OutputLogs::default();
        let first = logs.start(key(1), 1, directory.path()).expect("start");
        assert!(logs.is_active(key(1)));
        assert_eq!(logs.stop(key(1)).expect("stop"), Some(first.clone()));
        assert!(!logs.is_active(key(1)));

        let second = logs.start(key(1), 1, directory.path()).expect("restart");
        assert_ne!(first, second);
        assert!(logs.is_active(key(1)));
    }

    #[test]
    fn write_failure_removes_only_the_failed_logger() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let mut logs = OutputLogs::default();
        logs.insert_logger(
            key(1),
            PaneLogger::with_writer(PathBuf::from("failed.log"), FailingWriter),
        );
        logs.insert_logger(
            key(2),
            PaneLogger::with_writer(PathBuf::from("healthy.log"), SharedWriter(bytes.clone())),
        );

        assert!(logs.append(key(1), "lost").is_err());
        assert!(!logs.is_active(key(1)));
        assert!(logs.append(key(2), "kept").is_ok());
        assert!(logs.is_active(key(2)));
        assert_eq!(&*bytes.lock().expect("bytes lock"), b"kept");
    }
}
