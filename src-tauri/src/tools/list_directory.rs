// src-tauri/src/tools/list_directory.rs
use crate::workspace::Workspace;
use std::fs;

/// Represents one entry (file or folder) inside a listed directory.
#[derive(serde::Serialize)]
pub struct DirEntry {
    pub name: String,
    pub is_directory: bool,
}

/// AI CoWorker's own internal state lives here (snapshot history,
/// audit log). It's not part of the user's actual workspace content,
/// so it's filtered out of anything a person or the agent sees —
/// the same way it's already excluded from the user's real .git via
/// exclude_from_real_repo_if_present in snapshot.rs.
const INTERNAL_DIR_NAME: &str = ".aicoworker";

/// Lists all files and folders inside a given relative path,
/// but ONLY if that path is inside the workspace.
pub fn list_directory(workspace: &Workspace, relative_path: &str) -> Result<Vec<DirEntry>, String> {
    let resolved_path = workspace.resolve(relative_path)?; // same boundary check as read_file

    if !resolved_path.is_dir() {
        return Err(format!("'{}' is not a directory", relative_path));
    }

    let entries = fs::read_dir(&resolved_path)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();

        if name == INTERNAL_DIR_NAME {
            continue;
        }

        let is_directory = entry.path().is_dir();
        result.push(DirEntry { name, is_directory });
    }

    Ok(result)
}