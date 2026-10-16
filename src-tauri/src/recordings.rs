//! Run recordings: every agent run seen in the Live view is saved as a small
//! JSON file (the live events + file-tree snapshots + the user's approve/decline
//! decisions), so it can be replayed later like a video — no API calls needed.
//!
//! Stored OUTSIDE the workspace (%APPDATA%\AiCoworker\recordings), like mcp.json:
//! the agent's file tools can't read, edit or delete them. These are UI-only
//! commands; they are not in the tool registry, so the model can't call them.

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_BYTES: usize = 5 * 1024 * 1024;
const KEEP_NEWEST: usize = 50;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct RecordingInfo {
    pub file: String,
    pub goal: String,
    pub workspace: String,
    pub started_at: u64,
    pub duration_ms: u64,
    pub actions: usize,
    pub outcome: String,
}

/// %APPDATA%\AiCoworker\recordings (or AICOWORKER_DATA_DIR\recordings).
pub fn recordings_dir() -> Option<PathBuf> {
    let base = std::env::var("AICOWORKER_DATA_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("APPDATA")
                .or_else(|_| std::env::var("XDG_CONFIG_HOME"))
                .or_else(|_| std::env::var("HOME").map(|h| format!("{}/.config", h)))
                .ok()
                .map(|b| Path::new(&b).join("AiCoworker"))
        })?;
    Some(base.join("recordings"))
}

/// Only names this module itself creates: digits, optional "-n", ".json".
/// Blocks "../", absolute paths, drive letters, anything else.
pub fn valid_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".json") else { return false };
    let mut parts = stem.splitn(2, '-');
    let a = parts.next().unwrap_or("");
    let b = parts.next();
    let digits = |s: &str| !s.is_empty() && s.len() <= 20 && s.chars().all(|c| c.is_ascii_digit());
    digits(a) && b.map(digits).unwrap_or(true)
}

fn summarize(file: &str, v: &Value) -> RecordingInfo {
    let events = v.get("events").and_then(|e| e.as_array()).cloned().unwrap_or_default();
    let kind = |e: &Value| e.get("e").and_then(|x| x.get("kind")).and_then(|k| k.as_str()).unwrap_or("").to_string();
    let actions = events.iter().filter(|e| kind(e) == "tool_started").count();
    let outcome = if events.iter().any(|e| kind(e) == "answer") {
        "done"
    } else if events.iter().any(|e| kind(e) == "stopped") {
        "stopped"
    } else {
        "unfinished"
    };
    RecordingInfo {
        file: file.to_string(),
        goal: v.get("goal").and_then(|g| g.as_str()).unwrap_or("").chars().take(200).collect(),
        workspace: v.get("workspace").and_then(|g| g.as_str()).unwrap_or("").to_string(),
        started_at: v.get("started_at").and_then(|g| g.as_u64()).unwrap_or(0),
        duration_ms: events.last().and_then(|e| e.get("t")).and_then(|t| t.as_u64()).unwrap_or(0),
        actions,
        outcome: outcome.to_string(),
    }
}

/// Saves a recording; returns the file name. Keeps only the newest 50.
pub fn save_in(dir: &Path, content: &str) -> Result<String, String> {
    if content.len() > MAX_BYTES {
        return Err(format!("Recording too large ({} bytes, max {})", content.len(), MAX_BYTES));
    }
    let v: Value = serde_json::from_str(content).map_err(|e| format!("Recording is not valid JSON: {}", e))?;
    if !v.get("events").map(|e| e.is_array()).unwrap_or(false) {
        return Err("Recording has no events list".into());
    }
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create recordings folder: {}", e))?;
    let stamp = v.get("started_at").and_then(|s| s.as_u64()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    });
    let mut name = format!("{}.json", stamp);
    let mut n = 1;
    while dir.join(&name).exists() {
        name = format!("{}-{}.json", stamp, n);
        n += 1;
    }
    fs::write(dir.join(&name), content).map_err(|e| format!("Cannot save recording: {}", e))?;

    // retention: delete the oldest beyond KEEP_NEWEST
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| valid_name(n)).collect())
        .unwrap_or_default();
    names.sort_by_key(|n| sort_key(n));
    while names.len() > KEEP_NEWEST {
        let old = names.remove(0);
        let _ = fs::remove_file(dir.join(old));
    }
    Ok(name)
}

fn sort_key(name: &str) -> (u64, u64) {
    let stem = name.trim_end_matches(".json");
    let mut p = stem.splitn(2, '-');
    (p.next().and_then(|a| a.parse().ok()).unwrap_or(0), p.next().and_then(|b| b.parse().ok()).unwrap_or(0))
}

/// Newest first.
pub fn list_in(dir: &Path) -> Vec<RecordingInfo> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| valid_name(n)).collect())
        .unwrap_or_default();
    names.sort_by_key(|n| std::cmp::Reverse(sort_key(n)));
    names
        .into_iter()
        .filter_map(|n| {
            let text = fs::read_to_string(dir.join(&n)).ok()?;
            let v: Value = serde_json::from_str(&text).ok()?;
            Some(summarize(&n, &v))
        })
        .collect()
}

pub fn load_in(dir: &Path, file: &str) -> Result<String, String> {
    if !valid_name(file) {
        return Err(format!("'{}' is not a recording name", file));
    }
    fs::read_to_string(dir.join(file)).map_err(|e| format!("Cannot read recording: {}", e))
}

// ---------------------------------------------------------------
// Tests: `cargo test recordings`
// ---------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aicw_rec_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn rec(start: u64, with_answer: bool) -> String {
        let mut events = vec![
            json!({ "t": 0, "e": { "kind": "thinking", "step": 1 } }),
            json!({ "t": 900, "e": { "kind": "tool_started", "call_id": "c1", "name": "write_file", "arguments": {}, "tier": "mutating" } }),
            json!({ "t": 950, "e": { "kind": "tool_finished", "call_id": "c1", "name": "write_file", "ok": true, "preview": "", "duration_ms": 50 } }),
        ];
        if with_answer {
            events.push(json!({ "t": 2400, "e": { "kind": "answer", "text": "done" } }));
        }
        json!({ "version": 1, "goal": "write a poem", "workspace": "D:\\ws", "started_at": start, "events": events }).to_string()
    }

    #[test]
    fn save_list_load_roundtrip() {
        let d = tmp("rt");
        let a = save_in(&d, &rec(1000, true)).unwrap();
        let b = save_in(&d, &rec(2000, false)).unwrap();
        let c = save_in(&d, &rec(2000, true)).unwrap(); // same start → unique name
        assert_eq!((a.as_str(), b.as_str(), c.as_str()), ("1000.json", "2000.json", "2000-1.json"));

        let list = list_in(&d);
        assert_eq!(list.iter().map(|r| r.file.as_str()).collect::<Vec<_>>(), vec!["2000-1.json", "2000.json", "1000.json"], "newest first");
        assert_eq!(list[1].outcome, "unfinished");
        assert_eq!(list[0].outcome, "done");
        assert_eq!((list[0].actions, list[0].duration_ms, list[0].goal.as_str()), (1, 2400, "write a poem"));
        assert_eq!(load_in(&d, "1000.json").unwrap(), rec(1000, true));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn rejects_bad_names_and_bad_content() {
        let d = tmp("bad");
        for n in ["../secret.json", "..\\x.json", "C:\\x.json", "abc.json", "1.txt", "1-.json", "-1.json", ""] {
            assert!(!valid_name(n), "{} must be rejected", n);
            assert!(load_in(&d, n).is_err());
        }
        assert!(valid_name("1760000000000.json") && valid_name("12-3.json"));
        assert!(save_in(&d, "not json").is_err());
        assert!(save_in(&d, r#"{"goal":"x"}"#).is_err(), "needs an events list");
        assert!(save_in(&d, &"x".repeat(MAX_BYTES + 1)).is_err());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn keeps_only_the_newest() {
        let d = tmp("keep");
        for i in 0..(KEEP_NEWEST as u64 + 3) {
            save_in(&d, &rec(10 + i, true)).unwrap();
        }
        let list = list_in(&d);
        assert_eq!(list.len(), KEEP_NEWEST);
        assert!(!d.join("10.json").exists() && !d.join("12.json").exists() && d.join("13.json").exists());
        let _ = fs::remove_dir_all(d);
    }
}