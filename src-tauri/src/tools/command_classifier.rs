#[derive(Debug, PartialEq, serde::Serialize)]
pub enum RiskLevel {
    Safe,
    Mutating,
    Privileged,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskLevel::Safe => "safe",
            RiskLevel::Mutating => "mutating",
            RiskLevel::Privileged => "privileged",
        }
    }
}

/// Classifies a shell command's risk tier by pattern matching.
/// Deliberately rule-based, not AI-based — the classification must
/// be deterministic and auditable, matching the rest of the
/// permission system's "code decides, not the model" philosophy.
pub fn classify_command_risk(command: &str) -> RiskLevel {
    let lower = command.to_lowercase();

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
        return RiskLevel::Privileged;
    }

    if safe_patterns.iter().any(|p| lower.starts_with(p)) {
        return RiskLevel::Safe;
    }

    // Default: anything not explicitly known-safe or known-dangerous
    // is treated as mutating — logged, but not blocked. This errs
    // toward caution without being overly restrictive on unknown commands.
    RiskLevel::Mutating
}