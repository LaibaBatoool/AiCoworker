//! create_docx / create_xlsx — structured Office document writers.
//!
//! Safety properties (enforced here):
//!  - every path goes through Workspace::resolve (boundary check) and must have the right extension
//!  - hard caps on blocks / rows / columns / cell length
//!  - XLSX: all text is written as plain strings, NEVER as formulas (blocks formula injection)

use crate::workspace::Workspace;
use docx_rs::*;
use rust_xlsxwriter::{Format, Workbook, XlsxError};
use serde_json::Value;
use std::fs;
use std::path::Path;

const MAX_BLOCKS: usize = 200;
const MAX_ROWS: usize = 1000;
const MAX_COLS: usize = 50;
const MAX_CELL_CHARS: usize = 5000;

fn clip(s: &str) -> String {
    s.chars().take(MAX_CELL_CHARS).collect()
}

fn val_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => clip(s),
        Value::Null => String::new(),
        other => clip(&other.to_string()),
    }
}

fn require_ext(path: &Path, want: &str) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == want {
        Ok(())
    } else {
        Err(format!("relative_path must end with .{}", want))
    }
}

fn xe(e: XlsxError) -> String {
    format!("XLSX error: {}", e)
}

// ---------------------------------------------------------------- DOCX

pub fn create_docx(workspace: &Workspace, relative_path: &str, blocks: &Value) -> Result<(), String> {
    let path = workspace.resolve(relative_path)?; // boundary check first, always
    require_ext(&path, "docx")?;

    let arr = blocks
        .as_array()
        .ok_or("'blocks' must be an array of {type, ...} objects")?;
    if arr.is_empty() {
        return Err("'blocks' must contain at least one block".to_string());
    }
    if arr.len() > MAX_BLOCKS {
        return Err(format!("Too many blocks (max {})", MAX_BLOCKS));
    }

    let mut docx = Docx::new();

    for (i, b) in arr.iter().enumerate() {
        let kind = b.get("type").and_then(|v| v.as_str()).unwrap_or("paragraph");
        match kind {
            "heading" => {
                let text = b.get("text").map(val_to_text).unwrap_or_default();
                let level = b.get("level").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 3);
                let size: usize = match level {
                    1 => 40,
                    2 => 32,
                    _ => 28,
                };
                docx = docx.add_paragraph(
                    Paragraph::new().add_run(Run::new().add_text(text).bold().size(size)),
                );
            }
            "paragraph" => {
                let text = b.get("text").map(val_to_text).unwrap_or_default();
                docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(text)));
            }
            "table" => {
                let headers = b.get("headers").and_then(|v| v.as_array());
                let rows = b.get("rows").and_then(|v| v.as_array());
                if headers.is_none() && rows.is_none() {
                    return Err(format!("Block {}: a table needs 'headers' and/or 'rows'", i));
                }
                let make_cell = |t: String, bold: bool| {
                    let mut r = Run::new().add_text(t);
                    if bold {
                        r = r.bold();
                    }
                    TableCell::new().add_paragraph(Paragraph::new().add_run(r))
                };

                let mut trs: Vec<TableRow> = Vec::new();
                if let Some(h) = headers {
                    if h.len() > MAX_COLS {
                        return Err(format!("Block {}: too many columns (max {})", i, MAX_COLS));
                    }
                    trs.push(TableRow::new(
                        h.iter().map(|c| make_cell(val_to_text(c), true)).collect(),
                    ));
                }
                if let Some(rs) = rows {
                    if rs.len() > MAX_ROWS {
                        return Err(format!("Block {}: too many rows (max {})", i, MAX_ROWS));
                    }
                    for r in rs {
                        let cells = r
                            .as_array()
                            .ok_or_else(|| format!("Block {}: every table row must be an array", i))?;
                        if cells.len() > MAX_COLS {
                            return Err(format!("Block {}: too many columns (max {})", i, MAX_COLS));
                        }
                        trs.push(TableRow::new(
                            cells.iter().map(|c| make_cell(val_to_text(c), false)).collect(),
                        ));
                    }
                }
                if trs.is_empty() {
                    return Err(format!("Block {}: table is empty", i));
                }
                docx = docx.add_table(Table::new(trs));
                docx = docx.add_paragraph(Paragraph::new()); // spacing after table
            }
            other => {
                return Err(format!(
                    "Block {}: unknown type '{}' (use heading, paragraph or table)",
                    i, other
                ))
            }
        }
    }

    let file = fs::File::create(&path).map_err(|e| format!("Failed to create DOCX file: {}", e))?;
    docx.build()
        .pack(file)
        .map_err(|e| format!("Failed to write DOCX content: {}", e))?;
    Ok(())
}

// ---------------------------------------------------------------- XLSX

pub fn create_xlsx(
    workspace: &Workspace,
    relative_path: &str,
    sheet_name: Option<String>,
    headers: &Value,
    rows: &Value,
) -> Result<(), String> {
    let path = workspace.resolve(relative_path)?; // boundary check first, always
    require_ext(&path, "xlsx")?;

    let header_arr = headers.as_array();
    let row_arr = rows
        .as_array()
        .ok_or("'rows' must be an array of arrays")?;
    if row_arr.len() > MAX_ROWS {
        return Err(format!("Too many rows (max {})", MAX_ROWS));
    }
    if header_arr.map(|h| h.len()).unwrap_or(0) > MAX_COLS {
        return Err(format!("Too many columns (max {})", MAX_COLS));
    }

    let mut wb = Workbook::new();
    let sheet = wb.add_worksheet();

    if let Some(name) = sheet_name {
        let name = name.trim().to_string();
        if !name.is_empty() {
            sheet
                .set_name(&name)
                .map_err(|e| format!("Invalid sheet name: {}", e))?;
        }
    }

    let bold = Format::new().set_bold();
    let mut next_row: u32 = 0;

    if let Some(h) = header_arr {
        for (c, cell) in h.iter().enumerate() {
            sheet
                .write_string_with_format(0, c as u16, val_to_text(cell), &bold)
                .map_err(xe)?;
        }
        next_row = 1;
        sheet.set_freeze_panes(1, 0).map_err(xe)?;
    }

    for (r, row) in row_arr.iter().enumerate() {
        let cells = row
            .as_array()
            .ok_or_else(|| format!("Row {} must be an array", r))?;
        if cells.len() > MAX_COLS {
            return Err(format!("Row {} has too many columns (max {})", r, MAX_COLS));
        }
        let row_idx = next_row + r as u32;
        for (c, cell) in cells.iter().enumerate() {
            let col = c as u16;
            match cell {
                Value::Number(n) => {
                    sheet
                        .write_number(row_idx, col, n.as_f64().unwrap_or(0.0))
                        .map_err(xe)?;
                }
                Value::Bool(b) => {
                    sheet.write_boolean(row_idx, col, *b).map_err(xe)?;
                }
                Value::Null => {}
                // Strings (and anything else) are ALWAYS stored as text, never as a formula.
                other => {
                    sheet
                        .write_string(row_idx, col, val_to_text(other))
                        .map_err(xe)?;
                }
            }
        }
    }

    sheet.autofit();
    wb.save(&path).map_err(xe)?;
    Ok(())
}