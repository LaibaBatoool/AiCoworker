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
            .canonicalize() // resolves to an absolute, normalized path
            .map_err(|e| format!("Invalid workspace root: {}", e))?;

        if !root_path.is_dir() {
            return Err("Workspace root must be a directory".to_string());
        }

        Ok(Workspace { root: root_path })
    }

    /// The core safety check: does this path live inside the workspace?
    /// Takes a relative path (what the agent/LLM will request) and
    /// returns the resolved absolute path ONLY if it's safely inside.
    pub fn resolve(&self, relative_path: &str) -> Result<PathBuf, String> {
        let candidate = self.root.join(relative_path);

        // canonicalize to resolve ".." and symlinks, THEN check containment.
        // If the file doesn't exist yet (e.g. we're about to create it),
        // canonicalize fails, so we check the parent directory instead.
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