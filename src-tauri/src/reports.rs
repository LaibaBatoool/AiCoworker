//! Run reports: a one-page HTML summary of an agent run (goal, plan, every step
//! with the tier the backend enforced, backend checks, files changed, answer),
//! built by the Live view from a live run or a replay — no API calls needed.
//!
//! Saved OUTSIDE the workspace (%APPDATA%\AiCoworker\reports), next to the
//! recordings, so the agent's tools can't touch them. UI-only command; not in
//! the tool registry, so the model can't call it.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_BYTES: usize = 5 * 1024 * 1024;

/// %APPDATA%\AiCoworker\reports (or AICOWORKER_DATA_DIR\reports).
pub fn reports_dir() -> Option<PathBuf> {
    crate::recordings::recordings_dir().and_then(|d| d.parent().map(|p| p.join("reports")))
}

/// Writes `run-<started_at>.html` (never overwrites: -1, -2, ...). Returns the full path.
pub fn save_in(dir: &Path, html: &str, started_at: u64) -> Result<PathBuf, String> {
    if html.len() > MAX_BYTES {
        return Err(format!("Report too large ({} bytes, max {})", html.len(), MAX_BYTES));
    }
    if !html.trim_start().to_lowercase().starts_with("<!doctype html") {
        return Err("Not an HTML report".to_string());
    }
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create reports folder: {}", e))?;
    let mut path = dir.join(format!("run-{}.html", started_at));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("run-{}-{}.html", started_at, n));
        n += 1;
    }
    fs::write(&path, html).map_err(|e| format!("Cannot save report: {}", e))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_without_overwriting() {
        let d = std::env::temp_dir().join(format!("aicw_reports_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let page = "<!DOCTYPE html><html><body>run</body></html>";
        let a = save_in(&d, page, 42).unwrap();
        let b = save_in(&d, page, 42).unwrap();
        assert_ne!(a, b, "a second report of the same run gets a new name");
        assert!(a.ends_with("run-42.html") && b.ends_with("run-42-1.html"));
        assert!(save_in(&d, "just text", 1).is_err(), "only HTML pages");
        assert!(save_in(&d, &format!("<!DOCTYPE html>{}", "x".repeat(MAX_BYTES)), 1).is_err(), "size cap");
        let _ = fs::remove_dir_all(&d);
    }
}