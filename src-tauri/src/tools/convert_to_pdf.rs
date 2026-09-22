use crate::workspace::Workspace;
use std::process::Command;

/// Converts a docx file to PDF using LibreOffice headless mode.
/// Requires LibreOffice to be installed on the system.
pub fn convert_to_pdf(workspace: &Workspace, relative_path: &str) -> Result<String, String> {
    let resolved_path = workspace.resolve(relative_path)?; // boundary check first

    let extension = resolved_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if extension != "docx" {
        return Err("convert_to_pdf currently only supports .docx input".to_string());
    }

    let output_dir = resolved_path
        .parent()
        .ok_or("Could not determine output directory")?;

    let soffice_path = r"C:\Program Files\LibreOffice\program\soffice.exe";

    let output = Command::new(soffice_path)
        .arg("--headless")
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(output_dir)
        .arg(&resolved_path)
        .output()
        .map_err(|e| format!("Failed to run LibreOffice: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("LibreOffice conversion failed: {}", stderr));
    }

    let pdf_filename = resolved_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Could not determine output filename")?;

    Ok(format!("{}.pdf", pdf_filename))
}