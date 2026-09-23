use crate::workspace::Workspace;
use std::fs;

/// Deletes a file or directory. Privileged operation — irreversible.
/// For directories: refuses to delete non-empty folders unless
/// `recursive` is explicitly set to true, to prevent accidental
/// wholesale deletion from a single bad path.
pub fn delete_file(
    workspace: &Workspace,
    relative_path: &str,
    recursive: bool,
) -> Result<(), String> {
    let resolved_path = workspace.resolve(relative_path)?; // boundary check first, always

    if !resolved_path.exists() {
        return Err(format!("'{}' does not exist — nothing to delete.", relative_path));
    }

    if resolved_path.is_dir() {
        let is_empty = fs::read_dir(&resolved_path)
            .map_err(|e| format!("Failed to inspect directory: {}", e))?
            .next()
            .is_none();

        if is_empty {
            fs::remove_dir(&resolved_path)
                .map_err(|e| format!("Failed to delete empty directory: {}", e))
        } else if recursive {
            fs::remove_dir_all(&resolved_path)
                .map_err(|e| format!("Failed to delete directory and its contents: {}", e))
        } else {
            Err(format!(
                "'{}' is a non-empty directory. Set recursive=true to delete it and all its contents.",
                relative_path
            ))
        }
    } else {
        fs::remove_file(&resolved_path)
            .map_err(|e| format!("Failed to delete file: {}", e))
    }
}