use std::path::{Path, PathBuf};

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

    /// The core safety check: does this path live inside the workspace?
    pub fn resolve(&self, relative_path: &str) -> Result<PathBuf, String> {
        let candidate = self.root.join(relative_path);

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