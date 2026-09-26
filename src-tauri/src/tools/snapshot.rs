use crate::workspace::Workspace;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(serde::Serialize)]
pub struct SnapshotRecord {
    pub commit_hash: String,
    pub timestamp_unix: u64,
    pub message: String,
}

const SNAPSHOT_DIR_NAME: &str = ".aicoworker/snapshots";

fn snapshot_git_dir(workspace: &Workspace) -> PathBuf {
    workspace.root().join(SNAPSHOT_DIR_NAME)
}

/// Strips Windows' "verbatim" \\?\ path prefix before a path is
/// handed to git.exe as a --git-dir/--work-tree argument.
///
/// std::fs::canonicalize (used by Workspace, likely for its own
/// boundary-checking) returns \\?\-prefixed paths on Windows. Rust's
/// own filesystem APIs handle that prefix transparently, but Git for
/// Windows does not reliably recognize a \\?\-prefixed --git-dir —
/// it reports "not a git repository" even immediately after a
/// successful `git init` at that exact path. This is a known
/// interop gap between Rust's canonicalization and Git for Windows,
/// not a bug in the snapshot logic itself. On non-Windows platforms
/// this is a no-op, since canonicalize() never adds this prefix.
fn git_safe_path(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", stripped)
    } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        s
    }
}

/// A reliable marker that the shadow repo is actually a valid,
/// initialized git repository — not just that the folder exists.
fn is_valid_repo(git_dir: &Path) -> bool {
    git_dir.join("HEAD").exists()
}

/// Runs a git command against the shadow snapshot repo — NOT the
/// user's own .git, if they have one. --git-dir/--work-tree keeps
/// this completely separate from git_ops.rs's git_diff/git_commit,
/// which operate on the user's real repository.
fn shadow_git(workspace: &Workspace, args: &[&str]) -> Result<std::process::Output, String> {
    let git_dir = snapshot_git_dir(workspace);
    let work_tree = workspace.root();

    let mut full_args = vec![
        "--git-dir".to_string(),
        git_safe_path(&git_dir),
        "--work-tree".to_string(),
        git_safe_path(work_tree),
    ];
    full_args.extend(args.iter().map(|s| s.to_string()));

    Command::new("git")
        .args(&full_args)
        .output()
        .map_err(|e| format!("Failed to run shadow git command: {}", e))
}

/// If the workspace itself has its own real git repo (the one
/// git_diff/git_commit operate on), makes sure that repo ignores
/// .aicoworker/ via its local (untracked) exclude file — so the
/// shadow snapshot database never shows up in the user's own git
/// status/diff/commits. Best-effort: failure here is not fatal to
/// snapshotting, just logged.
fn exclude_from_real_repo_if_present(workspace: &Workspace) {
    let real_git_dir = workspace.root().join(".git");
    if !real_git_dir.is_dir() {
        return; // workspace isn't its own git repo — nothing to protect
    }

    let exclude_path = real_git_dir.join("info").join("exclude");
    let already_excluded = std::fs::read_to_string(&exclude_path)
        .map(|contents| contents.lines().any(|line| line.trim() == ".aicoworker/"))
        .unwrap_or(false);

    if already_excluded {
        return;
    }

    if let Some(parent) = exclude_path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            eprintln!("Warning: could not create .git/info for exclude patch");
            return;
        }
    }

    use std::io::Write;
    match std::fs::OpenOptions::new().create(true).append(true).open(&exclude_path) {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, ".aicoworker/") {
                eprintln!("Warning: could not append to .git/info/exclude: {}", e);
            }
        }
        Err(e) => eprintln!("Warning: could not open .git/info/exclude: {}", e),
    }
}

/// Ensures the shadow snapshot repo exists and is actually valid.
/// Idempotent and self-healing: if a previous attempt left behind a
/// half-created folder, this detects that via `is_valid_repo` and
/// retries init rather than trusting the folder's mere existence.
fn ensure_snapshot_repo(workspace: &Workspace) -> Result<(), String> {
    let git_dir = snapshot_git_dir(workspace);
    exclude_from_real_repo_if_present(workspace);

    if is_valid_repo(&git_dir) {
        return Ok(());
    }

    std::fs::create_dir_all(&git_dir)
        .map_err(|e| format!("Failed to create snapshot dir: {}", e))?;

    let init = shadow_git(workspace, &["init"])?;
    if !init.status.success() {
        return Err(format!(
            "Failed to init snapshot repo: {}",
            String::from_utf8_lossy(&init.stderr)
        ));
    }

    shadow_git(workspace, &["config", "user.email", "aicoworker@local"])?;
    shadow_git(workspace, &["config", "user.name", "AI CoWorker"])?;

    let exclude_path = git_dir.join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&exclude_path, ".aicoworker/\n")
        .map_err(|e| format!("Failed to write exclude file: {}", e))?;

    if !is_valid_repo(&git_dir) {
        return Err(format!(
            "git init reported success but no HEAD was created at {}",
            git_safe_path(&git_dir)
        ));
    }

    Ok(())
}

/// Takes a snapshot of the entire workspace as it exists RIGHT NOW,
/// before a mutating/privileged action runs.
pub fn take_snapshot(workspace: &Workspace, label: &str) -> Result<String, String> {
    ensure_snapshot_repo(workspace)?;

    let add = shadow_git(workspace, &["add", "-A"])?;
    if !add.status.success() {
        return Err(format!(
            "Snapshot 'git add' failed: {}",
            String::from_utf8_lossy(&add.stderr)
        ));
    }

    let commit = shadow_git(workspace, &["commit", "--allow-empty", "-m", label])?;
    if !commit.status.success() {
        return Err(format!(
            "Snapshot commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        ));
    }

    let rev_parse = shadow_git(workspace, &["rev-parse", "HEAD"])?;
    Ok(String::from_utf8_lossy(&rev_parse.stdout).trim().to_string())
}

/// Lists all snapshots, most recent first.
pub fn list_snapshots(workspace: &Workspace) -> Result<Vec<SnapshotRecord>, String> {
    let git_dir = snapshot_git_dir(workspace);
    if !is_valid_repo(&git_dir) {
        return Ok(Vec::new());
    }

    let log = shadow_git(workspace, &["log", "--pretty=format:%H|%ct|%s"])?;
    if !log.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&log.stdout);
    let records = stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            let commit_hash = parts.next()?.to_string();
            let timestamp_unix: u64 = parts.next()?.parse().ok()?;
            let message = parts.next().unwrap_or("").to_string();
            Some(SnapshotRecord { commit_hash, timestamp_unix, message })
        })
        .collect();

    Ok(records)
}

/// Restores the workspace to exactly the state captured in
/// `commit_hash`.
pub fn restore_snapshot(workspace: &Workspace, commit_hash: &str) -> Result<(), String> {
    let git_dir = snapshot_git_dir(workspace);
    if !is_valid_repo(&git_dir) {
        return Err("No snapshots exist for this workspace yet.".to_string());
    }

    let verify = shadow_git(workspace, &["cat-file", "-e", commit_hash])?;
    if !verify.status.success() {
        return Err(format!("'{}' is not a known snapshot.", commit_hash));
    }

    let reset = shadow_git(workspace, &["reset", "--hard", commit_hash])?;
    if !reset.status.success() {
        return Err(format!(
            "Restore failed: {}",
            String::from_utf8_lossy(&reset.stderr)
        ));
    }

    Ok(())
}