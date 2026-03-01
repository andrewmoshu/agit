use anyhow::{Context, Result};
use std::env;

use crate::config::Config;
use crate::core::log::{Confidence, EntryType, LogEntry, LogFile};
use crate::core::shadow::Shadow;

pub fn run(old: &str, new: &str) -> Result<()> {
    let cwd = env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project (run `agit init` first)")?;
    let shadow = Shadow::new(project_root);

    let old_md = shadow.knowledge_path(old);
    let old_log = shadow.log_path(old);

    if !old_md.exists() && !old_log.exists() {
        println!("No knowledge found for {}. Nothing to move.", old);
        return Ok(());
    }

    // Move shadow files
    shadow.move_shadow(old, new)?;

    // Append a log entry noting the rename
    let entry = LogEntry::new(
        "agit".to_string(),
        EntryType::Insight,
        format!("File renamed from {} to {}.", old, new),
        Confidence::Observed,
        vec![],
        vec!["rename".to_string()],
    );

    let new_log_path = shadow.log_path(new);
    let log = LogFile::new(new_log_path);
    log.append(&entry)?;

    println!("Moved knowledge: {} -> {}", old, new);

    Ok(())
}
