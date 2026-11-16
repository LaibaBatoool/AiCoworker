use std::path::{Component, Path, PathBuf};

/// The app's own folder inside every workspace: audit log + undo snapshots.
/// Agent tools must never read or change it (the agent could otherwise
/// rewrite its own audit trail or wipe the undo history).
pub const INTERNAL_DIR: &str = ".aicoworker";

/// Represents the one folder the agent is allowed to operate inside.
#[derive(Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Create a new workspace rooted at the given folder.
    /// Fails if the folder doesn't exist.
    pub fn new(root: &str) -> Result<Self, String> {
        let root_path = Path::new(root)
            .canonicalize()
            .map_err(|e| format!("Invalid workspace root: {}", e))?;

        if !root_path.is_dir() {
            return Err("Workspace root must be a directory".to_string());
        }

        Ok(Workspace { root: root_path })
    }

    /// Exposes the workspace root path, e.g. for setting a subprocess's
    /// working directory to confine terminal command execution.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// True for the workspace root itself (deleting or moving it would also
    /// remove the internal folder and everything the user has).
    pub fn is_root(&self, resolved: &Path) -> bool {
        resolved == self.root
    }

    /// True if a resolved path is the internal folder or inside it.
    /// Case-insensitive because Windows paths are.
    fn is_internal(&self, resolved: &Path) -> bool {
        match resolved.strip_prefix(&self.root).ok().and_then(|r| r.components().next()) {
            Some(Component::Normal(first)) => first.to_string_lossy().eq_ignore_ascii_case(INTERNAL_DIR),
            _ => false,
        }
    }

    /// The core safety check: does this path live inside the workspace?
    pub fn resolve(&self, relative_path: &str) -> Result<PathBuf, String> {
        let candidate = self.root.join(relative_path);

        // Dangling symlink check. `exists()` FOLLOWS links, so a link
        // whose target doesn't exist yet (e.g. notes.txt -> C:\outside\new.txt)
        // reports "doesn't exist" and would fall into the branch below,
        // which only checks the PARENT folder. The parent is inside the
        // workspace, so it would pass, and write_file would then create
        // the file THROUGH the link, outside the workspace.
        // symlink_metadata does NOT follow links, so it sees the link itself.
        // (Links whose target exists are fine: canonicalize follows them and
        // the starts_with check below catches an outside target.)
        if !candidate.exists() && std::fs::symlink_metadata(&candidate).is_ok() {
            return Err(format!(
                "Access denied: '{}' is a broken symbolic link (its target does not exist), which could point outside the workspace",
                relative_path
            ));
        }

        let resolved = if candidate.exists() {
            candidate
                .canonicalize()
                .map_err(|e| format!("Failed to resolve path: {}", e))?
        } else {
            let parent = candidate
                .parent()
                .ok_or("Invalid path".to_string())?
                .canonicalize()
                .map_err(|e| format!("Failed to resolve parent path: {}", e))?;
            parent.join(candidate.file_name().ok_or("Invalid file name")?)
        };

        if resolved.starts_with(&self.root) {
            if self.is_internal(&resolved) {
                return Err(format!(
                    "Access denied: '{}' is inside the app's internal folder ({}), which holds the audit log and undo snapshots. Agent tools cannot read or change it.",
                    relative_path, INTERNAL_DIR
                ));
            }
            Ok(resolved)
        } else {
            Err(format!(
                "Access denied: '{}' is outside the workspace boundary",
                relative_path
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Creates <tmp>/aicw_test_<tag>_<pid>/{ws, outside}; outside holds a canary file.
    fn make_dirs(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("aicw_test_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let ws = base.join("ws");
        let outside = base.join("outside");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("canary.txt"), "DO NOT TOUCH").unwrap();
        (ws, outside)
    }

    #[test]
    fn blocks_parent_traversal() {
        let (ws_dir, outside) = make_dirs("traversal");
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        for p in [
            "../outside/canary.txt",
            "../outside/new.txt",
            "sub/../../outside/canary.txt",
            "..\\outside\\canary.txt",
            "..",
        ] {
            assert!(ws.resolve(p).is_err(), "should block: {}", p);
        }
        assert_eq!(fs::read_to_string(outside.join("canary.txt")).unwrap(), "DO NOT TOUCH");
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    #[test]
    fn blocks_absolute_paths_outside() {
        let (ws_dir, outside) = make_dirs("absolute");
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        let abs = outside.join("canary.txt");
        assert!(ws.resolve(abs.to_str().unwrap()).is_err());
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    /// Creates a file symlink. Returns false if the OS refused (on Windows
    /// that needs Developer Mode or admin), so the test can skip instead
    /// of failing for an environment reason.
    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let r = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let r = std::os::windows::fs::symlink_file(target, link);
        match r {
            Ok(()) => true,
            Err(e) => {
                eprintln!("SKIPPED symlink test: could not create symlink ({}). On Windows, enable Developer Mode.", e);
                false
            }
        }
    }

    #[test]
    fn blocks_symlink_escapes() {
        let (ws_dir, outside) = make_dirs("symlink");
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();

        // 1. live link -> existing file outside: canonicalize follows it, blocked
        let live = ws_dir.join("live.txt");
        // 2. dangling link -> not-yet-existing file outside: the gap this fixes
        let dangling = ws_dir.join("dangling.txt");
        let target_new = outside.join("created_through_link.txt");

        if !try_symlink_file(&outside.join("canary.txt"), &live)
            || !try_symlink_file(&target_new, &dangling)
        {
            let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
            return;
        }

        assert!(ws.resolve("live.txt").is_err(), "live link to outside must be blocked");
        assert!(ws.resolve("dangling.txt").is_err(), "dangling link must be blocked");

        // and the real write path: write_file goes through resolve first
        let r = crate::tools::write_file::write_file(&ws, "dangling.txt", "pwned");
        assert!(r.is_err(), "write through dangling link must fail");
        assert!(!target_new.exists(), "nothing may be created outside the workspace");
        assert_eq!(fs::read_to_string(outside.join("canary.txt")).unwrap(), "DO NOT TOUCH");

        let _ = fs::remove_file(&live);
        let _ = fs::remove_file(&dangling);
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    /// Directory junctions need NO special rights on Windows (unlike
    /// symlinks), so this is the realistic escape. canonicalize follows
    /// junctions, so they should already be blocked — this proves it.
    #[cfg(windows)]
    #[test]
    fn blocks_junction_escapes() {
        let (ws_dir, outside) = make_dirs("junction");
        let link = ws_dir.join("j");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !made {
            eprintln!("SKIPPED junction test: mklink /J failed");
            let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
            return;
        }
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();

        assert!(ws.resolve("j").is_err(), "the junction itself points outside");
        assert!(ws.resolve("j/canary.txt").is_err(), "existing file through junction");
        assert!(ws.resolve("j/new.txt").is_err(), "new file through junction");

        let r = crate::tools::write_file::write_file(&ws, "j/new.txt", "pwned");
        assert!(r.is_err(), "write through junction must fail");
        assert!(!outside.join("new.txt").exists(), "nothing may be created outside the workspace");

        // remove the junction FIRST so cleanup can never recurse into `outside`
        let _ = fs::remove_dir(&link);
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    /// The agent must not reach the audit log or the undo snapshots
    /// (found 16 Nov 2026: write_file could empty audit_log.jsonl).
    #[test]
    fn blocks_internal_folder() {
        let (ws_dir, _outside) = make_dirs("internal");
        fs::create_dir_all(ws_dir.join(".aicoworker/snapshots")).unwrap();
        fs::create_dir_all(ws_dir.join("sub")).unwrap();
        fs::write(ws_dir.join(".aicoworker/audit_log.jsonl"), "entry\n").unwrap();
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        for p in [
            ".aicoworker",
            ".aicoworker/audit_log.jsonl",
            ".aicoworker/new.txt",
            ".aicoworker/snapshots",
            "sub/../.aicoworker/audit_log.jsonl",
            "./.aicoworker/audit_log.jsonl",
            #[cfg(windows)]
            ".aicoworker\\audit_log.jsonl",
        ] {
            let r = ws.resolve(p);
            assert!(r.as_ref().err().map(|e| e.contains("internal folder")).unwrap_or(false), "should block: {} (got {:?})", p, r);
        }
        // the real tools, end to end
        assert!(crate::tools::write_file::write_file(&ws, ".aicoworker/audit_log.jsonl", "").is_err());
        assert!(crate::tools::delete_file::delete_file(&ws, ".aicoworker", true).is_err());
        assert!(crate::tools::move_rename::move_rename(&ws, ".aicoworker", "moved").is_err());
        assert_eq!(fs::read_to_string(ws_dir.join(".aicoworker/audit_log.jsonl")).unwrap(), "entry\n");
        assert!(ws_dir.join(".aicoworker/snapshots").exists());
        // look-alike names are ordinary files
        assert!(ws.resolve(".aicoworker-notes.txt").is_ok());
        assert!(ws.resolve("sub/.aicoworker").is_ok(), "only the top-level folder is internal");
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    /// Windows paths ignore case, so .AiCoworker is the same folder there.
    #[cfg(windows)]
    #[test]
    fn blocks_internal_folder_any_case() {
        let (ws_dir, _outside) = make_dirs("internal_case");
        fs::create_dir_all(ws_dir.join(".aicoworker")).unwrap();
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        assert!(ws.resolve(".AICOWORKER/audit_log.jsonl").is_err());
        assert!(ws.resolve(".AiCoworker").is_err());
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    /// Deleting or moving "." would take the whole workspace (and the
    /// internal folder) with it.
    #[test]
    fn root_cannot_be_deleted_or_moved() {
        let (ws_dir, _outside) = make_dirs("root");
        fs::write(ws_dir.join("keep.txt"), "x").unwrap();
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        for p in [".", "", "./", "sub/.."] {
            let r = crate::tools::delete_file::delete_file(&ws, p, true);
            assert!(r.is_err(), "delete of root must fail: {:?}", p);
        }
        assert!(crate::tools::move_rename::move_rename(&ws, ".", "elsewhere").is_err());
        assert!(ws_dir.join("keep.txt").exists());
        assert!(ws.resolve(".").is_ok(), "listing the root is still fine");
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }

    #[test]
    fn allows_paths_inside() {
        let (ws_dir, _outside) = make_dirs("inside");
        fs::create_dir(ws_dir.join("sub")).unwrap();
        let ws = Workspace::new(ws_dir.to_str().unwrap()).unwrap();
        assert!(ws.resolve("notes.txt").is_ok());
        assert!(ws.resolve("sub/new.txt").is_ok());
        let _ = fs::remove_dir_all(ws_dir.parent().unwrap());
    }
}