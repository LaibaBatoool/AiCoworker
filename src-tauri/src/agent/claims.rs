//! Claim check: before a run ends, the backend checks the files the agent SAYS
//! it created actually exist.
//!
//! Why: false completion was the most frequent failure in the benchmark (S2,
//! S7, and the memory test — findings 3, 14, 21, 24): the agent answered "done"
//! while the file it named was missing. Telling the model "don't do that" in
//! the prompt is the kind of rule that did not hold elsewhere (finding 1), so
//! this is a deterministic check in code instead.
//!
//! What it does: takes the sentences of the final answer that claim creation
//! ("created", "wrote", "saved", ...), pulls out file names, and looks for each
//! one in the workspace (exact path, or the same file name anywhere inside).
//! Deliberately conservative: negated sentences are ignored, and a file only
//! counts as missing if no file with that name exists anywhere in the workspace.
//!
//! OFF by default (the benchmark stays comparable); the app switches it on.
//! `AICOWORKER_CLAIM_CHECK=on|off` overrides both.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Marks the backend's feedback message, so the check runs at most once per run.
pub const MARKER: &str = "[Automatic check by AI CoWorker backend]";

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    match std::env::var("AICOWORKER_CLAIM_CHECK").map(|v| v.to_lowercase()) {
        Ok(v) if v == "on" || v == "1" || v == "true" => true,
        Ok(v) if v == "off" || v == "0" || v == "false" => false,
        _ => ENABLED.load(Ordering::Relaxed),
    }
}

const CLAIM_VERBS: &[&str] = &[
    "created", "create", "wrote", "written", "write", "saved", "save", "generated", "produced",
    "built", "exported", "made", "added", "stored", "output", "contains", "containing",
];
const NEGATIONS: &[&str] = &[
    "not", "no", "never", "cannot", "can't", "couldn't", "didn't", "wasn't", "won't", "unable", "failed",
    "deleted", "removed", "already", "skipped", "without", "nothing",
];
/// Names that look like files but are usually technologies.
const NOT_FILES: &[&str] = &["node.js", "next.js", "vue.js", "react.js", "asp.net", "e.g", "i.e", "etc."];

/// Splits text into rough sentences / list items.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let next = chars.get(i + 1).copied().unwrap_or(' ');
        // a '.' ends a sentence only when followed by whitespace (so "notes.txt" stays whole)
        if c == '\n' || ((c == '.' || c == '!' || c == '?') && next.is_whitespace()) {
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_string());
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// File-like tokens: something.ext with a letter in a short extension.
fn file_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in s.split(|c: char| c.is_whitespace() || "`'\"()[]{}<>,;*".contains(c)) {
        let t = raw.trim_end_matches(|c: char| c == '.' || c == '!' || c == '?' || c == ':');
        if t.is_empty() || t.contains("://") || t.starts_with('-') {
            continue;
        }
        if NOT_FILES.contains(&t.to_lowercase().as_str()) {
            continue;
        }
        let Some((stem, ext)) = t.rsplit_once('.') else { continue };
        let stem_name = stem.rsplit(|c| c == '/' || c == '\\').next().unwrap_or("");
        let ext_ok = (1..=5).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic());
        let stem_ok = !stem_name.is_empty() && stem_name.chars().all(|c| c.is_alphanumeric() || "_-. ".contains(c));
        if ext_ok && stem_ok {
            out.push(t.replace('\\', "/").trim_start_matches("./").to_string());
        }
    }
    out
}

/// File names the answer claims were created/written (negated sentences skipped).
pub fn claimed_files(answer: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    for s in sentences(answer) {
        let w = words(&s);
        let claims = w.iter().any(|x| CLAIM_VERBS.contains(&x.as_str()));
        let negated = w.iter().any(|x| NEGATIONS.contains(&x.as_str()));
        if claims && !negated {
            for f in file_tokens(&s) {
                found.insert(f);
            }
        }
    }
    found.into_iter().collect()
}

/// All file names (lower-case) anywhere in the workspace, up to a sane limit.
fn names_in_workspace(root: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > 20_000 {
                return names;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if matches!(name.as_str(), ".aicoworker" | ".git" | "node_modules" | "target") {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < 8 {
                    stack.push((e.path(), depth + 1));
                }
            } else {
                names.insert(name.to_lowercase());
            }
        }
    }
    names
}

/// Claimed files that exist nowhere in the workspace.
pub fn missing_claimed_files(workspace_root: &str, answer: &str) -> Vec<String> {
    let claimed = claimed_files(answer);
    if claimed.is_empty() {
        return Vec::new();
    }
    let root = Path::new(workspace_root);
    let names = names_in_workspace(root);
    claimed
        .into_iter()
        .filter(|f| {
            let exact = root.join(f).is_file();
            let base = f.rsplit('/').next().unwrap_or(f).to_lowercase();
            !exact && !names.contains(&base)
        })
        .collect()
}

/// The message sent back to the model when claimed files are missing.
pub fn feedback(missing: &[String]) -> String {
    format!(
        "{} Your answer says these files were created or written, but they do NOT exist in the workspace: {}. \
         Do the missing work now with your tools, or correct your answer. Do not claim work you did not do.",
        MARKER,
        missing.join(", ")
    )
}

// ---------------------------------------------------------------
// Tests: `cargo test claims`
// ---------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_claimed_files() {
        let a = "I created `count.txt` with the total.\n- Wrote report/summary.md\nThe data came from sales.csv.";
        assert_eq!(claimed_files(a), vec!["count.txt", "report/summary.md"], "sales.csv is only read, not claimed");
    }

    #[test]
    fn ignores_negations_urls_and_tech_names() {
        assert!(claimed_files("I could not create notes.txt because the folder is read-only.").is_empty());
        assert!(claimed_files("I didn't write out.txt.").is_empty());
        assert!(claimed_files("Deleted old.log and created nothing else.").is_empty(), "negated sentence");
        assert!(claimed_files("Created a page at https://example.com/a.html").is_empty());
        assert!(claimed_files("Built it with Node.js and saved it.").is_empty());
        assert!(claimed_files("Saved version 1.2 of the plan").is_empty(), "1.2 is not a file");
    }

    #[test]
    fn missing_means_nowhere_in_the_workspace() {
        let d = std::env::temp_dir().join(format!("aicw_claims_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("docs")).unwrap();
        std::fs::write(d.join("docs/notes.txt"), "x").unwrap();
        let root = d.to_str().unwrap();
        // named without its folder, but it exists → not missing
        assert!(missing_claimed_files(root, "I created notes.txt in the docs folder.").is_empty());
        assert_eq!(missing_claimed_files(root, "I wrote count.txt and notes.txt."), vec!["count.txt"]);
        assert!(feedback(&["count.txt".into()]).starts_with(MARKER));
        let _ = std::fs::remove_dir_all(&d);
    }
}