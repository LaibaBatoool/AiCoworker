use crate::workspace::Workspace;
use crate::tools::command_classifier::classify_command_risk;
use shared_child::SharedChild;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use wait_timeout::ChildExt;

#[derive(Debug, serde::Serialize)]
pub struct CommandOutput {
    pub risk_level: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_OUTPUT_LINES: usize = 100;

/// Kills a process AND all its descendants, by PID. Necessary on
/// Windows because every command here is spawned as
/// `cmd /C <command>` — so a plain kill() only terminates cmd.exe
/// itself, not whatever program cmd.exe spawned (e.g. ping.exe for a
/// long-running `ping -t`). An orphaned grandchild can keep running
/// AND keep its own handle to our piped stdout open, which makes any
/// code reading that pipe block forever waiting for an EOF that will
/// never come — even though the direct child is already dead.
/// taskkill's /T flag kills the whole tree rooted at a PID, which is
/// the standard workaround for this. Best-effort: if taskkill itself
/// can't be spawned, this logs and gives up rather than panicking —
/// same "never block the main flow" philosophy as snapshot_before.
pub fn kill_process_tree(pid: u32) {
    let pid_str = pid.to_string();
    if let Err(e) = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", pid_str.as_str()])
        .output()
    {
        eprintln!("Warning: failed to run taskkill for PID {}: {}", pid, e);
    }
}

pub fn execute_terminal_command(workspace: &Workspace, command: &str) -> Result<CommandOutput, String> {
    let risk = classify_command_risk(command);

    let mut child = Command::new("cmd")
        .args(["/C", command])
        .current_dir(workspace.root())
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
            kill_process_tree(child.id());
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

/// Spawns a command WITHOUT waiting for it — the first half of the
/// cancellable path used by start_command_tool. Returns immediately
/// so the caller can hand back a job id and let the frontend show a
/// Cancel button while the process is still running.
pub fn spawn_terminal_command(workspace: &Workspace, command: &str) -> Result<(Arc<SharedChild>, String), String> {
    let risk = classify_command_risk(command);

    let mut cmd = Command::new("cmd");
    cmd.args(["/C", command])
        .current_dir(workspace.root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = SharedChild::spawn(&mut cmd).map_err(|e| format!("Failed to spawn command: {}", e))?;

    Ok((Arc::new(child), risk.as_str().to_string()))
}

/// Waits on an already-spawned SharedChild, killing its whole process
/// tree automatically after DEFAULT_TIMEOUT_SECS if it hasn't
/// finished, OR earlier if a concurrent cancel_command_tool call sets
/// `cancelled` and kills it itself (also via kill_process_tree — see
/// running_commands.rs). Drains stdout/stderr on their own threads as
/// the process runs, rather than reading only after wait() returns —
/// a long-running or chatty command can otherwise fill the OS pipe
/// buffer and deadlock (the child blocks writing, nobody's reading
/// yet). This matters more here than in the synchronous path above,
/// since start_command_tool is specifically for commands expected to
/// potentially run long.
pub fn wait_for_spawned_command(
    child: Arc<SharedChild>,
    cancelled: Arc<AtomicBool>,
    risk_level: String,
) -> CommandOutput {
    let stdout_handle = child.take_stdout();
    let stderr_handle = child.take_stderr();

    let stdout_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stdout_handle {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stderr_handle {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });

    let watcher_child = Arc::clone(&child);
    let timed_out = Arc::new(AtomicBool::new(false));
    let watcher_timed_out = Arc::clone(&timed_out);
    let watcher_cancelled = Arc::clone(&cancelled);
    let _watcher = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(DEFAULT_TIMEOUT_SECS);
        loop {
            if watcher_cancelled.load(Ordering::SeqCst) {
                return; // cancel_command_tool is already killing it
            }
            match watcher_child.try_wait() {
                Ok(Some(_)) | Err(_) => return, // finished naturally (or already gone)
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        watcher_timed_out.store(true, Ordering::SeqCst);
                        kill_process_tree(watcher_child.id());
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    });

    let wait_result = child.wait();
    let exit_code = wait_result.ok().and_then(|s| s.code());

    let stdout_bytes = stdout_reader.join().unwrap_or_default();
    let stderr_bytes = stderr_reader.join().unwrap_or_default();

    let was_cancelled = cancelled.load(Ordering::SeqCst);
    let did_time_out = timed_out.load(Ordering::SeqCst);

    CommandOutput {
        risk_level: if was_cancelled {
            format!("{} (cancelled by user)", risk_level)
        } else if did_time_out {
            format!("{} (timed out)", risk_level)
        } else {
            risk_level
        },
        stdout: truncate_output(&String::from_utf8_lossy(&stdout_bytes)),
        stderr: truncate_output(&String::from_utf8_lossy(&stderr_bytes)),
        exit_code,
        timed_out: did_time_out,
    }
}

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