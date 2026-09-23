mod workspace;
mod tools;

use workspace::Workspace;
use tools::read_file::read_file;
use tools::list_directory::{list_directory, DirEntry};
use tools::write_file::write_file;
use tools::convert_to_pdf::convert_to_pdf;
use tools::edit_file::edit_file;
use tools::delete_file::delete_file;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            read_file_tool,
            list_directory_tool,
            write_file_tool,
            convert_to_pdf_tool,
            edit_file_tool,
            delete_file_tool
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}