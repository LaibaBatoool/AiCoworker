use crate::workspace::Workspace;
use std::fs;

/// Reads a file's contents, but ONLY if it's inside the workspace.
pub fn read_file(workspace: &Workspace, relative_path: &str) -> Result<String, String> {
    let resolved_path = workspace.resolve(relative_path)?; // boundary check happens here

    fs::read_to_string(&resolved_path)
        .map_err(|e| format!("Failed to read file: {}", e))
}