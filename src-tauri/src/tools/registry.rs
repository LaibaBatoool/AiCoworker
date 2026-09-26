use serde::Serialize;
use serde_json::{json, Value};

/// Describes one tool the agent can call, in a shape that maps
/// directly onto Ollama's / OpenAI-style function-calling `tools`
/// array: { name, description, parameters: <JSON Schema> }.
///
/// IMPORTANT: `tier` here is informational only — for the UI to show
/// a badge, and later for the orchestrator to decide when to pause
/// and ask the user before even sending a privileged call to the
/// model's confirmation step. It is NOT the source of truth for
/// enforcement. The backend re-derives and enforces the real tier
/// independently inside each `_tool` command via `checkpoint()` (and,
/// for execute_command, via `classify_command_risk`), so a model
/// hallucinating or lying about risk can never bypass anything.
#[derive(Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub tier: String,
    pub parameters: Value,
}

fn tool(name: &str, description: &str, tier: &str, parameters: Value) -> ToolSchema {
    ToolSchema {
        name: name.to_string(),
        description: description.to_string(),
        tier: tier.to_string(),
        parameters,
    }
}

/// The full set of tools exposed to the agent loop.
///
/// Deliberately NOT included as parameters here: `workspace_root`
/// and `confirmed`. Those are injected by the orchestrator itself
/// (workspace_root is fixed per session; confirmed is set to true
/// only after the user actually approves a privileged/mutating
/// action) — the model never gets to choose either one directly.
pub fn all_tool_schemas() -> Vec<ToolSchema> {
    vec![
        tool(
            "read_file",
            "Read the full text content of a file in the workspace.",
            "safe",
            json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string", "description": "Path relative to the workspace root, e.g. 'notes.txt' or 'src/App.tsx'." }
                },
                "required": ["relative_path"]
            }),
        ),
        tool(
            "list_directory",
            "List the files and subfolders directly inside a directory.",
            "safe",
            json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string", "description": "Directory path relative to the workspace root. Use '.' for the workspace root itself." }
                },
                "required": ["relative_path"]
            }),
        ),
        tool(
            "search_files",
            "Search the workspace for files by filename pattern and/or by text content, up to 50 results.",
            "safe",
            json!({
                "type": "object",
                "properties": {
                    "name_pattern": { "type": "string", "description": "Substring to match against file names. Omit to skip name filtering." },
                    "content_pattern": { "type": "string", "description": "Text to search for inside files. Omit to skip content search." }
                },
                "required": []
            }),
        ),
        tool(
            "get_file_metadata",
            "Get size, type, last-modified time, and read-only status for a file or folder.",
            "safe",
            json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string", "description": "Path relative to the workspace root." }
                },
                "required": ["relative_path"]
            }),
        ),
        tool(
            "git_diff",
            "Show the current uncommitted changes (diff) in the workspace's own git repository.",
            "safe",
            json!({ "type": "object", "properties": {}, "required": [] }),
        ),
        tool(
            "list_snapshots",
            "List the agent's own auto-generated undo snapshots for this workspace, most recent first.",
            "safe",
            json!({ "type": "object", "properties": {}, "required": [] }),
        ),
        tool(
            "write_file",
            "Create a new file or overwrite an existing file's entire content.",
            "mutating",
            json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string", "description": "Path relative to the workspace root." },
                    "content": { "type": "string", "description": "The full content the file should contain after this call." }
                },
                "required": ["relative_path", "content"]
            }),
        ),
        tool(
            "edit_file",
            "Replace one exact, unique occurrence of text inside an existing file.",
            "mutating",
            json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string" },
                    "old_text": { "type": "string", "description": "Exact text to find. Must match exactly once in the file." },
                    "new_text": { "type": "string", "description": "Text to replace it with." }
                },
                "required": ["relative_path", "old_text", "new_text"]
            }),
        ),
        tool(
            "convert_to_pdf",
            "Convert a .docx file in the workspace to a .pdf file of the same name.",
            "mutating",
            json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string", "description": "Path to the .docx file to convert." }
                },
                "required": ["relative_path"]
            }),
        ),
        tool(
            "create_directory",
            "Create a new, possibly nested, directory in the workspace.",
            "mutating",
            json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string" }
                },
                "required": ["relative_path"]
            }),
        ),
        tool(
            "move_rename",
            "Move or rename a file or folder within the workspace.",
            "mutating",
            json!({
                "type": "object",
                "properties": {
                    "from_relative_path": { "type": "string" },
                    "to_relative_path": { "type": "string" }
                },
                "required": ["from_relative_path", "to_relative_path"]
            }),
        ),
        tool(
            "git_commit",
            "Stage every change in the workspace and commit it to the workspace's own git repository.",
            "mutating",
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "The commit message." }
                },
                "required": ["message"]
            }),
        ),
        tool(
            "delete_file",
            "Permanently delete a file or folder. Irreversible outside of the undo-snapshot system.",
            "privileged",
            json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string" },
                    "recursive": { "type": "boolean", "description": "Must be true to delete a non-empty folder." }
                },
                "required": ["relative_path", "recursive"]
            }),
        ),
        tool(
            "execute_command",
            "Run a shell command in the workspace directory. Actual risk tier is computed server-side by classify_command_risk and may require confirmation regardless of what is requested here.",
            "privileged",
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The full shell command to run, e.g. 'npm test'." }
                },
                "required": ["command"]
            }),
        ),
        tool(
            "restore_snapshot",
            "Roll the entire workspace back to a previous undo snapshot. Discards everything done since that snapshot.",
            "privileged",
            json!({
                "type": "object",
                "properties": {
                    "commit_hash": { "type": "string", "description": "The snapshot's commit hash, from list_snapshots." }
                },
                "required": ["commit_hash"]
            }),
        ),
    ]
}