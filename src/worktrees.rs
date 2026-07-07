use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};

#[derive(Debug, Clone)]
pub struct ManagedWorktreeOptions {
    pub prefix: String,
}

#[derive(Debug, Clone)]
pub struct ManagedPaneWorktree {
    pub cwd: PathBuf,
    pub folder_name: String,
    pub branch_name: String,
}

#[derive(Debug)]
struct GitRepo {
    root: PathBuf,
    common_git_dir: PathBuf,
    relative_cwd: PathBuf,
    base_slug: String,
}

#[derive(Debug)]
struct GitWorktree {
    path: PathBuf,
    branch: Option<String>,
}

impl ManagedWorktreeOptions {
    pub fn new(prefix: String) -> Result<Self> {
        let prefix = sanitize_slug(&prefix)
            .ok_or_else(|| anyhow!("worktree prefix must contain at least one letter or number"))?;
        Ok(Self { prefix })
    }
}

pub fn ensure_pane_worktrees(
    cwd: &Path,
    count: usize,
    options: &ManagedWorktreeOptions,
) -> Result<Vec<ManagedPaneWorktree>> {
    let repo = GitRepo::from_cwd(cwd)?;
    ensure_clean_tracked_checkout(&repo.root)?;

    let existing = list_git_worktrees(&repo.root)?;
    let worktree_dir = repo.root.join(".worktrees");
    fs::create_dir_all(&worktree_dir).with_context(|| {
        format!(
            "failed to create managed worktree directory {}",
            display_path(&worktree_dir)
        )
    })?;

    (0..count)
        .map(|index| ensure_pane_worktree(&repo, &existing, &worktree_dir, options, index))
        .collect()
}

impl GitRepo {
    fn from_cwd(cwd: &Path) -> Result<Self> {
        let root = git_output(cwd, args(&["rev-parse", "--show-toplevel"]))
            .with_context(|| format!("{} is not inside a git repository", display_path(cwd)))?;
        let root = PathBuf::from(root);
        let root = normalize_path(root.canonicalize().unwrap_or(root));
        let cwd = normalize_path(cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf()));
        let common_git_dir = git_common_dir(&root)?;
        let relative_cwd = cwd
            .strip_prefix(&root)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let base_name = git_output(&root, args(&["branch", "--show-current"]))
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                git_output(&root, args(&["rev-parse", "--short", "HEAD"]))
                    .ok()
                    .map(|hash| format!("detached-{hash}"))
            })
            .ok_or_else(|| anyhow!("git repository has no commit to base worktrees on"))?;
        let base_slug = sanitize_slug(&base_name)
            .ok_or_else(|| anyhow!("current git branch cannot be used for worktree names"))?;

        Ok(Self {
            root,
            common_git_dir,
            relative_cwd,
            base_slug,
        })
    }
}

fn ensure_pane_worktree(
    repo: &GitRepo,
    existing: &[GitWorktree],
    worktree_dir: &Path,
    options: &ManagedWorktreeOptions,
    index: usize,
) -> Result<ManagedPaneWorktree> {
    let pane_number = index + 1;
    let branch_name = format!(
        "{}/{}-pane-{pane_number:02}",
        options.prefix, repo.base_slug
    );
    if let Some(worktree) = existing
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch_name.as_str()))
    {
        return managed_pane(worktree.path.clone(), repo, branch_name);
    }

    let folder_name = format!("{}-{}-{pane_number:02}", options.prefix, repo.base_slug);
    let worktree_path = worktree_dir.join(&folder_name);
    if worktree_path.exists() {
        validate_existing_worktree_path(&worktree_path, &branch_name, repo)?;
        return managed_pane(worktree_path, repo, branch_name);
    }

    if branch_exists(&repo.root, &branch_name)? {
        git_output(
            &repo.root,
            vec![
                OsString::from("worktree"),
                OsString::from("add"),
                worktree_path.as_os_str().to_os_string(),
                OsString::from(&branch_name),
            ],
        )?;
    } else {
        git_output(
            &repo.root,
            vec![
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("-b"),
                OsString::from(&branch_name),
                worktree_path.as_os_str().to_os_string(),
                OsString::from("HEAD"),
            ],
        )?;
    }

    managed_pane(worktree_path, repo, branch_name)
}

fn managed_pane(
    worktree_path: PathBuf,
    repo: &GitRepo,
    branch_name: String,
) -> Result<ManagedPaneWorktree> {
    let cwd = if repo.relative_cwd.as_os_str().is_empty() {
        worktree_path.clone()
    } else {
        worktree_path.join(&repo.relative_cwd)
    };
    if !cwd.is_dir() {
        bail!(
            "managed worktree {} does not contain launch folder {}",
            display_path(&worktree_path),
            display_path(&cwd)
        );
    }

    Ok(ManagedPaneWorktree {
        cwd,
        folder_name: folder_display_name(&worktree_path),
        branch_name,
    })
}

fn validate_existing_worktree_path(
    path: &Path,
    expected_branch: &str,
    repo: &GitRepo,
) -> Result<()> {
    if !path.is_dir() {
        bail!(
            "managed worktree path exists but is not a directory: {}",
            display_path(path)
        );
    }

    let common_git_dir = git_common_dir(path)
        .with_context(|| format!("{} is not a git worktree", display_path(path)))?;
    if common_git_dir != repo.common_git_dir {
        bail!(
            "managed worktree path {} belongs to a different git repository",
            display_path(path)
        );
    }

    let branch = git_output(path, args(&["branch", "--show-current"]))
        .with_context(|| format!("{} is not a git worktree", display_path(path)))?;
    if branch != expected_branch {
        bail!(
            "managed worktree path {} is on branch {}, expected {}",
            display_path(path),
            branch,
            expected_branch
        );
    }
    Ok(())
}

fn git_common_dir(path: &Path) -> Result<PathBuf> {
    let common_dir = git_output(
        path,
        args(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
    )?;
    let common_dir = PathBuf::from(common_dir);
    Ok(normalize_path(
        common_dir.canonicalize().unwrap_or(common_dir),
    ))
}

fn ensure_clean_tracked_checkout(repo_root: &Path) -> Result<()> {
    let status = git_output(
        repo_root,
        args(&["status", "--porcelain", "--untracked-files=no"]),
    )?;
    if !status.is_empty() {
        bail!(
            "tracked changes are present in {}; commit or stash them before launching managed worktrees",
            display_path(repo_root)
        );
    }
    Ok(())
}

fn list_git_worktrees(repo_root: &Path) -> Result<Vec<GitWorktree>> {
    let output = git_output(repo_root, args(&["worktree", "list", "--porcelain"]))?;
    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;

    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            push_worktree(&mut worktrees, &mut current_path, &mut current_branch);
            current_path = Some(PathBuf::from(path));
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            current_branch = Some(branch.to_string());
        }
    }
    push_worktree(&mut worktrees, &mut current_path, &mut current_branch);

    Ok(worktrees)
}

fn push_worktree(
    worktrees: &mut Vec<GitWorktree>,
    path: &mut Option<PathBuf>,
    branch: &mut Option<String>,
) {
    if let Some(path) = path.take() {
        worktrees.push(GitWorktree {
            path,
            branch: branch.take(),
        });
    }
}

fn branch_exists(repo_root: &Path, branch: &str) -> Result<bool> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .status()
        .context("failed to run git rev-parse")?;
    Ok(status.success())
}

fn git_output(repo_root: &Path, args: Vec<OsString>) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(&args)
        .output()
        .with_context(|| format!("failed to run git {}", display_args(&args)))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("git {} failed", display_args(&args))
        } else {
            format!("git {} failed: {stderr}", display_args(&args))
        };
        bail!(message);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn display_args(args: &[OsString]) -> String {
    args.iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_slug(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    (!normalized.is_empty()).then_some(normalized)
}

fn folder_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| display_path(path))
}

fn display_path(path: &Path) -> String {
    let value = path.display().to_string();
    value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
}

fn normalize_path(path: PathBuf) -> PathBuf {
    PathBuf::from(display_path(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRepo {
        root: PathBuf,
    }

    impl TempRepo {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "gridbash-{name}-{}-{}",
                std::process::id(),
                unique_suffix()
            ));
            fs::create_dir_all(&root).expect("temp repo dir");
            run_git(&root, &["init"]);
            run_git(&root, &["branch", "-M", "main"]);
            fs::write(root.join(".gitignore"), ".worktrees/\n").expect("gitignore");
            fs::create_dir_all(root.join("crates").join("cli")).expect("subdir");
            fs::write(root.join("crates").join("cli").join("README.md"), "test\n")
                .expect("fixture file");
            run_git(&root, &["add", "."]);
            run_git_with_identity(&root, &["commit", "-m", "initial"]);
            Self { root }
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn creates_one_worktree_per_pane_and_reuses_existing_branches() {
        let repo = TempRepo::new("managed");
        let cwd = repo.root.join("crates").join("cli");
        let options = ManagedWorktreeOptions::new("gridbash".into()).expect("options");

        let panes = ensure_pane_worktrees(&cwd, 2, &options).expect("worktrees");

        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].folder_name, "gridbash-main-01");
        assert_eq!(panes[0].branch_name, "gridbash/main-pane-01");
        assert!(panes[0].cwd.ends_with(Path::new("crates").join("cli")));
        assert!(panes[0].cwd.is_dir());
        assert_eq!(
            git_branch(&panes[1].cwd),
            Some("gridbash/main-pane-02".into())
        );

        let reused = ensure_pane_worktrees(&cwd, 2, &options).expect("reused worktrees");
        assert_eq!(reused[0].cwd, panes[0].cwd);
        assert_eq!(reused[1].cwd, panes[1].cwd);
    }

    #[test]
    fn rejects_non_git_directories() {
        let root = std::env::temp_dir().join(format!(
            "gridbash-non-git-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&root).expect("temp dir");
        let options = ManagedWorktreeOptions::new("gridbash".into()).expect("options");

        let error = ensure_pane_worktrees(&root, 1, &options)
            .expect_err("non-git directory should fail")
            .to_string();

        assert!(error.contains("not inside a git repository"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_tracked_dirty_base_checkouts() {
        let repo = TempRepo::new("dirty");
        fs::write(
            repo.root.join("crates").join("cli").join("README.md"),
            "dirty\n",
        )
        .expect("dirty file");
        let options = ManagedWorktreeOptions::new("gridbash".into()).expect("options");

        let error = ensure_pane_worktrees(&repo.root, 1, &options)
            .expect_err("dirty checkout should fail")
            .to_string();

        assert!(error.contains("tracked changes are present"));
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn run_git_with_identity(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "user.name=GridBash Test"])
            .args(["-c", "user.email=gridbash@example.test"])
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn git_branch(root: &Path) -> Option<String> {
        git_output(root, args(&["branch", "--show-current"])).ok()
    }
}
