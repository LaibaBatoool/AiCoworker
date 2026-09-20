mod workspace;
mod tools;

use workspace::Workspace;
use tools::read_file::read_file;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn read_file_tool(workspace_root: String, relative_path: String) -> Result<String, String> {
    let ws = Workspace::new(&workspace_root)?;
    read_file(&ws, &relative_path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![read_file_tool])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}