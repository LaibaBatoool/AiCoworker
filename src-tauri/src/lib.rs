mod workspace;
mod tools;

use workspace::Workspace;
use tools::read_file::read_file;
use tools::list_directory::{list_directory, DirEntry};
use tools::write_file::write_file;
use tools::convert_to_pdf::convert_to_pdf;
use tools::edit_file::edit_file;
use tools::delete_file::delete_file;
use tools::execute_command::{execute_terminal_command, CommandOutput};
use tools::create_directory::create_directory;
use tools::move_rename::move_rename;
use tools::search_files::{search_files, SearchResult};
use tools::file_metadata::{get_file_metadata, FileMetadata};

#[tauri::command]
fn read_file_tool(workspace_root: String, relative_path: String) -> Result<String, String> {
    let ws = Workspace::new(&workspace_root)?;
    read_file(&ws, &relative_path)
}

#[tauri::command]
fn list_directory_tool(workspace_root: String, relative_path: String) -> Result<Vec<DirEntry>, String> {
    let ws = Workspace::new(&workspace_root)?;
    list_directory(&ws, &relative_path)
}

#[tauri::command]
fn write_file_tool(workspace_root: String, relative_path: String, content: String) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    write_file(&ws, &relative_path, &content)
}

#[tauri::command]
fn convert_to_pdf_tool(workspace_root: String, relative_path: String) -> Result<String, String> {
    let ws = Workspace::new(&workspace_root)?;
    convert_to_pdf(&ws, &relative_path)
}

#[tauri::command]
fn edit_file_tool(
    workspace_root: String,
    relative_path: String,
    old_text: String,
    new_text: String,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    edit_file(&ws, &relative_path, &old_text, &new_text)
}

#[tauri::command]
fn delete_file_tool(
    workspace_root: String,
    relative_path: String,
    recursive: bool,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    delete_file(&ws, &relative_path, recursive)
}

#[tauri::command]
fn execute_command_tool(
    workspace_root: String,
    command: String,
    confirmed: bool,
) -> Result<CommandOutput, String> {
    let ws = Workspace::new(&workspace_root)?;
    execute_terminal_command(&ws, &command, confirmed)
}

#[tauri::command]
fn create_directory_tool(workspace_root: String, relative_path: String) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    create_directory(&ws, &relative_path)
}

#[tauri::command]
fn move_rename_tool(
    workspace_root: String,
    from_relative_path: String,
    to_relative_path: String,
) -> Result<(), String> {
    let ws = Workspace::new(&workspace_root)?;
    move_rename(&ws, &from_relative_path, &to_relative_path)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            read_file_tool,
            list_directory_tool,
            write_file_tool,
            convert_to_pdf_tool,
            edit_file_tool,
            delete_file_tool,
            execute_command_tool,
            create_directory_tool,
            move_rename_tool,
            search_files_tool,
            get_file_metadata_tool
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}