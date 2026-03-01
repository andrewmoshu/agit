use anyhow::{Context, Result};
use std::env;

use crate::config::Config;
use crate::core::log::{Confidence, EntryType, LogEntry, LogFile};
use crate::core::shadow::Shadow;
use crate::git::{last_commit_changes, FileChange};

/// Sync shadow files with the latest git commit.
///
/// Detects renames and deletes from `git diff --name-status HEAD~1..HEAD`
/// and automatically moves/archives shadow files to match.
///
/// Intended to run as a post-commit hook:
///   .git/hooks/post-commit → `agit sync 2>/dev/null || true`
pub fn run() -> Result<()> {
    let cwd = env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project")?;
    let shadow = Shadow::new(project_root.clone());

    let changes = last_commit_changes(&project_root)?;

    let mut moved = 0;
    let mut archived = 0;

    for change in &changes {
        match change {
            FileChange::Renamed { old_path, new_path, .. } => {
                let has_md = shadow.knowledge_path(old_path).exists();
                let has_log = shadow.log_path(old_path).exists();

                if has_md || has_log {
                    shadow.move_shadow(old_path, new_path)?;

                    // Log the rename
                    let entry = LogEntry::new(
                        "agit".to_string(),
                        EntryType::Insight,
                        format!("File renamed from {} to {} (auto-detected by agit sync).", old_path, new_path),
                        Confidence::Observed,
                        vec![],
                        vec!["rename".to_string()],
                    );
                    let log = LogFile::new(shadow.log_path(new_path));
                    log.append(&entry)?;

                    moved += 1;
                }
            }
            FileChange::Deleted { path } => {
                let has_md = shadow.knowledge_path(path).exists();
                let has_log = shadow.log_path(path).exists();

                if has_md || has_log {
                    shadow.archive_deleted(path)?;
                    archived += 1;
                }
            }
            _ => {}
        }
    }

    if moved > 0 || archived > 0 {
        println!("agit sync: {} moved, {} archived", moved, archived);
    }

    Ok(())
}
