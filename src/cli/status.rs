use anyhow::{Context, Result};
use std::env;

use crate::config::Config;
use crate::core::shadow::Shadow;
use crate::core::staleness::{check_all_staleness, StalenessSignal};

pub fn run() -> Result<()> {
    let cwd = env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project (run `agit init` first)")?;
    let config = Config::load(&project_root)?;
    let shadow = Shadow::new(project_root);

    let targets = shadow.tracked_targets()?;

    if targets.is_empty() {
        println!("No tracked targets. Use `agit write <file> ...` to start recording knowledge.");
        return Ok(());
    }

    let reports = check_all_staleness(&shadow, &config)?;

    let stale: Vec<_> = reports.iter().filter(|r| r.is_stale()).collect();
    let orphaned: Vec<_> = reports
        .iter()
        .filter(|r| {
            r.signals
                .iter()
                .any(|s| matches!(s, StalenessSignal::Orphaned))
        })
        .collect();
    let fresh: Vec<_> = reports.iter().filter(|r| !r.is_stale()).collect();

    println!("agit status: {} tracked targets\n", targets.len());

    if !orphaned.is_empty() {
        println!("Orphaned knowledge (source file missing):");
        for report in &orphaned {
            print!("{}", report);
        }
        println!();
    }

    if !stale.is_empty() {
        let non_orphan_stale: Vec<_> = stale
            .iter()
            .filter(|r| {
                !r.signals
                    .iter()
                    .any(|s| matches!(s, StalenessSignal::Orphaned))
            })
            .collect();

        if !non_orphan_stale.is_empty() {
            println!("Compaction recommended:");
            for report in &non_orphan_stale {
                print!("{}", report);
            }
            println!();
        }
    }

    if !fresh.is_empty() {
        println!("Up to date: {}", fresh.len());
    }

    if !stale.is_empty() {
        println!(
            "\nRun `agit compact --stale` to compact all stale targets."
        );
    }

    Ok(())
}
