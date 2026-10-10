//! extract_archive / create_archive — .zip handling inside the workspace.
//!
//! Safety properties (enforced here, not by the model):
//!  - ZIP-SLIP: every entry name is checked BEFORE anything is written.
//!    Names with `..`, absolute paths, drive letters (`C:`), UNC/root
//!    paths, NTFS alternate streams (`a.txt:evil`) or embedded NULs make
//!    the WHOLE archive be rejected — nothing is extracted at all.
//!  - symlink entries inside a zip are rejected (they could point anywhere).
//!  - every final path is re-checked against the destination folder after
//!    its parent folder is created (catches pre-existing links/junctions).
//!  - ZIP-BOMB caps: max entries, and a max on the bytes ACTUALLY written
//!    (we never trust the sizes the archive claims about itself).
//!  - never overwrites an existing file (all conflicts reported up front).
//!  - create_archive never follows symlinks/junctions and skips the
//!    internal .aicoworker folder (snapshots + audit log).

use crate::workspace::Workspace;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MAX_ENTRIES: usize = 10_000;
const MAX_TOTAL_BYTES: u64 = 500 * 1024 * 1024; // 500 MB written in total

#[derive(Serialize, Debug)]
pub struct ExtractResult {
    pub destination: String,
    pub files_extracted: usize,
    pub folders_created: usize,
    pub total_bytes: u64,
}

#[derive(Serialize, Debug)]
pub struct CreateArchiveResult {
    pub archive: String,
    pub files_added: usize,
    pub skipped: Vec<String>,
    pub total_bytes: u64,
}

fn require_zip_ext(path: &Path) -> Result<(), String> {
    let ok = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err("Only .zip archives are supported.".to_string())
    }
}

/// Turns an entry name from inside the zip into a SAFE relative path, or
/// explains why it's unsafe. Checked on the raw name ourselves (both `/`
/// and `\` count as separators, because Windows treats both as one).
fn safe_entry_path(raw_name: &str) -> Result<PathBuf, String> {
    let name = raw_name.replace('\\', "/");
    if name.is_empty() {
        return Err("empty entry name".into());
    }
    if name.contains('\0') {
        return Err("entry name contains a NUL byte".into());
    }
    if name.starts_with('/') {
        return Err("absolute path".into());
    }
    if name.contains(':') {
        // C:\x (drive), or a.txt:stream (NTFS alternate data stream)
        return Err("contains ':' (drive letter or alternate data stream)".into());
    }
    let mut out = PathBuf::new();
    for part in name.split('/') {
        match part {
            "" | "." => continue, // "a//b" or "./a" — harmless
            p if p.chars().all(|c| c == '.') => {
                // "..", "...", etc. Windows trims trailing dots, so any
                // all-dots component is treated as a parent reference.
                return Err("parent-directory reference (..)".into());
            }
            p if p.ends_with(' ') || p.ends_with('.') => {
                // Windows silently strips trailing dots/spaces, so "evil. "
                // and "evil" collide — reject instead of guessing.
                return Err("component ends with a dot or space".into());
            }
            p => out.push(p),
        }
    }
    // Final belt-and-braces check on what PathBuf actually built.
    if out.as_os_str().is_empty() || out.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("not a plain relative path".into());
    }
    Ok(out)
}

/// One entry, validated in pass 1.
struct Planned {
    index: usize,
    rel: PathBuf,
    is_dir: bool,
}

pub fn extract_archive(
    workspace: &Workspace,
    archive_relative_path: &str,
    destination_relative_path: &str,
) -> Result<ExtractResult, String> {
    extract_with_limits(workspace, archive_relative_path, destination_relative_path, MAX_ENTRIES, MAX_TOTAL_BYTES)
}

fn extract_with_limits(
    workspace: &Workspace,
    archive_relative_path: &str,
    destination_relative_path: &str,
    max_entries: usize,
    max_total_bytes: u64,
) -> Result<ExtractResult, String> {
    let archive_path = workspace.resolve(archive_relative_path)?; // boundary check
    require_zip_ext(&archive_path)?;
    if !archive_path.is_file() {
        return Err(format!("'{}' does not exist or is not a file.", archive_relative_path));
    }

    // Destination: "." / "" = workspace root. Created if missing (one level).
    let dest_rel = if destination_relative_path.trim().is_empty() { "." } else { destination_relative_path };
    let dest = workspace.resolve(dest_rel)?;
    if !dest.exists() {
        fs::create_dir(&dest).map_err(|e| format!("Failed to create destination folder: {}", e))?;
    }
    if !dest.is_dir() {
        return Err(format!("Destination '{}' is not a folder.", dest_rel));
    }
    let dest = dest.canonicalize().map_err(|e| format!("Failed to resolve destination: {}", e))?;
    if !dest.starts_with(workspace.root()) {
        return Err("Destination is outside the workspace boundary.".into());
    }

    let file = File::open(&archive_path).map_err(|e| format!("Failed to open archive: {}", e))?;
    let mut zip = ZipArchive::new(file).map_err(|e| format!("Not a valid .zip archive: {}", e))?;
    if zip.len() > max_entries {
        return Err(format!("Archive has {} entries; the limit is {}.", zip.len(), max_entries));
    }

    // ---- pass 1: validate EVERYTHING before writing anything
    let mut plan: Vec<Planned> = Vec::with_capacity(zip.len());
    let mut problems: Vec<String> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| format!("Failed to read archive entry {}: {}", i, e))?;
        let raw = entry.name().to_string();
        if entry.is_symlink() {
            problems.push(format!("'{}': symbolic link entries are not allowed", raw));
            continue;
        }
        match safe_entry_path(&raw) {
            Ok(rel) => {
                let target = dest.join(&rel);
                if !entry.is_dir() && target.exists() {
                    conflicts.push(rel.to_string_lossy().to_string());
                }
                plan.push(Planned { index: i, rel, is_dir: entry.is_dir() });
            }
            Err(why) => problems.push(format!("'{}': {}", raw, why)),
        }
    }
    if !problems.is_empty() {
        return Err(format!(
            "Archive REJECTED, nothing was extracted. Unsafe entries (possible zip-slip attack): {}",
            problems.join("; ")
        ));
    }
    if !conflicts.is_empty() {
        return Err(format!(
            "Nothing was extracted: these files already exist in the destination and would be overwritten: {}. Extract into a new folder instead.",
            conflicts.join(", ")
        ));
    }

    // ---- pass 2: extract, counting the bytes we actually write
    let mut files = 0usize;
    let mut folders = 0usize;
    let mut total: u64 = 0;
    for p in &plan {
        let target = dest.join(&p.rel);
        let parent = if p.is_dir { target.clone() } else { target.parent().unwrap_or(&dest).to_path_buf() };
        if !parent.exists() {
            fs::create_dir_all(&parent).map_err(|e| format!("Failed to create folder: {}", e))?;
            folders += 1;
        }
        // Re-check after creating folders: a pre-existing junction/symlink
        // inside the destination must not redirect us outside it.
        let real_parent = parent.canonicalize().map_err(|e| format!("Failed to resolve folder: {}", e))?;
        if !real_parent.starts_with(&dest) {
            return Err(format!(
                "Stopped: '{}' would be written outside the destination (through a link). Files extracted before this point: {}.",
                p.rel.display(), files
            ));
        }
        if p.is_dir {
            continue;
        }

        let mut entry = zip.by_index(p.index).map_err(|e| format!("Failed to read archive entry: {}", e))?;
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true) // never overwrite, never follow an existing link
            .open(real_parent.join(target.file_name().unwrap_or_default()))
            .map_err(|e| format!("Failed to create '{}': {}", p.rel.display(), e))?;

        // Copy at most (remaining budget + 1) bytes, so a zip bomb is
        // detected from REAL output, not from the sizes it claims.
        let remaining = max_total_bytes.saturating_sub(total);
        let mut limited = (&mut entry).take(remaining + 1);
        let written = std::io::copy(&mut limited, &mut out).map_err(|e| format!("Failed to extract '{}': {}", p.rel.display(), e))?;
        if written > remaining {
            drop(out);
            let _ = fs::remove_file(real_parent.join(target.file_name().unwrap_or_default()));
            return Err(format!(
                "Stopped: archive expands to more than {} MB (possible zip bomb). Files extracted before this point: {}.",
                max_total_bytes / (1024 * 1024), files
            ));
        }
        total += written;
        files += 1;
    }

    Ok(ExtractResult {
        destination: dest_rel.to_string(),
        files_extracted: files,
        folders_created: folders,
        total_bytes: total,
    })
}

/// Walks a folder WITHOUT following symlinks/junctions.
fn collect_files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>, skipped: &mut Vec<String>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("Failed to read folder: {}", e))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        if e.file_name() == ".aicoworker" {
            skipped.push(format!("{} (internal snapshots/audit log)", rel));
            continue;
        }
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() || is_reparse_point(&meta) {
            skipped.push(format!("{} (link — not followed)", rel));
            continue;
        }
        if meta.is_dir() {
            collect_files(&path, root, out, skipped)?;
        } else if meta.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Junctions on Windows are reparse points but not always reported as symlinks.
#[cfg(windows)]
fn is_reparse_point(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}
#[cfg(not(windows))]
fn is_reparse_point(_meta: &fs::Metadata) -> bool {
    false
}

pub fn create_archive(
    workspace: &Workspace,
    source_relative_paths: &[String],
    archive_relative_path: &str,
) -> Result<CreateArchiveResult, String> {
    if source_relative_paths.is_empty() {
        return Err("Give at least one file or folder to put in the archive.".into());
    }
    let archive_path = workspace.resolve(archive_relative_path)?;
    require_zip_ext(&archive_path)?;
    if archive_path.exists() {
        return Err(format!("'{}' already exists. Pick a new archive name.", archive_relative_path));
    }
    let root = workspace.root().to_path_buf();

    // Gather files first (so a bad source fails before creating the zip).
    let mut files: Vec<PathBuf> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for src in source_relative_paths {
        let p = workspace.resolve(src)?; // boundary check (follows links, then checks)
        let meta = fs::symlink_metadata(root.join(src)).map_err(|_| format!("'{}' does not exist.", src))?;
        if meta.file_type().is_symlink() || is_reparse_point(&meta) {
            skipped.push(format!("{} (link — not followed)", src));
            continue;
        }
        if p.is_dir() {
            collect_files(&p, &root, &mut files, &mut skipped)?;
        } else {
            files.push(p);
        }
    }
    files.sort();
    files.dedup();
    files.retain(|f| f != &archive_path);
    if files.len() > MAX_ENTRIES {
        return Err(format!("{} files is over the {}-file limit.", files.len(), MAX_ENTRIES));
    }

    let out = File::create(&archive_path).map_err(|e| format!("Failed to create archive: {}", e))?;
    let mut zip = ZipWriter::new(out);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut total: u64 = 0;
    let build = (|| -> Result<(), String> {
        for f in &files {
            let name = f.strip_prefix(&root).unwrap_or(f).to_string_lossy().replace('\\', "/");
            zip.start_file(name.as_str(), options).map_err(|e| format!("Failed to add '{}': {}", name, e))?;
            let mut src = File::open(f).map_err(|e| format!("Failed to read '{}': {}", name, e))?;
            let mut buf = Vec::new();
            src.read_to_end(&mut buf).map_err(|e| format!("Failed to read '{}': {}", name, e))?;
            total += buf.len() as u64;
            if total > MAX_TOTAL_BYTES {
                return Err(format!("Files add up to more than {} MB.", MAX_TOTAL_BYTES / (1024 * 1024)));
            }
            zip.write_all(&buf).map_err(|e| format!("Failed to write '{}': {}", name, e))?;
        }
        zip.finish().map_err(|e| format!("Failed to finish archive: {}", e))?;
        Ok(())
    })();
    if let Err(e) = build {
        let _ = fs::remove_file(&archive_path); // don't leave a broken zip behind
        return Err(e);
    }

    Ok(CreateArchiveResult {
        archive: archive_relative_path.to_string(),
        files_added: files.len(),
        skipped,
        total_bytes: total,
    })
}

// ---------------------------------------------------------------
// Tests: `cargo test archive`
// ---------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// <tmp>/aicw_zip_<tag>_<pid>/{ws, outside}
    fn dirs(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("aicw_zip_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let ws = base.join("ws");
        let outside = base.join("outside");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&outside).unwrap();
        (ws, outside)
    }

    /// Writes a zip with RAW entry names (the zip crate lets us put any
    /// name in, which is exactly what an attacker would do).
    fn evil_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut z = ZipWriter::new(File::create(path).unwrap());
        let opt = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, body) in entries {
            z.start_file(*name, opt).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn entry_name_rules() {
        for bad in [
            "../evil.txt", "a/../../evil.txt", "..\\evil.txt", "a\\..\\..\\evil.txt",
            "/etc/passwd", "\\Windows\\x", "C:\\x.txt", "c:x.txt", "notes.txt:hidden",
            "...\\x", "evil. ", "dir./x",
        ] {
            assert!(safe_entry_path(bad).is_err(), "should reject: {:?}", bad);
        }
        for good in ["a.txt", "dir/a.txt", "dir\\sub\\a.txt", "./a.txt", "v1.2/notes.v3.txt"] {
            assert!(safe_entry_path(good).is_ok(), "should allow: {:?}", good);
        }
    }

    #[test]
    fn zip_slip_rejects_whole_archive() {
        let (ws_dir, outside) = dirs("slip");
        evil_zip(
            &ws_dir.join("update.zip"),
            &[("readme.txt", "hello"), ("../outside/pwned.txt", "pwned"), ("..\\outside\\pwned2.txt", "pwned")],
        );
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        let r = extract_archive(&ws, "update.zip", ".");
        assert!(r.is_err(), "zip-slip archive must be rejected");
        assert!(r.unwrap_err().contains("REJECTED"));
        assert!(!outside.join("pwned.txt").exists(), "nothing may escape");
        assert!(!outside.join("pwned2.txt").exists(), "nothing may escape");
        assert!(!ws_dir.join("readme.txt").exists(), "all-or-nothing: safe entries are not extracted either");
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    #[test]
    fn absolute_and_drive_entries_rejected() {
        let (ws_dir, _outside) = dirs("abs");
        evil_zip(&ws_dir.join("a.zip"), &[("C:/Windows/evil.txt", "x")]);
        evil_zip(&ws_dir.join("b.zip"), &[("/tmp/evil.txt", "x")]);
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        assert!(extract_archive(&ws, "a.zip", "out_a").is_err());
        assert!(extract_archive(&ws, "b.zip", "out_b").is_err());
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    #[test]
    fn zip_bomb_cap_uses_real_bytes() {
        let (ws_dir, _outside) = dirs("bomb");
        let big = "A".repeat(10_000);
        evil_zip(&ws_dir.join("bomb.zip"), &[("a.txt", &big), ("b.txt", &big)]);
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        // tiny budget (15 KB) so the second file trips it
        let r = extract_with_limits(&ws, "bomb.zip", "out", MAX_ENTRIES, 15_000);
        assert!(r.is_err() && r.unwrap_err().contains("zip bomb"));
        assert!(!ws_dir.join("out").join("b.txt").exists(), "partial file must be removed");
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    #[test]
    fn never_overwrites_existing_files() {
        let (ws_dir, _outside) = dirs("overwrite");
        evil_zip(&ws_dir.join("a.zip"), &[("keep.txt", "from zip")]);
        fs::write(ws_dir.join("keep.txt"), "original").unwrap();
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        assert!(extract_archive(&ws, "a.zip", ".").is_err());
        assert_eq!(fs::read_to_string(ws_dir.join("keep.txt")).unwrap(), "original");
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    #[test]
    fn round_trip_create_then_extract() {
        let (ws_dir, _outside) = dirs("roundtrip");
        fs::create_dir_all(ws_dir.join("docs/sub")).unwrap();
        fs::write(ws_dir.join("docs/a.txt"), "alpha").unwrap();
        fs::write(ws_dir.join("docs/sub/b.txt"), "beta").unwrap();
        fs::create_dir_all(ws_dir.join(".aicoworker")).unwrap();
        fs::write(ws_dir.join(".aicoworker/audit_log.jsonl"), "secret-ish").unwrap();
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();

        let c = create_archive(&ws, &["docs".to_string()], "backup.zip").unwrap();
        assert_eq!(c.files_added, 2);
        let c2 = create_archive(&ws, &[".".to_string()], "everything.zip").unwrap();
        assert!(c2.skipped.iter().any(|s| s.starts_with(".aicoworker")), "internal folder must be skipped");

        let x = extract_archive(&ws, "backup.zip", "restored").unwrap();
        assert_eq!(x.files_extracted, 2);
        assert_eq!(fs::read_to_string(ws_dir.join("restored/docs/sub/b.txt")).unwrap(), "beta");
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    #[test]
    fn archive_paths_stay_inside_workspace() {
        let (ws_dir, outside) = dirs("bounds");
        fs::write(outside.join("secret.txt"), "s").unwrap();
        fs::write(ws_dir.join("a.txt"), "a").unwrap();
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        assert!(create_archive(&ws, &["../outside/secret.txt".to_string()], "x.zip").is_err());
        assert!(create_archive(&ws, &["a.txt".to_string()], "../outside/x.zip").is_err());
        assert!(!outside.join("x.zip").exists());
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn create_archive_does_not_follow_links() {
        let (ws_dir, outside) = dirs("links");
        fs::write(outside.join("secret.txt"), "TOP SECRET").unwrap();
        fs::create_dir_all(ws_dir.join("docs")).unwrap();
        fs::write(ws_dir.join("docs/a.txt"), "a").unwrap();
        std::os::unix::fs::symlink(&outside, ws_dir.join("docs/link")).unwrap();
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        let c = create_archive(&ws, &["docs".to_string()], "docs.zip").unwrap();
        assert_eq!(c.files_added, 1, "the link's target must not be archived");
        assert!(c.skipped.iter().any(|s| s.contains("link")));
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }
}