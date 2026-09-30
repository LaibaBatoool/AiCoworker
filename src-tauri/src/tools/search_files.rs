// src-tauri/src/tools/search_files.rs
use crate::workspace::Workspace;
use std::fs;
use std::path::Path;

#[derive(serde::Serialize)]
pub struct SearchResult {
    pub relative_path: String,
    pub matched_on: String, // "filename" or "content"
}

const MAX_RESULTS: usize = 50; // context-flooding guard, same principle as list_directory's non-recursion
const MAX_FILE_SIZE_FOR_CONTENT_SEARCH: u64 = 2_000_000; // 2MB — skip huge files for content search

/// AI CoWorker's own internal state (snapshot history, audit log) —
/// never something a search should recurse into or return matches
/// from. Same exclusion as list_directory.rs. This also matters for
/// correctness, not just tidiness: without it, search_files would
/// recurse into .aicoworker's git internals (loose objects, refs)
/// and waste time/results on binary git data.
const INTERNAL_DIR_NAME: &str = ".aicoworker";

/// Searches the workspace recursively for files matching a name
/// pattern and/or containing specific text. At least one of
/// name_pattern / content_pattern must be provided.
pub fn search_files(
    workspace: &Workspace,
    name_pattern: Option<&str>,
    content_pattern: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    if name_pattern.is_none() && content_pattern.is_none() {
        return Err("Provide at least a name_pattern or a content_pattern to search for.".to_string());
    }

    let mut results = Vec::new();
    search_recursive(
        workspace.root(),
        workspace.root(),
        name_pattern,
        content_pattern,
        &mut results,
    )?;

    Ok(results)
}

fn search_recursive(
    root: &Path,
    current_dir: &Path,
    name_pattern: Option<&str>,
    content_pattern: Option<&str>,
    results: &mut Vec<SearchResult>,
) -> Result<(), String> {
    if results.len() >= MAX_RESULTS {
        return Ok(()); // stop early once we've hit the cap
    }

    let entries = fs::read_dir(current_dir)
        .map_err(|e| format!("Failed to read directory during search: {}", e))?;

    for entry in entries {
        if results.len() >= MAX_RESULTS {
            break;
        }

        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name == INTERNAL_DIR_NAME {
            continue; // never descend into or report on our own snapshot/audit state
        }

        if path.is_dir() {
            search_recursive(root, &path, name_pattern, content_pattern, results)?;
            continue;
        }

        let relative_path = path
            .strip_prefix(root)
            .map_err(|_| "Failed to compute relative path".to_string())?
            .to_string_lossy()
            .to_string();

        // Name matching (simple substring match, case-insensitive)
        if let Some(pattern) = name_pattern {
            if name.to_lowercase().contains(&pattern.to_lowercase()) {
                results.push(SearchResult {
                    relative_path: relative_path.clone(),
                    matched_on: "filename".to_string(),
                });
                continue; // don't double-count if it also matches content
            }
        }

        // Content matching (only for reasonably small text-like files)
        if let Some(pattern) = content_pattern {
            if let Ok(metadata) = fs::metadata(&path) {
                if metadata.len() <= MAX_FILE_SIZE_FOR_CONTENT_SEARCH {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if content.to_lowercase().contains(&pattern.to_lowercase()) {
                            results.push(SearchResult {
                                relative_path,
                                matched_on: "content".to_string(),
                            });
                        }
                    }
                    // non-UTF8/binary files silently skipped for content search — expected
                }
            }
        }
    }

    Ok(())
}