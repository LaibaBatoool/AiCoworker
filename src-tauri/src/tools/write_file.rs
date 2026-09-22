use crate::workspace::Workspace;
use std::fs;
use docx_rs::*;

/// Writes content to a file, dispatching based on extension.
/// Plain text extensions get a raw write; .docx gets a properly
/// constructed Word document (one paragraph per line of content).
pub fn write_file(workspace: &Workspace, relative_path: &str, content: &str) -> Result<(), String> {
    let resolved_path = workspace.resolve(relative_path)?; // boundary check first, always

    let extension = resolved_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "docx" => write_docx(&resolved_path, content),
        // default: plain text write (.txt, .md, .json, source files, etc.)
        _ => fs::write(&resolved_path, content)
            .map_err(|e| format!("Failed to write file: {}", e)),
    }
}

fn write_docx(path: &std::path::Path, content: &str) -> Result<(), String> {
    let file = fs::File::create(path)
        .map_err(|e| format!("Failed to create DOCX file: {}", e))?;

    let mut docx = Docx::new();

    // Split content into paragraphs by line, so multi-line text
    // becomes multiple paragraphs rather than one giant blob.
    for line in content.lines() {
        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(line)));
    }

    // Handle the case of empty content — still produce a valid (empty) docx
    if content.is_empty() {
        docx = docx.add_paragraph(Paragraph::new());
    }

    docx.build()
        .pack(file)
        .map_err(|e| format!("Failed to write DOCX content: {}", e))?;

    Ok(())
}