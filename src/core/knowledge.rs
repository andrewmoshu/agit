use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A knowledge .md file: pure markdown, no frontmatter.
///
/// Old files with YAML frontmatter are handled gracefully — the frontmatter
/// is stripped on load so everything migrates transparently.
#[derive(Debug, Clone)]
pub struct KnowledgeFile {
    pub body: String,
    pub path: PathBuf,
}

impl KnowledgeFile {
    /// Load a knowledge .md file from disk.
    /// Strips YAML frontmatter if present (backward compat).
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("reading knowledge file {}", path.display()))?;
        Ok(Self {
            body: strip_frontmatter(&contents),
            path: path.to_path_buf(),
        })
    }

    /// Write to disk (pure markdown, no frontmatter).
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
        std::fs::write(&self.path, &self.body)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }

    /// Create a new empty knowledge file for a source file.
    pub fn new_for_file(source_path: &str, agit_path: PathBuf) -> Self {
        Self {
            body: format!("# {}\n\n## Intent\n\n\n## Decisions\n\n\n## Constraints\n\n\n## Lessons Learned\n\n", source_path),
            path: agit_path,
        }
    }

    /// Create a new empty knowledge file for a directory.
    pub fn new_for_dir(dir_path: &str, agit_path: PathBuf) -> Self {
        Self {
            body: format!(
                "# {}/\n\n## Purpose\n\n\n## Architecture Decisions\n\n\n## Conventions\n\n",
                dir_path
            ),
            path: agit_path,
        }
    }

    /// Create a new root knowledge file.
    pub fn new_root(_project_name: &str, agit_path: PathBuf) -> Self {
        Self {
            body: "# Project Knowledge\n\n## Stack\n\n\n## Conventions\n\n\n## Known Landmines\n\n".to_string(),
            path: agit_path,
        }
    }
}

/// Strip YAML frontmatter from markdown content.
/// If no frontmatter, returns content as-is.
fn strip_frontmatter(contents: &str) -> String {
    let trimmed = contents.trim_start();

    if !trimmed.starts_with("---") {
        return contents.to_string();
    }

    let after_first = &trimmed[3..];
    let after_first = after_first.trim_start_matches(['\r', '\n']);

    if let Some(end_idx) = after_first.find("\n---") {
        let body_start = end_idx + 4; // skip \n---
        let body = &after_first[body_start..];
        // Trim leading newlines but preserve the rest
        body.trim_start_matches(['\r', '\n']).to_string()
    } else {
        // Malformed frontmatter — return everything
        contents.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_with_frontmatter() {
        // Old format: YAML frontmatter should be stripped
        let content = "---\nfile: src/auth/login.tsx\nlast_compacted: 2026-02-20T00:00:00Z\n---\n\n# src/auth/login.tsx\n\n## Intent\n\nHandles user login.\n";
        let kf = KnowledgeFile {
            body: strip_frontmatter(content),
            path: PathBuf::from("test.md"),
        };
        assert!(kf.body.contains("Handles user login"));
        assert!(!kf.body.contains("---"));
        assert!(!kf.body.contains("last_compacted"));
    }

    #[test]
    fn test_load_without_frontmatter() {
        let content = "# Just a heading\n\nSome content.\n";
        let kf = KnowledgeFile {
            body: strip_frontmatter(content),
            path: PathBuf::from("test.md"),
        };
        assert!(kf.body.contains("Just a heading"));
    }

    #[test]
    fn test_save_no_frontmatter() {
        let kf = KnowledgeFile::new_for_file("src/main.rs", PathBuf::from("test.md"));
        // Body should be pure markdown, no ---
        assert!(!kf.body.contains("---"));
        assert!(kf.body.starts_with("# src/main.rs"));
    }
}
