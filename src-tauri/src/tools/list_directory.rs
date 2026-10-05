use crate::workspace::Workspace;
use std::fs;
use std::time::UNIX_EPOCH;

/// Represents one entry (file or folder) inside a listed directory.
/// size_bytes/modified_unix_timestamp/read_only are only populated
/// when list_directory is called with include_metadata=true — kept
/// as Option so a plain listing's JSON stays exactly as small as
/// before for anyone not asking for metadata.
#[derive(serde::Serialize)]
pub struct DirEntry {
    pub name: String,
    pub is_directory: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_unix_timestamp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
}

/// AI CoWorker's own internal state lives here (snapshot history,
/// audit log). It's not part of the user's actual workspace content,
/// so it's filtered out of anything a person or the agent sees —
/// the same way it's already excluded from the user's real .git via
/// exclude_from_real_repo_if_present in snapshot.rs.
const INTERNAL_DIR_NAME: &str = ".aicoworker";

/// Lists all files and folders inside a given relative path, but
/// ONLY if that path is inside the workspace. When include_metadata
/// is true, each entry also carries size/modified-time/read-only in
/// this SAME call — this exists specifically because "list every
/// file and its metadata" was previously costing one list_directory
/// call plus one get_file_metadata call PER FILE, which is exactly
/// the kind of inefficiency the benchmark's tool-call-efficiency
/// metric would penalize.
pub fn list_directory(
    workspace: &Workspace,
    relative_path: &str,
    include_metadata: bool,
) -> Result<Vec<DirEntry>, String> {
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

        let entry_path = entry.path();
        let is_directory = entry_path.is_dir();

        let (size_bytes, modified_unix_timestamp, read_only) = if include_metadata {
            match fs::metadata(&entry_path) {
                Ok(metadata) => {
                    let modified = metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    (
                        Some(metadata.len()),
                        modified,
                        Some(metadata.permissions().readonly()),
                    )
                }
                // Best-effort — one entry's metadata failing to read
                // shouldn't fail the whole listing.
                Err(_) => (None, None, None),
            }
        } else {
            (None, None, None)
        };

        result.push(DirEntry {
            name,
            is_directory,
            size_bytes,
            modified_unix_timestamp,
            read_only,
        });
    }

    Ok(result)
}