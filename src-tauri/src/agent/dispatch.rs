use serde_json::Value;

fn get_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Missing or non-string required argument '{}'", key))
}

fn get_bool(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn get_opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("Failed to serialize tool result: {}", e))
}

/// Executes one model-requested tool call by name, through the EXACT
/// same `_tool` functions your UI calls — this never talks to
/// permission.rs, audit logging, or snapshot.rs directly. `confirmed`
/// is decided entirely by the orchestrator (always false on a tool's
/// first attempt; true only when resuming after a user-approved
/// AwaitingConfirmation) — the model has no `confirmed` parameter in
/// its tool schema at all (see registry.rs's own comment on this).
pub fn dispatch_tool_call(
    workspace_root: &str,
    tool_name: &str,
    args: &Value,
    confirmed: bool,
) -> Result<Value, String> {
    let ws = workspace_root.to_string();

    match tool_name {
        "read_file" => crate::read_file_tool(ws, get_str(args, "relative_path")?).and_then(to_value),
        "list_directory" => {
            let include_metadata = get_bool(args, "include_metadata", false);
            crate::list_directory_tool(ws, get_str(args, "relative_path")?, Some(include_metadata)).and_then(to_value)
        }
        "search_files" => crate::search_files_tool(
            ws,
            get_opt_str(args, "name_pattern"),
            get_opt_str(args, "content_pattern"),
        )
        .and_then(to_value),
        "get_file_metadata" => crate::get_file_metadata_tool(ws, get_str(args, "relative_path")?).and_then(to_value),
        "git_diff" => crate::git_diff_tool(ws).and_then(to_value),
        "list_snapshots" => crate::list_snapshots_tool(ws).and_then(to_value),
        "write_file" => crate::write_file_tool(ws, get_str(args, "relative_path")?, get_str(args, "content")?, confirmed).and_then(to_value),
        "edit_file" => crate::edit_file_tool(
            ws,
            get_str(args, "relative_path")?,
            get_str(args, "old_text")?,
            get_str(args, "new_text")?,
            confirmed,
        )
        .and_then(to_value),
        "convert_to_pdf" => crate::convert_to_pdf_tool(ws, get_str(args, "relative_path")?, confirmed).and_then(to_value),
        "create_directory" => crate::create_directory_tool(ws, get_str(args, "relative_path")?, confirmed).and_then(to_value),
        "move_rename" => crate::move_rename_tool(
            ws,
            get_str(args, "from_relative_path")?,
            get_str(args, "to_relative_path")?,
            confirmed,
        )
        .and_then(to_value),
        "git_commit" => crate::git_commit_tool(ws, get_str(args, "message")?, confirmed).and_then(to_value),
        "delete_file" => crate::delete_file_tool(
            ws,
            get_str(args, "relative_path")?,
            get_bool(args, "recursive", false),
            confirmed,
        )
        .and_then(to_value),
        "execute_command" => crate::execute_command_tool(ws, get_str(args, "command")?, confirmed).and_then(to_value),
        "restore_snapshot" => crate::restore_snapshot_tool(ws, get_str(args, "commit_hash")?, confirmed).and_then(to_value),
        other => Err(format!("Unknown tool requested by model: '{}'", other)),
    }
}