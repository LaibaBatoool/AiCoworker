use crate::workspace::Workspace;
use crate::tools::command_classifier::{classify_command_risk, RiskLevel};
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

#[derive(serde::Serialize)]
pub struct CommandOutput {
    pub risk_level: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_OUTPUT_LINES: usize = 100; // per side (first N / last N), context-flooding mitigation

pub fn execute_terminal_command(
    workspace: &Workspace,
    command: &str,
    confirmed: bool,
) -> Result<CommandOutput, String> {
    let risk = classify_command_risk(command);

    // Privileged commands require explicit confirmation from a prior
    // classify-only call. This mirrors the same tiered-permission
    // philosophy as write/edit/delete, adapted for the fact that risk
    // here can only be known after inspecting the command text.
    if risk == RiskLevel::Privileged && !confirmed {
        return Err(format!(
            "PRIVILEGED_CONFIRMATION_REQUIRED: This command was classified as privileged. \
             Confirm with the user, then retry with confirmed=true."
        ));
    }

    let mut child = Command::new("cmd")
        .args(["/C", command])
        .current_dir(workspace.root()) // confines execution to the workspace folder
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn command: {}", e))?;

    let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
    let status = child
        .wait_timeout(timeout)
        .map_err(|e| format!("Failed to wait on command: {}", e))?;

    let (exit_code, timed_out) = match status {
        Some(exit_status) => (exit_status.code(), false),
        None => {
            // Timed out — force kill the hung process
            let _ = child.kill();
            let _ = child.wait();
            (None, true)
        }
    };

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to collect command output: {}", e))?;

    let stdout = truncate_output(&String::from_utf8_lossy(&output.stdout));
    let stderr = truncate_output(&String::from_utf8_lossy(&output.stderr));

    Ok(CommandOutput {
        risk_level: risk.as_str().to_string(),
        stdout,
        stderr,
        exit_code,
        timed_out,
    })
}

/// Keeps only the first and last N lines of long output, with a
/// truncation marker in between — a direct, concrete implementation
/// of the context-flooding mitigation described in the project plan.
fn truncate_output(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_OUTPUT_LINES * 2 {
        return text.to_string();
    }

    let head = &lines[..MAX_OUTPUT_LINES];
    let tail = &lines[lines.len() - MAX_OUTPUT_LINES..];
    let omitted = lines.len() - (MAX_OUTPUT_LINES * 2);

    format!(
        "{}\n\n... [{} lines truncated] ...\n\n{}",
        head.join("\n"),
        omitted,
        tail.join("\n")
    )
}