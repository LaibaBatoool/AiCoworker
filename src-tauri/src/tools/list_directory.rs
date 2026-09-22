use crate::workspace::Workspace;
use std::fs;

/// Represents one entry (file or folder) inside a listed directory.
#[derive(serde::Serialize)]
pub struct DirEntry {
    pub name: String,
    pub is_directory: bool,
}

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
        let is_directory = entry.path().is_dir();
        result.push(DirEntry { name, is_directory });
    }

    Ok(result)
}