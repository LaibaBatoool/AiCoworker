mod workspace;
mod permission;
mod tools;
pub mod agent;
mod running_commands;

use std::sync::Arc;
use tauri::Emitter;
use workspace::Workspace;
use permission::{checkpoint, log_action, read_audit_log, AuditLogRecord, PermissionTier};
use tools::read_file::read_file;
use tools::list_directory::{list_directory, DirEntry};
use tools::write_file::write_file;
use tools::convert_to_pdf::convert_to_pdf;
use tools::edit_file::edit_file;
use tools::delete_file::delete_file;
use tools::execute_command::{
    execute_terminal_command, spawn_terminal_command, wait_for_spawned_command, CommandOutput,
};
use tools::create_directory::create_directory;
use tools::move_rename::move_rename;
use tools::search_files::{search_files, SearchResult};
use tools::file_metadata::{get_file_metadata, FileMetadata};
use tools::git_ops::{git_diff, git_commit, git_status, git_log, GitDiffResult, GitCommitResult, GitStatusResult, GitLogEntry};
use tools::fetch_url::{fetch_url, FetchUrlResult};
use tools::office_docs::{create_docx, create_xlsx};
use tools::archive::{create_archive, extract_archive, CreateArchiveResult, ExtractResult};
use tools::command_classifier::classify_command_risk;
use tools::snapshot::{take_snapshot, list_snapshots, restore_snapshot, SnapshotRecord};
use tools::registry::{all_tool_schemas, ToolSchema};
use running_commands::RunningCommands;

/// Takes a pre-action snapshot. Snapshot failures never block the
/// actual operation (same philosophy as audit logging), but ARE now
/// recorded into the audit log with success=false, so a failure is
/// visible in the "Audit Log" panel instead of only in stderr.
fn snapshot_before(ws: &Workspace, label: &str) {
    if let Err(e) = take_snapshot(ws, label) {
        eprintln!("Warning: failed to take pre-action snapshot: {}", e);
        log_action(
            ws,
            &PermissionTier::Safe,
            &format!("snapshot_before: {}", label),
            false,
            &e,
        );
    }
}

// ---- Safe (read-only) commands: no checkpoint/logging needed ----

#[tauri::command]
fn read_file_tool(workspace_root: String, relative_path: String) -> Result<String, String> {
    let ws = Workspace::new(&workspace_root)?;
    read_file(&ws, &relative_path)
}

#[tauri::command]
fn list_directory_tool(
    workspace_root: String,
    relative_path: String,
    include_metadata: Option<bool>,
) -> Result<Vec<DirEntry>, String> {
    let ws = Workspace::new(&workspace_root)?;
    list_directory(&ws, &relative_path, include_metadata.unwrap_or(false))
}

#[tauri::command]
fn search_files_tool(
    workspace_root: String,
    name_pattern: Option<String>,
    content_pattern: Option<String>,
) -> Result<Vec<SearchResult>, String> {
    let ws = Workspace::new(&workspace_root)?;
    search_files(&ws, name_pattern.as_deref(), content_pattern.as_deref())
}

#[tauri::command]
fn get_file_metadata_tool(workspace_root: String, relative_path: String) -> Result<FileMetadata, String> {
    let ws = Workspace::new(&workspace_root)?;
    get_file_metadata(&ws, &relative_path)
}

#[tauri::command]
fn git_diff_tool(workspace_root: String) -> Result<GitDiffResult, String> {
    let ws = Workspace::new(&workspace_root)?;
    git_diff(&ws)
}

#[tauri::command]
fn git_status_tool(workspace_root: String) -> Result<GitStatusResult, String> {
    let ws = Workspace::new(&workspace_root)?;
    git_status(&ws)
}

#[tauri::command]
fn git_log_tool(workspace_root: String, max_count: u32) -> Result<Vec<GitLogEntry>, String> {
    let ws = Workspace::new(&workspace_root)?;
    git_log(&ws, max_count)
}

#[tauri::command]
fn fetch_url_tool(workspace_root: String, url: String, confirmed: bool) -> Result<FetchUrlResult, String> {
    let ws = Workspace::new(&workspace_root)?;

    // Central permission checkpoint: refuses unconfirmed privileged actions with the
    // standard PRIVILEGED_CONFIRMATION_REQUIRED error that the orchestrator turns into a dialog.
    checkpoint(&PermissionTier::Privileged, &format!("fetch_url: {}", url), confirmed)?;

    match fetch_url(&url) {
        Ok(r) => {
            log_action(&ws, &PermissionTier::Privileged, &format!("fetch_url: {}", url), true, "ok");
            Ok(r)
        }
        Err(e) => {
            log_action(&ws, &PermissionTier::Privileged, &format!("fetch_url: {}", url), false, &e);
            Err(e)
        }
    }
}

#[tauri::command]
fn create_docx_tool(
    workspace_root: String,
    relative_path: String,
    blocks: serde_json::Value,
    confirmed: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("create_docx: {}", relative_path);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = create_docx(&ws, &relative_path, &blocks);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn create_xlsx_tool(
    workspace_root: String,
    relative_path: String,
    sheet_name: Option<String>,
    headers: serde_json::Value,
    rows: serde_json::Value,
    confirmed: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("create_xlsx: {}", relative_path);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = create_xlsx(&ws, &relative_path, sheet_name, &headers, &rows);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn extract_archive_tool(
    workspace_root: String,
    archive_relative_path: String,
    destination_relative_path: Option<String>,
    confirmed: bool,
) -> Result<ExtractResult, String> {
    let ws = Workspace::new(&workspace_root)?;
    let dest = destination_relative_path.unwrap_or_else(|| ".".to_string());
    let action = format!("extract_archive: {} -> {}", archive_relative_path, dest);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = extract_archive(&ws, &archive_relative_path, &dest);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn create_archive_tool(
    workspace_root: String,
    source_relative_paths: Vec<String>,
    archive_relative_path: String,
    confirmed: bool,
) -> Result<CreateArchiveResult, String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("create_archive: {} <- {}", archive_relative_path, source_relative_paths.join(", "));
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = create_archive(&ws, &source_relative_paths, &archive_relative_path);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn get_audit_log_tool(workspace_root: String) -> Result<Vec<AuditLogRecord>, String> {
    let ws = Workspace::new(&workspace_root)?;
    read_audit_log(&ws)
}

#[tauri::command]
fn list_snapshots_tool(workspace_root: String) -> Result<Vec<SnapshotRecord>, String> {
    let ws = Workspace::new(&workspace_root)?;
    list_snapshots(&ws)
}

#[tauri::command]
fn list_tool_schemas_tool() -> Vec<ToolSchema> {
    all_tool_schemas()
}

// ---- Mutating commands: checkpoint (always passes) + always logged ----

#[tauri::command]
fn write_file_tool(
    workspace_root: String,
    relative_path: String,
    content: String,
    confirmed: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("write_file: {}", relative_path);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = write_file(&ws, &relative_path, &content);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn convert_to_pdf_tool(
    workspace_root: String,
    relative_path: String,
    confirmed: bool,
) -> Result<String, String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("convert_to_pdf: {}", relative_path);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = convert_to_pdf(&ws, &relative_path);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn edit_file_tool(
    workspace_root: String,
    relative_path: String,
    old_text: String,
    new_text: String,
    confirmed: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("edit_file: {}", relative_path);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = edit_file(&ws, &relative_path, &old_text, &new_text);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn create_directory_tool(
    workspace_root: String,
    relative_path: String,
    confirmed: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("create_directory: {}", relative_path);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = create_directory(&ws, &relative_path);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn move_rename_tool(
    workspace_root: String,
    from_relative_path: String,
    to_relative_path: String,
    confirmed: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("move_rename: {} -> {}", from_relative_path, to_relative_path);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = move_rename(&ws, &from_relative_path, &to_relative_path);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn git_commit_tool(
    workspace_root: String,
    message: String,
    confirmed: bool,
) -> Result<GitCommitResult, String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("git_commit: {}", message);
    checkpoint(&PermissionTier::Mutating, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = git_commit(&ws, &message);
    log_action(&ws, &PermissionTier::Mutating, &action, result.is_ok(), &format!("{:?}", result));
    result
}

// ---- Privileged commands: checkpoint BLOCKS until confirmed=true ----

#[tauri::command]
fn delete_file_tool(
    workspace_root: String,
    relative_path: String,
    recursive: bool,
    confirmed: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("delete_file: {} (recursive={})", relative_path, recursive);
    checkpoint(&PermissionTier::Privileged, &action, confirmed)?;
    snapshot_before(&ws, &action);

    let result = delete_file(&ws, &relative_path, recursive);
    log_action(&ws, &PermissionTier::Privileged, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn execute_command_tool(
    workspace_root: String,
    command: String,
    confirmed: bool,
) -> Result<CommandOutput, String> {
    let ws = Workspace::new(&workspace_root)?;
    let tier = classify_command_risk(&command);
    let action = format!("execute_command: {}", command);
    checkpoint(&tier, &action, confirmed)?;
    if tier != PermissionTier::Safe {
        snapshot_before(&ws, &action);
    }

    let result = execute_terminal_command(&ws, &command);
    log_action(&ws, &tier, &action, result.is_ok(), &format!("{:?}", result));
    result
}

#[tauri::command]
fn restore_snapshot_tool(
    workspace_root: String,
    commit_hash: String,
    confirmed: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    let action = format!("restore_snapshot: {}", commit_hash);
    checkpoint(&PermissionTier::Privileged, &action, confirmed)?;

    let result = restore_snapshot(&ws, &commit_hash);
    log_action(&ws, &PermissionTier::Privileged, &action, result.is_ok(), &format!("{:?}", result));
    result
}

// ---- Cancellable command execution (manual UI panel only — the
//      agent loop keeps using execute_command_tool above, untouched)

#[tauri::command]
fn start_command_tool(
    app: tauri::AppHandle,
    running: tauri::State<Arc<RunningCommands>>,
    workspace_root: String,
    command: String,
    confirmed: bool,
) -> Result<u64, String> {
    let ws = Workspace::new(&workspace_root)?;
    let tier = classify_command_risk(&command);
    let action = format!("execute_command: {}", command);
    checkpoint(&tier, &action, confirmed)?; // still blocks synchronously for privileged commands
    if tier != PermissionTier::Safe {
        snapshot_before(&ws, &action);
    }

    let (child, risk_level) = spawn_terminal_command(&ws, &command)?;
    let (job_id, cancelled_flag) = running.register(Arc::clone(&child));

    let running_for_thread = Arc::clone(running.inner());
    let workspace_root_for_log = workspace_root.clone();
    let command_for_log = command.clone();
    let action_for_log = action.clone();
    let app_for_emit = app.clone();

    std::thread::spawn(move || {
        let output = wait_for_spawned_command(child, cancelled_flag, risk_level);

        if let Ok(log_ws) = Workspace::new(&workspace_root_for_log) {
            let tier_for_log = classify_command_risk(&command_for_log);
            log_action(
                &log_ws,
                &tier_for_log,
                &action_for_log,
                output.exit_code == Some(0),
                &format!("{:?}", output),
            );
        }

        running_for_thread.remove(job_id);

        let _ = app_for_emit.emit(
            "command-finished",
            serde_json::json!({ "jobId": job_id, "output": output }),
        );
    });

    Ok(job_id)
}

#[tauri::command]
fn cancel_command_tool(running: tauri::State<Arc<RunningCommands>>, job_id: u64) -> Result<(), String> {
    running.cancel(job_id)
}

// ---- Agent orchestration entry point ----

#[tauri::command]
async fn run_agent_tool(
    workspace_root: String,
    api_key: String,
    model: String,
    messages_json: String,
    resume_confirmed: bool,
) -> Result<agent::AgentStepResult, String> {
    let messages: Vec<agent::model_client::ChatMessage> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("messages_json was not valid: {}", e))?;

    let client = agent::model_client::OpenAiCompatibleClient::groq(api_key, model);
    let outcome = agent::orchestrator::run_agent_loop(&client, &workspace_root, messages, resume_confirmed).await;

    Ok(match outcome {
        agent::orchestrator::LoopOutcome::Done { messages, answer } => agent::AgentStepResult::Done { messages, answer },
        agent::orchestrator::LoopOutcome::AwaitingConfirmation { messages, tool_name, arguments } => {
            agent::AgentStepResult::AwaitingConfirmation { messages, tool_name, arguments }
        }
        agent::orchestrator::LoopOutcome::StoppedForSafety { messages, reason } => {
            agent::AgentStepResult::StoppedForSafety { messages, reason }
        }
        agent::orchestrator::LoopOutcome::Error(message) => agent::AgentStepResult::Error { message },
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    dotenvy::dotenv().ok(); // loads src-tauri/.env if present; harmless if it's missing

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(RunningCommands::default()))
        .invoke_handler(tauri::generate_handler![
            read_file_tool,
            list_directory_tool,
            write_file_tool,
            convert_to_pdf_tool,
            edit_file_tool,
            delete_file_tool,
            execute_command_tool,
            start_command_tool,
            cancel_command_tool,
            create_directory_tool,
            move_rename_tool,
            search_files_tool,
            get_file_metadata_tool,
            git_diff_tool,
            git_status_tool,
            git_log_tool,
            fetch_url_tool,
            create_docx_tool,
            create_xlsx_tool,
            extract_archive_tool,
            create_archive_tool,
            git_commit_tool,
            get_audit_log_tool,
            list_snapshots_tool,
            restore_snapshot_tool,
            list_tool_schemas_tool,
            run_agent_tool
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}