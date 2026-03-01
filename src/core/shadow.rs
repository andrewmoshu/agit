use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Target scope for write/compact operations.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetScope {
    /// Root project level ("/")
    Root,
    /// Directory level ("src/auth/")
    Dir(String),
    /// File level ("src/auth/login.ts")
    File(String),
}

impl TargetScope {
    /// Parse a target path into a scope.
    /// "/" or "" → Root, trailing "/" → Dir, else → File.
    pub fn parse(target: &str) -> Self {
        let trimmed = target.trim_matches('/').trim();
        if trimmed.is_empty() {
            Self::Root
        } else if target.ends_with('/') {
            Self::Dir(trimmed.to_string())
        } else {
            Self::File(target.to_string())
        }
    }
}

/// Manages the .agit/ shadow directory that mirrors the source tree.
///
/// Source file: `src/auth/login.tsx`
/// Knowledge:  `.agit/src/auth/login.tsx.md`
/// Log:        `.agit/src/auth/login.tsx.jsonl`
///
/// Directory:  `src/auth/`
/// Knowledge:  `.agit/src/auth/_dir.md`
/// Log:        `.agit/src/auth/_dir.jsonl`
///
/// Root:       `.agit/_root.md`, `.agit/_root.jsonl`
pub struct Shadow {
    /// Absolute path to the project root (contains .agit/).
    pub project_root: PathBuf,
}

impl Shadow {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    /// Path to the .agit directory.
    pub fn agit_dir(&self) -> PathBuf {
        self.project_root.join(".agit")
    }

    /// Knowledge .md path for a source file (relative path from project root).
    pub fn knowledge_path(&self, source_rel: &str) -> PathBuf {
        self.agit_dir().join(format!("{}.md", source_rel))
    }

    /// Log .jsonl path for a source file (relative path from project root).
    pub fn log_path(&self, source_rel: &str) -> PathBuf {
        self.agit_dir().join(format!("{}.jsonl", source_rel))
    }

    /// Knowledge .md path for a directory.
    pub fn dir_knowledge_path(&self, dir_rel: &str) -> PathBuf {
        self.agit_dir().join(dir_rel).join("_dir.md")
    }

    /// Log .jsonl path for a directory.
    pub fn dir_log_path(&self, dir_rel: &str) -> PathBuf {
        self.agit_dir().join(dir_rel).join("_dir.jsonl")
    }

    /// Root knowledge path.
    pub fn root_knowledge_path(&self) -> PathBuf {
        self.agit_dir().join("_root.md")
    }

    /// Root log path.
    pub fn root_log_path(&self) -> PathBuf {
        self.agit_dir().join("_root.jsonl")
    }

    /// Resolve the log path for any target scope.
    pub fn resolve_log_path(&self, target: &str) -> PathBuf {
        match TargetScope::parse(target) {
            TargetScope::Root => self.root_log_path(),
            TargetScope::Dir(d) => self.dir_log_path(&d),
            TargetScope::File(f) => self.log_path(&f),
        }
    }

    /// Resolve the knowledge path for any target scope.
    pub fn resolve_knowledge_path(&self, target: &str) -> PathBuf {
        match TargetScope::parse(target) {
            TargetScope::Root => self.root_knowledge_path(),
            TargetScope::Dir(d) => self.dir_knowledge_path(&d),
            TargetScope::File(f) => self.knowledge_path(&f),
        }
    }

    /// Config path.
    pub fn config_path(&self) -> PathBuf {
        self.agit_dir().join("config.yaml")
    }

    /// Compaction archive directory.
    pub fn compaction_dir(&self) -> PathBuf {
        self.agit_dir().join(".compaction")
    }

    /// Archive path for a pre-compaction snapshot.
    pub fn archive_path(&self, source_rel: &str, date: &str) -> PathBuf {
        self.compaction_dir().join(date).join(format!("{}.md", source_rel))
    }

    /// Deleted files archive path.
    pub fn deleted_archive_path(&self, source_rel: &str) -> PathBuf {
        self.compaction_dir().join("deleted").join(format!("{}.md", source_rel))
    }

    /// Initialize the .agit directory structure.
    pub fn init(&self) -> Result<()> {
        let agit_dir = self.agit_dir();
        std::fs::create_dir_all(&agit_dir)
            .with_context(|| format!("creating {}", agit_dir.display()))?;
        Ok(())
    }

    /// Check if .agit exists in the project root.
    pub fn is_initialized(&self) -> bool {
        self.agit_dir().is_dir()
    }

    /// Given a source file relative path, return the hierarchy of knowledge
    /// files to read (root → dir chain → file), from general to specific.
    ///
    /// Example: "src/auth/login.tsx" returns:
    ///   [_root.md, src/_dir.md, src/auth/_dir.md, src/auth/login.tsx.md]
    pub fn knowledge_hierarchy(&self, source_rel: &str) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 1. Root knowledge
        paths.push(self.root_knowledge_path());

        // 2. Directory chain
        let source_path = Path::new(source_rel);
        let mut ancestors: Vec<&Path> = source_path.ancestors().skip(1).collect();
        ancestors.reverse();
        // ancestors now contains: ["", "src", "src/auth"] for "src/auth/login.tsx"
        for ancestor in ancestors {
            let dir_str = ancestor.to_string_lossy();
            if !dir_str.is_empty() {
                paths.push(self.dir_knowledge_path(&dir_str));
            }
        }

        // 3. File knowledge
        paths.push(self.knowledge_path(source_rel));

        paths
    }

    /// Move shadow files when a source file is renamed.
    pub fn move_shadow(&self, old_rel: &str, new_rel: &str) -> Result<()> {
        // Move knowledge .md
        let old_md = self.knowledge_path(old_rel);
        let new_md = self.knowledge_path(new_rel);
        if old_md.exists() {
            if let Some(parent) = new_md.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&old_md, &new_md)
                .with_context(|| format!("moving {} → {}", old_md.display(), new_md.display()))?;
        }

        // Move log .jsonl
        let old_log = self.log_path(old_rel);
        let new_log = self.log_path(new_rel);
        if old_log.exists() {
            if let Some(parent) = new_log.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&old_log, &new_log)
                .with_context(|| format!("moving {} → {}", old_log.display(), new_log.display()))?;
        }

        Ok(())
    }

    /// Archive shadow files for a deleted source file.
    pub fn archive_deleted(&self, source_rel: &str) -> Result<()> {
        let md_path = self.knowledge_path(source_rel);
        if md_path.exists() {
            let archive = self.deleted_archive_path(source_rel);
            if let Some(parent) = archive.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&md_path, &archive)
                .with_context(|| format!("archiving {}", md_path.display()))?;
        }

        // Remove the log — it's raw data, less critical to preserve
        let log_path = self.log_path(source_rel);
        if log_path.exists() {
            std::fs::remove_file(&log_path)?;
        }

        Ok(())
    }

    /// List all source files that have knowledge (by scanning .agit/).
    pub fn tracked_files(&self) -> Result<Vec<String>> {
        let agit_dir = self.agit_dir();
        if !agit_dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        collect_tracked_files(&agit_dir, &agit_dir, &mut files)?;
        files.sort();
        Ok(files)
    }

    /// List all tracked targets — files, directories, and root.
    /// Returns target paths using the scope convention:
    ///   "/" for root, "src/auth/" for dirs, "src/auth/login.tsx" for files.
    pub fn tracked_targets(&self) -> Result<Vec<String>> {
        let agit_dir = self.agit_dir();
        if !agit_dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut targets = Vec::new();

        // Check root
        if self.root_knowledge_path().exists() || self.root_log_path().exists() {
            targets.push("/".to_string());
        }

        collect_tracked_targets(&agit_dir, &agit_dir, &mut targets)?;
        targets.sort();
        Ok(targets)
    }
}

/// Recursively collect tracked files from the .agit directory.
fn collect_tracked_files(base: &Path, current: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let name = path.file_name().unwrap().to_string_lossy();
            // Skip .compaction and other dot-dirs
            if name.starts_with('.') {
                continue;
            }
            collect_tracked_files(base, &path, out)?;
        } else if let Some(name) = path.file_name() {
            let name = name.to_string_lossy();
            // We want .md files that aren't _root.md or _dir.md
            if name.ends_with(".md") && name != "_root.md" && name != "_dir.md" {
                // Strip ".md" suffix and make relative to .agit/
                let rel = path.strip_prefix(base).unwrap();
                let source_rel = rel.to_string_lossy();
                // Remove trailing .md
                let source = source_rel.trim_end_matches(".md");
                out.push(source.replace('\\', "/"));
            }
        }
    }
    Ok(())
}

/// Recursively collect all tracked targets (files + directories) from .agit.
fn collect_tracked_targets(base: &Path, current: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let name = path.file_name().unwrap().to_string_lossy();
            if name.starts_with('.') {
                continue;
            }

            // Check if this directory has _dir.md or _dir.jsonl
            let dir_md = path.join("_dir.md");
            let dir_jsonl = path.join("_dir.jsonl");
            if dir_md.exists() || dir_jsonl.exists() {
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                out.push(format!("{}/", rel));
            }

            collect_tracked_targets(base, &path, out)?;
        } else if let Some(name) = path.file_name() {
            let name = name.to_string_lossy();
            if name.ends_with(".md") && name != "_root.md" && name != "_dir.md" {
                let rel = path.strip_prefix(base).unwrap();
                let source_rel = rel.to_string_lossy();
                let source = source_rel.trim_end_matches(".md");
                out.push(source.replace('\\', "/"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_paths() {
        let shadow = Shadow::new(PathBuf::from("/project"));
        assert_eq!(
            shadow.knowledge_path("src/auth/login.tsx"),
            PathBuf::from("/project/.agit/src/auth/login.tsx.md")
        );
        assert_eq!(
            shadow.log_path("src/auth/login.tsx"),
            PathBuf::from("/project/.agit/src/auth/login.tsx.jsonl")
        );
        assert_eq!(
            shadow.dir_knowledge_path("src/auth"),
            PathBuf::from("/project/.agit/src/auth/_dir.md")
        );
    }

    #[test]
    fn test_knowledge_hierarchy() {
        let shadow = Shadow::new(PathBuf::from("/project"));
        let hierarchy = shadow.knowledge_hierarchy("src/auth/login.tsx");
        assert_eq!(hierarchy.len(), 4);
        assert_eq!(hierarchy[0], PathBuf::from("/project/.agit/_root.md"));
        assert_eq!(hierarchy[1], PathBuf::from("/project/.agit/src/_dir.md"));
        assert_eq!(hierarchy[2], PathBuf::from("/project/.agit/src/auth/_dir.md"));
        assert_eq!(
            hierarchy[3],
            PathBuf::from("/project/.agit/src/auth/login.tsx.md")
        );
    }
}
