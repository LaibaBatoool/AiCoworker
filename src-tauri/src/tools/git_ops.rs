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

fn ensure_git_repo(workspace: &Workspace) -> Result<(), String> {
    let git_dir = workspace.root().join(".git");
    if !git_dir.exists() {
        return Err("This workspace is not a git repository (no .git folder found).".to_string());
    }
    Ok(())
}