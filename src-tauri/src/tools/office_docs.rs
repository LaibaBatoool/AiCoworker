//! create_docx / create_xlsx — structured Office document writers (Word: see docx_engine.rs).
//!
//! Safety properties (enforced here):
//!  - every path goes through Workspace::resolve (boundary check) and must have the right extension
//!  - hard caps on blocks / rows / columns / cell length
//!  - XLSX: all text is written as plain strings, NEVER as formulas (blocks formula injection)

use crate::workspace::Workspace;
use rust_xlsxwriter::{Format, Workbook, XlsxError};
use serde_json::Value;
use std::path::Path;

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

/// create_docx lives in docx_engine.rs (same block types as edit_docx's insert).
/// `opts` may hold header, footer and page_numbers.
pub fn create_docx(workspace: &Workspace, relative_path: &str, blocks: &Value, opts: &Value) -> Result<Value, String> {
    crate::tools::docx_engine::create_docx(workspace, relative_path, blocks, opts)
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