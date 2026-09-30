use crate::permission::PermissionTier;

/// Classifies a shell command's risk tier by pattern matching.
/// Deliberately rule-based, not AI-based — the classification must
/// be deterministic and auditable, matching the rest of the
/// permission system's "code decides, not the model" philosophy.
pub fn classify_command_risk(command: &str) -> PermissionTier {
    let lower = command.to_lowercase();

    // `date`/`time` get checked by exact first-token match, not a
    // plain substring like the rest of dangerous_patterns below —
    // a substring check would also flag harmless commands like
    // "update.txt" or "validate-input.sh". Found via a live agent
    // run: it called `date -d @<unix_timestamp>` to convert a
    // timestamp for display. That's Unix syntax; on Windows, `date`
    // with no recognized flag drops into an INTERACTIVE prompt to
    // set the system clock ("Enter the new date: (dd-mm-yy)"). It
    // was classified Mutating (always passes, no confirmation) and
    // ran without asking — only the existing execute-timeout caught
    // the resulting hang. `/t` (Windows' read-only "just print it"
    // flag) is the one safe case that should stay unprivileged.
    let first_token = lower.split_whitespace().next().unwrap_or("");
    if (first_token == "date" || first_token == "time") && !lower.contains("/t") {
        return PermissionTier::Privileged;
    }

    let dangerous_patterns = [
        "rm -rf", "del /f", "del /s", "format ", "rd /s",
        "sudo", "runas", "chmod 777", "chmod -r 777",
        "> /dev/", "curl | sh", "curl | bash", "iwr | iex",
        "shutdown", "reg delete", "diskpart",
    ];

    let safe_patterns = [
        "ls", "dir", "cat", "type ", "pwd", "cd ", "echo ",
        "git status", "git diff", "git log", "whoami",
        "node -v", "npm -v", "python --version", "cargo --version",
    ];

    if dangerous_patterns.iter().any(|p| lower.contains(p)) {
        return PermissionTier::Privileged;
    }

    if safe_patterns.iter().any(|p| lower.starts_with(p)) {
        return PermissionTier::Safe;
    }

    PermissionTier::Mutating
}