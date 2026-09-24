use crate::workspace::Workspace;
use std::fs;

/// Moves or renames a file/directory. Both source and destination
/// must resolve inside the workspace boundary — this prevents both
/// moving something OUT of the workspace and moving something IN
/// from outside (the destination check alone isn't enough, since
/// `resolve` is called on both paths independently).
pub fn move_rename(
    workspace: &Workspace,
    from_relative_path: &str,
    to_relative_path: &str,
) -> Result<(), String> {
    let from_path = workspace.resolve(from_relative_path)?;
    let to_path = workspace.resolve(to_relative_path)?;

    if !from_path.exists() {
        return Err(format!("'{}' does not exist.", from_relative_path));
    }

    if to_path.exists() {
        return Err(format!(
            "'{}' already exists — refusing to overwrite via move. Delete it first if intended.",
            to_relative_path
        ));
    }

    fs::rename(&from_path, &to_path)
        .map_err(|e| format!("Failed to move/rename: {}", e))
}