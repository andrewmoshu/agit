use anyhow::{Context, Result};
use std::env;

use crate::config::Config;
use crate::core::log::{Confidence, EntryType, LogEntry, LogFile};
use crate::core::shadow::Shadow;

pub fn run(
    file: &str,
    agent: &str,
    entry_type: &str,
    content: &str,
    confidence: &str,
    anchors: Option<Vec<String>>,
    tags: Option<Vec<String>>,
) -> Result<()> {
    let cwd = env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project (run `agit init` first)")?;
    let shadow = Shadow::new(project_root);

    let entry_type: EntryType = entry_type.parse()?;
    let confidence: Confidence = confidence.parse()?;

    let entry = LogEntry::new(
        agent.to_string(),
        entry_type,
        content.to_string(),
        confidence,
        anchors.unwrap_or_default(),
        tags.unwrap_or_default(),
    );

    let log_path = shadow.resolve_log_path(file);
    let log = LogFile::new(log_path);
    log.append(&entry)?;

    println!("Recorded {} for {}.", entry.entry_type, file);

    // Check if compaction is recommended
    let count = log.count()?;
    let config = Config::load(&shadow.project_root)?;
    if count >= config.compaction.log_threshold {
        println!(
            "  Log has {} entries (threshold: {}). Consider: agit compact {}",
            count, config.compaction.log_threshold, file
        );
    }

    Ok(())
}
