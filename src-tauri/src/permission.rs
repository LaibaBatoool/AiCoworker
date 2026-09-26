use crate::workspace::Workspace;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum PermissionTier {
    Safe,
    Mutating,
    Privileged,
}

impl PermissionTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionTier::Safe => "safe",
            PermissionTier::Mutating => "mutating",
            PermissionTier::Privileged => "privileged",
        }
    }
}

#[derive(serde::Serialize)]
struct AuditLogEntry<'a> {
    timestamp_unix: u64,
    tier: &'a str,
    action: &'a str,
    success: bool,
    detail: &'a str,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct AuditLogRecord {
    pub timestamp_unix: u64,
    pub tier: String,
    pub action: String,
    pub success: bool,
    pub detail: String,
}

/// THE central permission checkpoint. Every mutating/privileged
/// action must pass through here before it is allowed to execute.
/// This is enforced at the Tauri-command layer — the frontend
/// cannot bypass it just by skipping a confirmation dialog, since
/// the backend itself refuses to proceed on an unconfirmed
/// privileged action regardless of what the frontend sends.
pub fn checkpoint(tier: &PermissionTier, action: &str, confirmed: bool) -> Result<(), String> {
    match tier {
        PermissionTier::Safe | PermissionTier::Mutating => Ok(()),
        PermissionTier::Privileged => {
            if confirmed {
                Ok(())
            } else {
                Err(format!(
                    "PRIVILEGED_CONFIRMATION_REQUIRED: '{}' is a privileged action and requires explicit user confirmation before it can run.",
                    action
                ))
            }
        }
    }
}

/// Appends an entry to the workspace's audit log
/// (.aicoworker/audit_log.jsonl). Called for every mutating and
/// privileged action, regardless of success or failure — this is
/// what makes the permission system inspectable and auditable,
/// not just "trust the UI happened to ask nicely."
pub fn log_action(workspace: &Workspace, tier: &PermissionTier, action: &str, success: bool, detail: &str) {
    // A logging failure should never break the actual operation —
    // note it to stderr, don't propagate it up to the caller.
    if let Err(e) = try_log_action(workspace, tier, action, success, detail) {
        eprintln!("Warning: failed to write audit log entry: {}", e);
    }
}

fn try_log_action(
    workspace: &Workspace,
    tier: &PermissionTier,
    action: &str,
    success: bool,
    detail: &str,
) -> Result<(), String> {
    let log_dir = workspace.root().join(".aicoworker");
    fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let log_path = log_dir.join("audit_log.jsonl");

    let timestamp_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let entry = AuditLogEntry {
        timestamp_unix,
        tier: tier.as_str(),
        action,
        success,
        detail,
    };

    let line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;

    writeln!(file, "{}", line).map_err(|e| e.to_string())?;

    Ok(())
}

/// Reads back the audit log for display, most recent entries first.
pub fn read_audit_log(workspace: &Workspace) -> Result<Vec<AuditLogRecord>, String> {
    let log_path = workspace.root().join(".aicoworker").join("audit_log.jsonl");

    if !log_path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&log_path).map_err(|e| e.to_string())?;

    let mut records: Vec<AuditLogRecord> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    records.reverse();
    Ok(records)
}