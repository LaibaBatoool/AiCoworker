//! read_spreadsheet — read rows from .xlsx / .xlsm / .xls / .ods / .csv / .tsv
//!
//! Safe (read-only) tool. Safety/size properties:
//!  - path goes through Workspace::resolve (boundary check)
//!  - formulas are NEVER returned or evaluated: only the value Excel last
//!    saved for each cell (calamine reads cached values)
//!  - paging: at most MAX_ROWS_PER_CALL rows per call (start_row/max_rows),
//!    each cell clipped to MAX_CELL_CHARS, file size capped
//!  - cell text is untrusted data (the tool description says so)

use crate::workspace::Workspace;
use calamine::{open_workbook_auto, Data, Reader};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;

const DEFAULT_ROWS: usize = 100;
const MAX_ROWS_PER_CALL: usize = 500;
const MAX_CELL_CHARS: usize = 300;
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Serialize, Debug)]
pub struct SpreadsheetData {
    /// Sheet these rows came from ("" for CSV/TSV).
    pub sheet: String,
    /// Every sheet in the workbook (so the model can ask for another one).
    pub sheets: Vec<String>,
    /// Rows/columns in the sheet's used area.
    pub total_rows: usize,
    pub total_columns: usize,
    /// 1-based row number (within the used area) of rows[0].
    pub start_row: usize,
    /// Excel address of the used area's top-left cell, e.g. "A1" or "B3".
    pub first_cell: String,
    pub rows: Vec<Vec<Value>>,
    /// true if more rows exist after the ones returned.
    pub truncated: bool,
}

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_CELL_CHARS {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(MAX_CELL_CHARS).collect();
        t.push('…');
        t
    }
}

/// 0 -> "A", 25 -> "Z", 26 -> "AA"
fn col_letters(mut c: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push((b'A' + (c % 26) as u8) as char);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.iter().rev().collect()
}

fn cell_to_json(d: &Data) -> Value {
    match d {
        Data::Empty => Value::Null,
        Data::Int(i) => json!(i),
        Data::Float(f) if f.is_finite() => {
            if f.fract() == 0.0 && f.abs() < 9.0e15 {
                json!(*f as i64) // 120.0 -> 120, easier for the model
            } else {
                json!(f)
            }
        }
        Data::Float(f) => json!(f.to_string()),
        Data::String(s) => json!(clip(s)),
        Data::Bool(b) => json!(b),
        Data::DateTime(dt) if dt.is_datetime() => json!(excel_serial_to_iso(dt.as_f64())),
        Data::DateTime(dt) => json!(dt.as_f64()), // a duration: leave as a number of days
        Data::DateTimeIso(s) | Data::DurationIso(s) => json!(clip(s)),
        Data::Error(e) => json!(format!("#ERROR {:?}", e)),
    }
}

/// Excel (1900 date system) serial number -> "YYYY-MM-DD" or
/// "YYYY-MM-DD HH:MM:SS". Day 0 is 1899-12-30 (this also absorbs
/// Excel's famous fake 1900-02-29 for every date after Feb 1900).
fn excel_serial_to_iso(serial: f64) -> String {
    if !serial.is_finite() || serial < 1.0 {
        return serial.to_string();
    }
    let days = serial.floor() as i64;
    let mut secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    let days = days + secs / 86_400;
    secs %= 86_400;
    // days since 1970-01-01, then Howard Hinnant's civil_from_days
    let z = days - 25_569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    if secs == 0 {
        format!("{:04}-{:02}-{:02}", y, m, d)
    } else {
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, secs / 3600, secs % 3600 / 60, secs % 60)
    }
}

/// Small RFC 4180 CSV parser: quoted fields, "" escapes, newlines inside quotes.
fn parse_delimited(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            in_quotes = true;
        } else if c == delim {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// CSV cells come back as numbers when they clearly are numbers ("120",
/// "-3.5"), otherwise as text. Leading zeros ("007") stay text.
fn csv_cell(s: &str) -> Value {
    let t = s.trim();
    if t.is_empty() {
        return Value::Null;
    }
    let looks_numeric = !(t.len() > 1 && t.starts_with('0') && !t.starts_with("0."));
    if looks_numeric {
        if let Ok(i) = t.parse::<i64>() {
            return json!(i);
        }
        if let Ok(f) = t.parse::<f64>() {
            if f.is_finite() {
                return json!(f);
            }
        }
    }
    json!(clip(s))
}

pub fn read_spreadsheet(
    workspace: &Workspace,
    relative_path: &str,
    sheet_name: Option<&str>,
    start_row: Option<usize>,
    max_rows: Option<usize>,
) -> Result<SpreadsheetData, String> {
    let path = workspace.resolve(relative_path)?; // boundary check
    let meta = fs::metadata(&path).map_err(|_| format!("'{}' does not exist.", relative_path))?;
    if !meta.is_file() {
        return Err(format!("'{}' is not a file.", relative_path));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("File is larger than {} MB.", MAX_FILE_BYTES / (1024 * 1024)));
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let start = start_row.unwrap_or(1).max(1);
    let limit = max_rows.unwrap_or(DEFAULT_ROWS).clamp(1, MAX_ROWS_PER_CALL);

    // Collect the whole used area as JSON cells, then page it.
    let (sheet, sheets, all_rows, first_cell): (String, Vec<String>, Vec<Vec<Value>>, String) = match ext.as_str() {
        "csv" | "tsv" => {
            let bytes = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
            let text = String::from_utf8_lossy(&bytes);
            let text = text.trim_start_matches('\u{feff}'); // Excel's UTF-8 BOM
            let delim = if ext == "tsv" {
                '\t'
            } else {
                // Excel in many locales writes ';' — pick whatever the header uses most
                let first = text.lines().next().unwrap_or("");
                let (c, s, t) = (first.matches(',').count(), first.matches(';').count(), first.matches('\t').count());
                if s > c && s >= t { ';' } else if t > c { '\t' } else { ',' }
            };
            let rows = parse_delimited(text, delim)
                .into_iter()
                .map(|r| r.iter().map(|c| csv_cell(c)).collect())
                .collect();
            (String::new(), Vec::new(), rows, "A1".to_string())
        }
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => {
            let mut wb = open_workbook_auto(&path).map_err(|e| format!("Could not open spreadsheet: {}", e))?;
            let names = wb.sheet_names().to_vec();
            if names.is_empty() {
                return Err("The workbook has no sheets.".into());
            }
            let chosen = match sheet_name {
                Some(want) => names
                    .iter()
                    .find(|n| n.eq_ignore_ascii_case(want.trim()))
                    .cloned()
                    .ok_or_else(|| format!("No sheet named '{}'. Sheets: {}", want, names.join(", ")))?,
                None => names[0].clone(),
            };
            let range = wb
                .worksheet_range(&chosen)
                .map_err(|e| format!("Could not read sheet '{}': {}", chosen, e))?;
            let first = match range.start() {
                Some((r, c)) => format!("{}{}", col_letters(c as usize), r + 1),
                None => "A1".to_string(),
            };
            let rows = range.rows().map(|r| r.iter().map(cell_to_json).collect()).collect();
            (chosen, names, rows, first)
        }
        other => {
            return Err(format!(
                "Unsupported file type '.{}'. Supported: .xlsx .xlsm .xlsb .xls .ods .csv .tsv",
                other
            ))
        }
    };

    let total_rows = all_rows.len();
    let total_columns = all_rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let page: Vec<Vec<Value>> = all_rows.into_iter().skip(start - 1).take(limit).collect();
    let truncated = start - 1 + page.len() < total_rows;

    Ok(SpreadsheetData {
        sheet,
        sheets,
        total_rows,
        total_columns,
        start_row: start,
        first_cell,
        rows: page,
        truncated,
    })
}

// ---------------------------------------------------------------
// Tests: `cargo test read_spreadsheet`
// ---------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ws(tag: &str) -> (PathBuf, Workspace) {
        let base = std::env::temp_dir().join(format!("aicw_sheet_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = base.join("ws");
        fs::create_dir_all(&dir).unwrap();
        fs::write(base.join("secret.csv"), "a,b\n1,2\n").unwrap();
        let w = Workspace::new(dir.to_str().unwrap()).unwrap();
        (dir, w)
    }

    fn make_xlsx(path: &std::path::Path) {
        use rust_xlsxwriter::{Formula, Workbook};
        let mut wb = Workbook::new();
        let s1 = wb.add_worksheet();
        s1.set_name("Q1").unwrap();
        s1.write_string(0, 0, "item").unwrap();
        s1.write_string(0, 1, "amount").unwrap();
        s1.write_string(1, 0, "widget").unwrap();
        s1.write_number(1, 1, 120).unwrap();
        let date_fmt = rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd");
        s1.write_string(0, 2, "date").unwrap();
        s1.write_number_with_format(1, 2, 45_658.0, &date_fmt).unwrap();
        let s2 = wb.add_worksheet();
        s2.set_name("Q2").unwrap();
        s2.write_string(0, 0, "item").unwrap();
        s2.write_string(0, 1, "amount").unwrap();
        for i in 0..250u32 {
            s2.write_string(i + 1, 0, &format!("row{}", i + 1)).unwrap();
            s2.write_number(i + 1, 1, (i + 1) as f64).unwrap();
        }
        // a formula with a cached result: we must return the VALUE, never the formula
        s2.write_formula(251, 1, Formula::new("=SUM(B2:B251)").set_result("31375")).unwrap();
        wb.save(path).unwrap();
    }

    #[test]
    fn reads_xlsx_sheets_and_pages() {
        let (dir, w) = ws("xlsx");
        make_xlsx(&dir.join("sales.xlsx"));

        let first = read_spreadsheet(&w, "sales.xlsx", None, None, None).unwrap();
        assert_eq!(first.sheet, "Q1");
        assert_eq!(first.sheets, vec!["Q1", "Q2"]);
        assert_eq!(first.rows[1], vec![json!("widget"), json!(120), json!("2025-01-01")]);
        assert!(!first.truncated);

        let q2 = read_spreadsheet(&w, "sales.xlsx", Some("q2"), None, None).unwrap();
        assert_eq!(q2.total_rows, 252);
        assert_eq!(q2.rows.len(), DEFAULT_ROWS);
        assert!(q2.truncated);

        let last = read_spreadsheet(&w, "sales.xlsx", Some("Q2"), Some(251), Some(10)).unwrap();
        assert_eq!(last.rows.len(), 2);
        assert_eq!(last.rows[1][1], json!(31375), "formula cell returns its cached value");
        assert!(!last.truncated);
        assert!(!serde_json::to_string(&last).unwrap().contains("SUM("), "formula text must never be returned");

        let err = read_spreadsheet(&w, "sales.xlsx", Some("Q9"), None, None).unwrap_err();
        assert!(err.contains("Q1") && err.contains("Q2"), "error should list real sheet names");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn reads_csv_with_quotes_and_numbers() {
        let (dir, w) = ws("csv");
        fs::write(
            dir.join("data.csv"),
            "\u{feff}name,qty,code\n\"Smith, John\",10,007\n\"say \"\"hi\"\"\",2.5,x\n\"multi\nline\",,=cmd|' /C calc'!A0\n",
        )
        .unwrap();
        let d = read_spreadsheet(&w, "data.csv", None, None, None).unwrap();
        assert_eq!(d.total_rows, 4);
        assert_eq!(d.rows[0], vec![json!("name"), json!("qty"), json!("code")]);
        assert_eq!(d.rows[1], vec![json!("Smith, John"), json!(10), json!("007")]);
        assert_eq!(d.rows[2][0], json!("say \"hi\""));
        assert_eq!(d.rows[2][1], json!(2.5));
        assert_eq!(d.rows[3][0], json!("multi\nline"));
        assert_eq!(d.rows[3][1], Value::Null);
        assert_eq!(d.rows[3][2], json!("=cmd|' /C calc'!A0"), "formula-looking text is just text");

        fs::write(dir.join("semi.csv"), "a;b\n1;2\n").unwrap();
        let s = read_spreadsheet(&w, "semi.csv", None, None, None).unwrap();
        assert_eq!(s.rows[1], vec![json!(1), json!(2)], "semicolon CSV detected");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn stays_inside_workspace_and_rejects_other_types() {
        let (dir, w) = ws("bounds");

        let err = read_spreadsheet(&w, "../secret.csv", None, None, None).unwrap_err();
        assert!(err.contains("outside the workspace boundary"));

        fs::write(dir.join("notes.txt"), "not a spreadsheet").unwrap();
        let err = read_spreadsheet(&w, "notes.txt", None, None, None).unwrap_err();
        assert!(err.contains("Unsupported file type"));

        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

        #[test]
    fn excel_dates() {
        assert_eq!(excel_serial_to_iso(45_658.0), "2025-01-01");
        assert_eq!(excel_serial_to_iso(61.0), "1900-03-01");
        assert_eq!(excel_serial_to_iso(45_658.5), "2025-01-01 12:00:00");
        assert_eq!(excel_serial_to_iso(46_303.0), "2026-10-08");
    }

    #[test]
    fn column_letters() {
        assert_eq!(col_letters(0), "A");
        assert_eq!(col_letters(25), "Z");
        assert_eq!(col_letters(26), "AA");
        assert_eq!(col_letters(701), "ZZ");
        assert_eq!(col_letters(702), "AAA");
    }
}