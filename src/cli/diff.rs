use anyhow::{Context, Result};
use std::env;

use crate::config::Config;
use crate::core::shadow::Shadow;

pub fn run(file: &str) -> Result<()> {
    let cwd = env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project (run `agit init` first)")?;
    let shadow = Shadow::new(project_root);

    let knowledge_path = shadow.knowledge_path(file);
    if !knowledge_path.exists() {
        println!("No knowledge file found for {}.", file);
        return Ok(());
    }

    // Find the most recent compaction archive
    let compaction_dir = shadow.compaction_dir();
    if !compaction_dir.exists() {
        println!("No compaction history found for {}.", file);
        println!("Knowledge file exists but has never been compacted.");
        return Ok(());
    }

    // Find dated directories, sorted descending
    let mut archive_dates: Vec<String> = std::fs::read_dir(&compaction_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            // Skip non-date dirs like "deleted"
            if name.starts_with("20") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    archive_dates.sort();
    archive_dates.reverse();

    // Find the most recent archive that contains this file
    let archive_rel = format!("{}.md", file);
    let mut found_archive = None;

    for date in &archive_dates {
        let archive_path = compaction_dir.join(date).join(&archive_rel);
        if archive_path.exists() {
            found_archive = Some((date.clone(), archive_path));
            break;
        }
    }

    match found_archive {
        Some((date, archive_path)) => {
            let old = std::fs::read_to_string(&archive_path)?;
            let new = std::fs::read_to_string(&knowledge_path)?;

            println!("Compaction diff for {} (archived {})\n", file, date);
            println!("--- pre-compaction ({})", date);
            println!("+++ current knowledge\n");

            // Simple line-by-line diff
            let old_lines: Vec<&str> = old.lines().collect();
            let new_lines: Vec<&str> = new.lines().collect();

            // Naive diff: show removed and added lines
            for line in &old_lines {
                if !new_lines.contains(line) {
                    println!("- {}", line);
                }
            }
            for line in &new_lines {
                if !old_lines.contains(line) {
                    println!("+ {}", line);
                }
            }
        }
        None => {
            println!("No compaction archive found for {}.", file);
            println!("This file hasn't been compacted yet.");
        }
    }

    Ok(())
}
