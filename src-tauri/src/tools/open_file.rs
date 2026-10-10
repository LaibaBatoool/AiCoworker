//! open_in_default_app — open a workspace file (or folder) in the user's
//! default application, e.g. a PDF in the PDF viewer.
//!
//! Safety properties:
//!  - DEFAULT-DENY extension allowlist: only plain documents, images,
//!    audio/video and folders. Anything that can run code is refused —
//!    .exe .bat .cmd .ps1 .vbs .js .lnk .url .hta .msi .reg, macro-enabled
//!    Office files (.docm .xlsm .pptm), and .html/.svg (can run scripts).
//!  - the check runs BEFORE the permission checkpoint, so a refused file
//!    never even produces a confirmation dialog the user could click through
//!  - path goes through Workspace::resolve (symlinks/junctions resolved first,
//!    then the REAL target's extension is checked)
//!  - ':' is refused (blocks NTFS alternate streams like "x.exe:a.pdf")
//!  - launched via explorer.exe with the path as ONE argument — no shell,
//!    so no `&`/`|` command injection through file names
//!  - AICOWORKER_NO_LAUNCH=1 (set by the benchmark) validates without opening

use crate::workspace::Workspace;
use serde::Serialize;
use std::path::{Path, PathBuf};

const ALLOWED_EXTENSIONS: &[&str] = &[
    // documents
    "pdf", "txt", "md", "csv", "tsv", "json", "log", "rtf",
    "docx", "doc", "xlsx", "xls", "pptx", "ppt", "odt", "ods", "odp",
    // images
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff",
    // audio / video
    "mp3", "wav", "m4a", "flac", "ogg", "mp4", "mkv", "mov", "avi", "webm",
];

#[derive(Serialize, Debug)]
pub struct OpenResult {
    pub opened: String,
    pub kind: String, // "file" | "folder"
    pub launched: bool,
}

/// Validates the path and returns the real target. No side effects.
pub fn check_openable(workspace: &Workspace, relative_path: &str) -> Result<(PathBuf, bool), String> {
    if relative_path.contains(':') {
        return Err("Paths containing ':' are not allowed (drive letters / alternate data streams).".into());
    }
    let path = workspace.resolve(relative_path)?; // follows links, then boundary check
    if !path.exists() {
        return Err(format!("'{}' does not exist.", relative_path));
    }
    if path.is_dir() {
        return Ok((path, true));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        Ok((path, false))
    } else {
        Err(format!(
            "Refused: '.{}' files are not opened automatically because they could run code or are not a plain document. Allowed: documents, images, audio/video and folders.",
            if ext.is_empty() { "(no extension)" } else { &ext }
        ))
    }
}

/// canonicalize() on Windows gives \\?\C:\... which explorer.exe rejects.
fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

pub fn open_in_default_app(workspace: &Workspace, relative_path: &str) -> Result<OpenResult, String> {
    let (path, is_dir) = check_openable(workspace, relative_path)?;
    let kind = if is_dir { "folder" } else { "file" }.to_string();
    if std::env::var("AICOWORKER_NO_LAUNCH").map(|v| v == "1").unwrap_or(false) {
        return Ok(OpenResult { opened: relative_path.to_string(), kind, launched: false });
    }
    let target = display_path(&path);

    #[cfg(windows)]
    let spawned = std::process::Command::new("explorer.exe").arg(&target).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(&target).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(&target).spawn();

    spawned.map_err(|e| format!("Could not open '{}': {}", relative_path, e))?;
    // explorer.exe returns immediately (and often with exit code 1 even on
    // success), so we don't wait on it or judge by its exit code.
    Ok(OpenResult { opened: relative_path.to_string(), kind, launched: true })
}

// ---------------------------------------------------------------
// Tests: `cargo test open_file`  (validation only — nothing is launched)
// ---------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ws(tag: &str) -> (PathBuf, Workspace) {
        let base = std::env::temp_dir().join(format!("aicw_open_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = base.join("ws");
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::write(base.join("outside.pdf"), "x").unwrap();
        let w = Workspace::new(dir.to_str().unwrap()).unwrap();
        (dir, w)
    }

    #[test]
    fn allows_documents_media_and_folders() {
        let (dir, w) = ws("ok");
        for f in ["report.pdf", "Notes.TXT", "photo.jpeg", "budget.xlsx", "talk.mp4"] {
            fs::write(dir.join(f), "x").unwrap();
            assert!(check_openable(&w, f).is_ok(), "should allow {}", f);
        }
        assert_eq!(check_openable(&w, "docs").unwrap().1, true, "folders open in Explorer");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn refuses_anything_that_can_run_code() {
        let (dir, w) = ws("bad");
        for f in [
            "setup.exe", "update.bat", "run.cmd", "x.ps1", "x.vbs", "x.js", "x.jar", "x.msi",
            "x.lnk", "x.url", "x.hta", "x.reg", "x.scr", "macro.xlsm", "macro.docm",
            "page.html", "pic.svg", "invoice.pdf.exe", "README",
        ] {
            fs::write(dir.join(f), "x").unwrap();
            let e = check_openable(&w, f).unwrap_err();
            assert!(e.starts_with("Refused"), "should refuse {}: {}", f, e);
        }
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn refuses_outside_missing_and_streams() {
        let (dir, w) = ws("bounds");
        assert!(check_openable(&w, "../outside.pdf").is_err());
        assert!(check_openable(&w, "missing.pdf").is_err());
        assert!(check_openable(&w, "x.exe:a.pdf").is_err());
        assert!(check_openable(&w, "C:\\Windows\\notepad.exe").is_err());
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn checks_the_real_target_of_a_link() {
        let (dir, w) = ws("link");
        fs::write(dir.join("evil.sh"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("evil.sh"), dir.join("nice.pdf")).unwrap();
        assert!(check_openable(&w, "nice.pdf").is_err(), "a .pdf link to a script must be refused");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn strips_verbatim_prefix_for_explorer() {
        assert_eq!(display_path(Path::new(r"\\?\C:\ws\a.pdf")), r"C:\ws\a.pdf");
        assert_eq!(display_path(Path::new(r"\\?\UNC\srv\share\a.pdf")), r"\\srv\share\a.pdf");
    }
}