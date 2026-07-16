use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

const DISCOVERY_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryRecord {
    pub version: u16,
    pub id: String,
    pub endpoint: String,
    pub pid: u32,
    pub started_at: u64,
}

impl DiscoveryRecord {
    pub fn new(id: String, endpoint: String) -> Self {
        Self {
            version: DISCOVERY_VERSION,
            id,
            endpoint,
            pid: std::process::id(),
            started_at: now_seconds(),
        }
    }
}

#[derive(Debug)]
pub struct DiscoveryLease {
    path: PathBuf,
}

impl DiscoveryLease {
    pub fn publish(record: &DiscoveryRecord) -> Result<Self> {
        let directory = discovery_directory()?;
        publish_in(&directory, record)
    }
}

impl Drop for DiscoveryLease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn discover_sessions(
    mut probe: impl FnMut(&DiscoveryRecord) -> bool,
) -> Result<Vec<DiscoveryRecord>> {
    discover_in(&discovery_directory()?, &mut probe)
}

fn discovery_directory() -> Result<PathBuf> {
    ProjectDirs::from("", "", "GridBash")
        .map(|dirs| dirs.data_local_dir().join("control"))
        .ok_or_else(|| anyhow!("failed to resolve GridBash control discovery directory"))
}

fn publish_in(directory: &Path, record: &DiscoveryRecord) -> Result<DiscoveryLease> {
    fs::create_dir_all(directory).with_context(|| {
        format!(
            "failed to create control discovery directory {}",
            directory.display()
        )
    })?;
    secure_directory(directory)?;

    let path = directory.join(format!("{}.json", record.id));
    let temporary = directory.join(format!(".{}.{}.tmp", record.id, std::process::id()));
    let bytes =
        serde_json::to_vec_pretty(record).context("failed to serialize control discovery")?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    secure_file_options(&mut options);
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("failed to create discovery file {}", temporary.display()))?;
    file.write_all(&bytes)
        .with_context(|| format!("failed to write discovery file {}", temporary.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush discovery file {}", temporary.display()))?;
    drop(file);
    fs::rename(&temporary, &path).with_context(|| {
        format!(
            "failed to publish control discovery file {}",
            path.display()
        )
    })?;
    Ok(DiscoveryLease { path })
}

fn discover_in(
    directory: &Path,
    probe: &mut impl FnMut(&DiscoveryRecord) -> bool,
) -> Result<Vec<DiscoveryRecord>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to read discovery directory {}", directory.display()))?
    {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let record = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<DiscoveryRecord>(&raw).ok());
        match record {
            Some(record) if record.version == DISCOVERY_VERSION && probe(&record) => {
                sessions.push(record);
            }
            _ => {
                let _ = fs::remove_file(path);
            }
        }
    }
    sessions.sort_by(|left, right| {
        right
            .started_at
            .cmp(&left.started_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(sessions)
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(unix)]
fn secure_directory(directory: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).with_context(|| {
        format!(
            "failed to secure discovery directory {}",
            directory.display()
        )
    })
}

#[cfg(not(unix))]
fn secure_directory(_directory: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_file_options(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
}

#[cfg(not(unix))]
fn secure_file_options(_options: &mut OpenOptions) {}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "gridbash-discovery-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("test clock")
                    .as_nanos()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovery_prunes_stale_and_invalid_records() {
        let directory = TestDirectory::new();
        let live = DiscoveryRecord::new("live".into(), "127.0.0.1:1".into());
        let stale = DiscoveryRecord::new("stale".into(), "127.0.0.1:2".into());
        let live_lease = publish_in(&directory.0, &live).expect("publish live");
        let stale_lease = publish_in(&directory.0, &stale).expect("publish stale");
        fs::write(directory.0.join("invalid.json"), "not json").expect("invalid fixture");

        let sessions = discover_in(&directory.0, &mut |record| record.id == "live")
            .expect("discover sessions");

        assert_eq!(sessions, [live]);
        assert!(live_lease.path.exists());
        assert!(!stale_lease.path.exists());
        assert!(!directory.0.join("invalid.json").exists());
    }

    #[test]
    fn published_metadata_contains_no_bearer_token_field() {
        let directory = TestDirectory::new();
        let record = DiscoveryRecord::new("session".into(), "127.0.0.1:4321".into());
        let lease = publish_in(&directory.0, &record).expect("publish");
        let raw = fs::read_to_string(&lease.path).expect("read discovery");

        assert!(!raw.contains("token"));
        assert_eq!(
            serde_json::from_str::<DiscoveryRecord>(&raw).unwrap(),
            record
        );
    }
}
