use crate::workspace::Workspace;
use std::fs;
use std::io::Read;
use dotext::{Docx as DotextDocx, MsDoc};
use docx_rs::*;

pub fn edit_file(
    workspace: &Workspace,
    relative_path: &str,
    old_text: &str,
    new_text: &str,
) -> Result<(), String> {
    let resolved_path = workspace.resolve(relative_path)?; // boundary check first, always

    let extension = resolved_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "docx" => edit_docx(&resolved_path, old_text, new_text),
        _ => edit_plain_text(&resolved_path, old_text, new_text),
    }
}

fn edit_plain_text(path: &std::path::Path, old_text: &str, new_text: &str) -> Result<(), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file for editing: {}", e))?;

    let updated = apply_unique_replace(&content, old_text, new_text)?;

    fs::write(path, updated)
        .map_err(|e| format!("Failed to write edited file: {}", e))
}

fn edit_docx(path: &std::path::Path, old_text: &str, new_text: &str) -> Result<(), String> {
    // Step 1: extract existing text (dotext's Docx, for reading)
    let mut file = DotextDocx::open(path)
        .map_err(|e| format!("Failed to open DOCX for editing: {}", e))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| format!("Failed to read DOCX content: {}", e))?;

    // Step 2: apply the find-and-replace on the extracted text
    let updated_content = apply_unique_replace(&content, old_text, new_text)?;

    // Step 3: regenerate the docx from the edited text (docx_rs's Docx, for writing)
    // (Note: this rebuilds the document as plain paragraphs — original
    // formatting/images are not preserved, same limitation as write_file's
    // docx creation.)
    let out_file = fs::File::create(path)
        .map_err(|e| format!("Failed to recreate DOCX file: {}", e))?;

    let mut docx = Docx::new(); // this is docx_rs::Docx, unambiguous now
    for line in updated_content.lines() {
        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(line)));
    }
    if updated_content.is_empty() {
        docx = docx.add_paragraph(Paragraph::new());
    }

    docx.build()
        .pack(out_file)
        .map_err(|e| format!("Failed to write edited DOCX content: {}", e))?;

    Ok(())
}

/// Shared logic: find old_text exactly once in content, replace it with new_text.
/// Errors on zero or multiple matches — same safety principle in both branches.
fn apply_unique_replace(content: &str, old_text: &str, new_text: &str) -> Result<String, String> {
    let match_count = content.matches(old_text).count();

    if match_count == 0 {
        return Err("Could not find the specified text. No changes made.".to_string());
    }

    if match_count > 1 {
        return Err(format!(
            "The specified text appears {} times — edit is ambiguous. \
             Provide more surrounding context to make the match unique.",
            match_count
        ));
    }

    Ok(content.replacen(old_text, new_text, 1))
}