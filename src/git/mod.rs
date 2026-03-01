use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Represents a file change detected by git.
#[derive(Debug, Clone)]
pub enum FileChange {
    /// File renamed from old_path to new_path, with similarity percentage.
    Renamed {
        old_path: String,
        new_path: String,
        similarity: u32,
    },
    /// File deleted.
    Deleted { path: String },
    /// File added.
    Added { path: String },
    /// File modified.
    Modified { path: String },
}

/// Find the git repository root from a given path.
pub fn find_git_root(from: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from)
        .output()
        .context("running git rev-parse")?;

    if !output.status.success() {
        anyhow::bail!(
            "not a git repository: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let root = String::from_utf8(output.stdout)
        .context("git output not utf8")?
        .trim()
        .to_string();

    Ok(PathBuf::from(root))
}

/// Parse `git diff --name-status` output to detect renames and deletions.
/// Used by the post-commit hook to automatically track file moves.
pub fn parse_name_status(from: &Path, revision_range: &str) -> Result<Vec<FileChange>> {
    let output = Command::new("git")
        .args([
            "diff",
            "--name-status",
            "--find-renames",
            "--find-copies",
            revision_range,
        ])
        .current_dir(from)
        .output()
        .context("running git diff --name-status")?;

    if !output.status.success() {
        anyhow::bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8(output.stdout).context("git output not utf8")?;
    parse_name_status_output(&stdout)
}

/// Parse the raw output of `git diff --name-status`.
pub fn parse_name_status_output(output: &str) -> Result<Vec<FileChange>> {
    let mut changes = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }

        let status = parts[0];

        if status.starts_with('R') {
            // Rename: R100\told_path\tnew_path
            if parts.len() >= 3 {
                let similarity = status[1..].parse::<u32>().unwrap_or(100);
                changes.push(FileChange::Renamed {
                    old_path: parts[1].to_string(),
                    new_path: parts[2].to_string(),
                    similarity,
                });
            }
        } else if status == "D" {
            if parts.len() >= 2 {
                changes.push(FileChange::Deleted {
                    path: parts[1].to_string(),
                });
            }
        } else if status == "A" {
            if parts.len() >= 2 {
                changes.push(FileChange::Added {
                    path: parts[1].to_string(),
                });
            }
        } else if status == "M" {
            if parts.len() >= 2 {
                changes.push(FileChange::Modified {
                    path: parts[1].to_string(),
                });
            }
        }
    }

    Ok(changes)
}

/// Check if a path is ignored by git.
pub fn is_git_ignored(from: &Path, file_path: &str) -> bool {
    Command::new("git")
        .args(["check-ignore", "-q", file_path])
        .current_dir(from)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the list of files modified in the latest commit.
pub fn last_commit_changes(from: &Path) -> Result<Vec<FileChange>> {
    parse_name_status(from, "HEAD~1..HEAD")
}

/// Find potential rename matches for an orphaned knowledge file by checking
/// for files with similar names that were recently added.
pub fn find_potential_renames(
    from: &Path,
    old_path: &str,
) -> Result<Vec<(String, u32)>> {
    // Use git log to find if the old path appears in recent renames
    let output = Command::new("git")
        .args([
            "log",
            "--diff-filter=R",
            "--find-renames",
            "--name-status",
            "--format=",
            "-20", // last 20 commits
            "--",
            old_path,
        ])
        .current_dir(from)
        .output()
        .context("running git log for rename detection")?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8(output.stdout).context("git output not utf8")?;
    let changes = parse_name_status_output(&stdout)?;

    Ok(changes
        .into_iter()
        .filter_map(|c| match c {
            FileChange::Renamed {
                old_path: old,
                new_path,
                similarity,
            } if old == old_path => Some((new_path, similarity)),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_name_status_output() {
        let output = "R100\tsrc/auth/login.tsx\tsrc/auth/SignIn.tsx\nD\tsrc/auth/old-util.tsx\nM\tsrc/api/routes.ts\nA\tsrc/new-file.ts\n";
        let changes = parse_name_status_output(output).unwrap();
        assert_eq!(changes.len(), 4);

        match &changes[0] {
            FileChange::Renamed {
                old_path,
                new_path,
                similarity,
            } => {
                assert_eq!(old_path, "src/auth/login.tsx");
                assert_eq!(new_path, "src/auth/SignIn.tsx");
                assert_eq!(*similarity, 100);
            }
            _ => panic!("expected rename"),
        }

        match &changes[1] {
            FileChange::Deleted { path } => assert_eq!(path, "src/auth/old-util.tsx"),
            _ => panic!("expected delete"),
        }

        match &changes[2] {
            FileChange::Modified { path } => assert_eq!(path, "src/api/routes.ts"),
            _ => panic!("expected modified"),
        }

        match &changes[3] {
            FileChange::Added { path } => assert_eq!(path, "src/new-file.ts"),
            _ => panic!("expected added"),
        }
    }

    #[test]
    fn test_parse_partial_rename() {
        let output = "R085\tsrc/old.rs\tsrc/new.rs\n";
        let changes = parse_name_status_output(output).unwrap();
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            FileChange::Renamed { similarity, .. } => assert_eq!(*similarity, 85),
            _ => panic!("expected rename"),
        }
    }
}
