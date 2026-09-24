use crate::workspace::Workspace;
use std::fs;

/// Creates a new directory, including any missing parent directories.
pub fn create_directory(workspace: &Workspace, relative_path: &str) -> Result<(), String> {
    let resolved_path = workspace.resolve(relative_path)?; // boundary check first, always

    if resolved_path.exists() {
        return Err(format!("'{}' already exists.", relative_path));
    }

    fs::create_dir_all(&resolved_path)
        .map_err(|e| format!("Failed to create directory: {}", e))
}