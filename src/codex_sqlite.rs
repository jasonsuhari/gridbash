use std::{
    collections::BTreeMap,
    env,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use fs4::{FileExt, TryLockError};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

const CODEX_HOME_ENV: &str = "CODEX_HOME";
const CODEX_SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";
const GRIDBASH_MANAGED_CODEX_SQLITE_ENV: &str = "GRIDBASH_MANAGED_CODEX_SQLITE_HOME";
const DEFAULT_CODEX_HOME_SCOPE: &str = "<default>";
const MAX_CODEX_SQLITE_LANES: usize = 4096;

pub(crate) struct CodexSqlitePool {
    root: PathBuf,
}

pub(crate) struct CodexSqliteLease {
    home: PathBuf,
    scope_id: String,
    _lock: File,
}

pub(crate) struct PaneCodexSqlite {
    pub(crate) env: Vec<(String, String)>,
    pub(crate) lease: Option<CodexSqliteLease>,
}

impl CodexSqlitePool {
    pub(crate) fn new() -> Result<Self> {
        let root = ProjectDirs::from("", "", "GridBash")
            .map(|dirs| dirs.data_local_dir().join("codex-sqlite"))
            .ok_or_else(|| anyhow!("failed to resolve GridBash data directory"))?;
        if !root.is_absolute() {
            bail!(
                "GridBash Codex SQLite directory is not absolute: {}",
                root.display()
            );
        }
        Ok(Self { root })
    }

    pub(crate) fn for_pane(
        &self,
        pane_env: &BTreeMap<String, String>,
        preferred: Option<CodexSqliteLease>,
    ) -> Result<PaneCodexSqlite> {
        self.for_pane_with_inherited(
            pane_env,
            env::var_os(CODEX_SQLITE_HOME_ENV).as_deref(),
            env::var_os(GRIDBASH_MANAGED_CODEX_SQLITE_ENV).as_deref(),
            env::var_os(CODEX_HOME_ENV).as_deref(),
            preferred,
        )
    }

    fn for_pane_with_inherited(
        &self,
        pane_env: &BTreeMap<String, String>,
        inherited_sqlite_home: Option<&OsStr>,
        inherited_managed_home: Option<&OsStr>,
        inherited_codex_home: Option<&OsStr>,
        preferred: Option<CodexSqliteLease>,
    ) -> Result<PaneCodexSqlite> {
        if has_explicit_sqlite_home(pane_env, inherited_sqlite_home, inherited_managed_home) {
            return Ok(PaneCodexSqlite {
                env: Vec::new(),
                lease: None,
            });
        }

        let scope = codex_home_scope(pane_env, inherited_codex_home);
        let scope_id = stable_scope_id(&scope);
        let lease = match preferred {
            Some(lease) if lease.scope_id == scope_id => lease,
            _ => self.acquire(&scope_id)?,
        };
        let home = lease.home.display().to_string();
        Ok(PaneCodexSqlite {
            env: vec![
                (CODEX_SQLITE_HOME_ENV.into(), home.clone()),
                (GRIDBASH_MANAGED_CODEX_SQLITE_ENV.into(), home),
            ],
            lease: Some(lease),
        })
    }

    fn acquire(&self, scope_id: &str) -> Result<CodexSqliteLease> {
        let scope_root = self.root.join(scope_id);
        create_private_dir_all(&scope_root).with_context(|| {
            format!(
                "failed to create Codex SQLite pool {}",
                scope_root.display()
            )
        })?;

        for lane in 1..=MAX_CODEX_SQLITE_LANES {
            let home = scope_root.join(format!("lane-{lane}"));
            create_private_dir_all(&home).with_context(|| {
                format!("failed to create Codex SQLite lane {}", home.display())
            })?;
            let lock_path = home.join(".gridbash.lock");
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .with_context(|| {
                    format!(
                        "failed to open Codex SQLite lane lock {}",
                        lock_path.display()
                    )
                })?;

            match FileExt::try_lock(&lock) {
                Ok(()) => {
                    return Ok(CodexSqliteLease {
                        home,
                        scope_id: scope_id.into(),
                        _lock: lock,
                    });
                }
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| {
                        format!("failed to lock Codex SQLite lane {}", lock_path.display())
                    });
                }
            }
        }

        bail!(
            "all {MAX_CODEX_SQLITE_LANES} Codex SQLite lanes are currently in use under {}",
            scope_root.display()
        )
    }
}

fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)?;

    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;

    Ok(())
}

fn has_explicit_sqlite_home(
    pane_env: &BTreeMap<String, String>,
    inherited_home: Option<&OsStr>,
    inherited_managed_home: Option<&OsStr>,
) -> bool {
    if let Some(value) = pane_env.get(CODEX_SQLITE_HOME_ENV) {
        return !value.trim().is_empty();
    }

    let inherited_is_managed = inherited_home.is_some() && inherited_home == inherited_managed_home;
    inherited_home.is_some_and(non_empty_os_str) && !inherited_is_managed
}

fn codex_home_scope(
    pane_env: &BTreeMap<String, String>,
    inherited_codex_home: Option<&OsStr>,
) -> String {
    if let Some(value) = pane_env.get(CODEX_HOME_ENV) {
        return non_empty(value)
            .unwrap_or(DEFAULT_CODEX_HOME_SCOPE)
            .to_string();
    }

    inherited_codex_home
        .filter(|value| non_empty_os_str(value))
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| DEFAULT_CODEX_HOME_SCOPE.into())
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then_some(value)
}

fn non_empty_os_str(value: &OsStr) -> bool {
    !value.to_string_lossy().trim().is_empty()
}

fn stable_scope_id(value: &str) -> String {
    fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
        bytes.iter().fold(seed, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    let bytes = value.as_bytes();
    let first = fnv1a(bytes, 0xcbf2_9ce4_8422_2325);
    let second = fnv1a(bytes, 0x8422_2325_cbf2_9ce4);
    format!("{first:016x}{second:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let mut random = [0_u8; 8];
            getrandom::fill(&mut random).expect("random test root");
            let suffix = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let root = env::temp_dir().join(format!(
                "gridbash-codex-sqlite-test-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("create test root");
            Self(root)
        }

        fn pool(&self) -> CodexSqlitePool {
            CodexSqlitePool {
                root: self.0.clone(),
            }
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sqlite_home(pane: &PaneCodexSqlite) -> &Path {
        Path::new(
            &pane
                .env
                .iter()
                .find(|(name, _)| name == CODEX_SQLITE_HOME_ENV)
                .expect("managed SQLite home")
                .1,
        )
    }

    #[test]
    fn leases_unique_existing_lanes_and_reuses_released_lane() {
        let root = TestRoot::new();
        let first_pool = root.pool();
        let second_pool = root.pool();
        let pane_env = BTreeMap::new();

        let first = first_pool
            .for_pane_with_inherited(&pane_env, None, None, None, None)
            .expect("first lease");
        let first_home = sqlite_home(&first).to_path_buf();
        let second = second_pool
            .for_pane_with_inherited(&pane_env, None, None, None, None)
            .expect("second lease");
        let second_home = sqlite_home(&second).to_path_buf();

        assert!(first_home.is_absolute());
        assert!(first_home.is_dir());
        assert!(second_home.is_dir());
        assert_ne!(first_home, second_home);
        assert_eq!(first.env[0].1, first.env[1].1);
        assert_eq!(second.env[0].1, second.env[1].1);
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&first_home)
                .expect("lane metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        drop(first);
        let mut recycled = second_pool
            .for_pane_with_inherited(&pane_env, None, None, None, None)
            .expect("recycled lease");
        assert_eq!(sqlite_home(&recycled), first_home);

        let preferred = recycled.lease.take();
        let replacement = second_pool
            .for_pane_with_inherited(&pane_env, None, None, None, preferred)
            .expect("preferred replacement lease");
        assert_eq!(sqlite_home(&replacement), first_home);
    }

    #[test]
    fn respects_user_overrides_and_replaces_blank_values() {
        let root = TestRoot::new();
        let pool = root.pool();
        let custom = OsStr::new("C:\\custom\\codex-sqlite");
        let other_marker = OsStr::new("C:\\gridbash\\managed-sqlite");

        let inherited = pool
            .for_pane_with_inherited(
                &BTreeMap::new(),
                Some(custom),
                Some(other_marker),
                None,
                None,
            )
            .expect("inherited override");
        assert!(inherited.env.is_empty());
        assert!(inherited.lease.is_none());

        let mut explicit = BTreeMap::new();
        explicit.insert(
            CODEX_SQLITE_HOME_ENV.into(),
            custom.to_string_lossy().into(),
        );
        let explicit = pool
            .for_pane_with_inherited(&explicit, None, None, None, None)
            .expect("pane override");
        assert!(explicit.env.is_empty());
        assert!(explicit.lease.is_none());

        let mut blank = BTreeMap::new();
        blank.insert(CODEX_SQLITE_HOME_ENV.into(), "   ".into());
        let blank = pool
            .for_pane_with_inherited(&blank, Some(custom), None, None, None)
            .expect("blank pane override");
        assert!(!blank.env.is_empty());
        assert!(blank.lease.is_some());
    }

    #[test]
    fn nested_gridbash_replaces_managed_parent_lane() {
        let root = TestRoot::new();
        let pool = root.pool();
        let parent = OsStr::new("C:\\gridbash\\parent-sqlite");

        let pane = pool
            .for_pane_with_inherited(&BTreeMap::new(), Some(parent), Some(parent), None, None)
            .expect("nested pane lease");

        assert_ne!(sqlite_home(&pane), Path::new(parent));
        assert!(pane.lease.is_some());
    }

    #[test]
    fn separates_lane_pools_by_codex_home() {
        let root = TestRoot::new();
        let pool = root.pool();
        let mut first_env = BTreeMap::new();
        first_env.insert(CODEX_HOME_ENV.into(), "C:\\codex-a".into());
        let mut second_env = BTreeMap::new();
        second_env.insert(CODEX_HOME_ENV.into(), "C:\\codex-b".into());

        let first = pool
            .for_pane_with_inherited(&first_env, None, None, None, None)
            .expect("first Codex home");
        let second = pool
            .for_pane_with_inherited(&second_env, None, None, None, None)
            .expect("second Codex home");

        assert_ne!(sqlite_home(&first).parent(), sqlite_home(&second).parent());
    }
}
