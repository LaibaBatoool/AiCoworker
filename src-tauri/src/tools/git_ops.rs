use crate::workspace::Workspace;
use std::process::Command;

#[derive(serde::Serialize)]
pub struct GitDiffResult {
    pub diff: String,
    pub has_changes: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct GitCommitResult {
    pub success: bool,
    pub output: String,
}

#[derive(serde::Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    pub status: String,
}

#[derive(serde::Serialize)]
pub struct GitStatusResult {
    pub entries: Vec<GitStatusEntry>,
    pub clean: bool,
}

#[derive(serde::Serialize)]
pub struct GitLogEntry {
    pub commit_hash: String,
    pub author: String,
    pub timestamp_unix: i64,
    pub message: String,
}

pub fn git_diff(workspace: &Workspace) -> Result<GitDiffResult, String> {
    ensure_git_repo(workspace)?;

    let output = Command::new("git")
        .args(["diff"])
        .current_dir(workspace.root())
        .output()
        .map_err(|e| format!("Failed to run git diff: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff failed: {}", stderr));
    }

    let diff = String::from_utf8_lossy(&output.stdout).to_string();
    let has_changes = !diff.trim().is_empty();

    Ok(GitDiffResult { diff, has_changes })
}

pub fn git_commit(workspace: &Workspace, message: &str) -> Result<GitCommitResult, String> {
    ensure_git_repo(workspace)?;

    if message.trim().is_empty() {
        return Err("Commit message cannot be empty.".to_string());
    }

    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace.root())
        .output()
        .map_err(|e| format!("Failed to run git status: {}", e))?;

    let status_text = String::from_utf8_lossy(&status_output.stdout);
    if status_text.trim().is_empty() {
        return Err("Nothing to commit — working tree is clean.".to_string());
    }

    let add_output = Command::new("git")
        .args(["add", "."])
        .current_dir(workspace.root())
        .output()
        .map_err(|e| format!("Failed to run git add: {}", e))?;

    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        return Err(format!("git add failed: {}", stderr));
    }

    let commit_output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(workspace.root())
        .output()
        .map_err(|e| format!("Failed to run git commit: {}", e))?;

    let success = commit_output.status.success();
    let combined_output = if success {
        String::from_utf8_lossy(&commit_output.stdout).to_string()
    } else {
        String::from_utf8_lossy(&commit_output.stderr).to_string()
    };

    if !success {
        return Err(format!("git commit failed: {}", combined_output));
    }

    Ok(GitCommitResult {
        success,
        output: combined_output,
    })
}

/// Returns a structured list of changed files, parsed from
/// `git status --porcelain` rather than handing the model raw
/// porcelain codes to interpret itself.
pub fn git_status(workspace: &Workspace) -> Result<GitStatusResult, String> {
    ensure_git_repo(workspace)?;

    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace.root())
        .output()
        .map_err(|e| format!("Failed to run git status: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git status failed: {}", stderr));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<GitStatusEntry> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let code = line.get(0..2).unwrap_or("  ");
            let path = line.get(3..).unwrap_or("").to_string();
            GitStatusEntry {
                path,
                status: describe_status_code(code),
            }
        })
        .collect();

    let clean = entries.is_empty();
    Ok(GitStatusResult { entries, clean })
}

/// Porcelain v1 status codes are two characters: index status, then
/// worktree status (a space in either slot means "no change there").
/// "??" is the one special case meaning fully untracked.
fn describe_status_code(code: &str) -> String {
    let mut chars = code.chars();
    let x = chars.next().unwrap_or(' ');
    let y = chars.next().unwrap_or(' ');

    if x == '?' && y == '?' {
        return "untracked".to_string();
    }

    let describe = |c: char| -> Option<&'static str> {
        match c {
            'M' => Some("modified"),
            'A' => Some("added"),
            'D' => Some("deleted"),
            'R' => Some("renamed"),
            'C' => Some("copied"),
            'U' => Some("unmerged"),
            _ => None,
        }
    };

    let mut parts = Vec::new();
    if let Some(d) = describe(x) {
        parts.push(format!("{} (staged)", d));
    }
    if let Some(d) = describe(y) {
        parts.push(format!("{} (unstaged)", d));
    }

    if parts.is_empty() {
        "changed".to_string()
    } else {
        parts.join(", ")
    }
}

/// Returns the most recent `max_count` commits (clamped to 1-50 so a
/// bad or missing value can't ask git to dump an entire huge repo's
/// worth of history into one tool result).
pub fn git_log(workspace: &Workspace, max_count: u32) -> Result<Vec<GitLogEntry>, String> {
    ensure_git_repo(workspace)?;
    let count = max_count.clamp(1, 50);

    // \x1f (ASCII unit separator) as the field delimiter avoids any
    // collision with spaces, colons, or punctuation that could appear
    // naturally inside an author name or commit message.
    let output = Command::new("git")
        .args(["log", &format!("-{}", count), "--pretty=format:%H\x1f%an\x1f%at\x1f%s"])
        .current_dir(workspace.root())
        .output()
        .map_err(|e| format!("Failed to run git log: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // An empty repo with no commits yet is a normal state, not a
        // real failure worth erroring out over.
        if stderr.contains("does not have any commits yet") {
            return Ok(Vec::new());
        }
        return Err(format!("git log failed: {}", stderr));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let entries = text
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\u{1f}');
            let commit_hash = parts.next()?.to_string();
            let author = parts.next()?.to_string();
            let timestamp_unix: i64 = parts.next()?.parse().ok()?;
            let message = parts.next().unwrap_or("").to_string();
            Some(GitLogEntry { commit_hash, author, timestamp_unix, message })
        })
        .collect();

    Ok(entries)
}

fn ensure_git_repo(workspace: &Workspace) -> Result<(), String> {
    let git_dir = workspace.root().join(".git");
    if !git_dir.exists() {
        return Err("This workspace is not a git repository (no .git folder found).".to_string());
    }
    Ok(())
}