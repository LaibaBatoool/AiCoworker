use crate::workspace::Workspace;
use std::fs;
use std::time::UNIX_EPOCH;

#[derive(serde::Serialize)]
pub struct FileMetadata {
    pub relative_path: String,
    pub size_bytes: u64,
    pub is_directory: bool,
    pub modified_unix_timestamp: Option<u64>,
    pub read_only: bool,
}

pub fn get_file_metadata(workspace: &Workspace, relative_path: &str) -> Result<FileMetadata, String> {
    let resolved_path = workspace.resolve(relative_path)?; // boundary check first, always

    let metadata = fs::metadata(&resolved_path)
        .map_err(|e| format!("Failed to get metadata: {}", e))?;

    let modified_unix_timestamp = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    Ok(FileMetadata {
        relative_path: relative_path.to_string(),
        size_bytes: metadata.len(),
        is_directory: metadata.is_dir(),
        modified_unix_timestamp,
        read_only: metadata.permissions().readonly(),
    })
}