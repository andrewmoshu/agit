use anyhow::Result;

use crate::config::Config;
use crate::core::log::LogFile;
use crate::core::shadow::{Shadow, TargetScope};

/// Staleness report for a single target (file, directory, or root).
#[derive(Debug)]
pub struct StalenessReport {
    /// Target path using scope convention: "src/auth/login.tsx", "src/auth/", or "/"
    pub target: String,
    pub signals: Vec<StalenessSignal>,
}

#[derive(Debug)]
pub enum StalenessSignal {
    /// Log has too many entries — compaction recommended.
    LogThreshold { count: usize, threshold: usize },
    /// Knowledge file exists but source file is missing (file scope only).
    Orphaned,
}

impl StalenessReport {
    pub fn is_stale(&self) -> bool {
        !self.signals.is_empty()
    }
}

impl std::fmt::Display for StalenessReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.signals.is_empty() {
            write!(f, "  {} — up to date", self.target)?;
            return Ok(());
        }

        writeln!(f, "  {} — stale", self.target)?;
        for signal in &self.signals {
            match signal {
                StalenessSignal::LogThreshold { count, threshold } => {
                    writeln!(f, "    log: {} entries (threshold: {})", count, threshold)?;
                }
                StalenessSignal::Orphaned => {
                    writeln!(f, "    orphaned: source file not found")?;
                }
            }
        }
        Ok(())
    }
}

/// Check staleness for any target (file, directory, or root).
pub fn check_staleness(
    target: &str,
    shadow: &Shadow,
    config: &Config,
) -> Result<StalenessReport> {
    let mut signals = Vec::new();
    let scope = TargetScope::parse(target);

    // Orphan check only applies to files
    if let TargetScope::File(ref source_rel) = scope {
        let source_path = shadow.project_root.join(source_rel);
        let knowledge_path = shadow.knowledge_path(source_rel);

        if !source_path.exists() && knowledge_path.exists() {
            return Ok(StalenessReport {
                target: target.to_string(),
                signals: vec![StalenessSignal::Orphaned],
            });
        }
    }

    // Check log threshold (works for all scopes)
    let log_path = shadow.resolve_log_path(target);
    let log = LogFile::new(log_path);
    if log.exists() {
        let count = log.count()?;
        if count >= config.compaction.log_threshold {
            signals.push(StalenessSignal::LogThreshold {
                count,
                threshold: config.compaction.log_threshold,
            });
        }
    }

    Ok(StalenessReport {
        target: target.to_string(),
        signals,
    })
}

/// Check staleness across all tracked targets (files, directories, root).
pub fn check_all_staleness(shadow: &Shadow, config: &Config) -> Result<Vec<StalenessReport>> {
    let targets = shadow.tracked_targets()?;
    let mut reports = Vec::new();

    for target in targets {
        let report = check_staleness(&target, shadow, config)?;
        reports.push(report);
    }

    Ok(reports)
}
