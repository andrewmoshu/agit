use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// A single log entry in a .jsonl file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// When the entry was written (ISO 8601).
    pub ts: DateTime<Utc>,

    /// Which agent wrote it.
    pub agent: String,

    /// Category of knowledge.
    #[serde(rename = "type")]
    pub entry_type: EntryType,

    /// The insight, in natural language.
    pub content: String,

    /// `observed` (fact) or `inferred` (theory). Defaults to `observed`.
    #[serde(default = "default_confidence", skip_serializing_if = "is_observed")]
    pub confidence: Confidence,

    /// Code elements this relates to (function names, class names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<String>,

    /// Freeform tags for filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    Insight,
    Decision,
    Failure,
    Constraint,
    Relationship,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Observed,
    Inferred,
}

fn default_confidence() -> Confidence {
    Confidence::Observed
}

fn is_observed(c: &Confidence) -> bool {
    *c == Confidence::Observed
}

impl std::fmt::Display for EntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryType::Insight => write!(f, "insight"),
            EntryType::Decision => write!(f, "decision"),
            EntryType::Failure => write!(f, "failure"),
            EntryType::Constraint => write!(f, "constraint"),
            EntryType::Relationship => write!(f, "relationship"),
        }
    }
}

impl std::str::FromStr for EntryType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "insight" => Ok(EntryType::Insight),
            "decision" => Ok(EntryType::Decision),
            "failure" => Ok(EntryType::Failure),
            "constraint" => Ok(EntryType::Constraint),
            "relationship" => Ok(EntryType::Relationship),
            other => anyhow::bail!("unknown entry type: {other}. Expected: insight, decision, failure, constraint, relationship"),
        }
    }
}

impl std::str::FromStr for Confidence {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "observed" => Ok(Confidence::Observed),
            "inferred" => Ok(Confidence::Inferred),
            other => anyhow::bail!("unknown confidence: {other}. Expected: observed, inferred"),
        }
    }
}

/// A log file (.jsonl) — read and append operations.
pub struct LogFile {
    pub path: PathBuf,
}

impl LogFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Append a single entry to the log. Creates the file (and parent dirs) if needed.
    pub fn append(&self, entry: &LogEntry) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("opening log file {}", self.path.display()))?;

        let line = serde_json::to_string(entry).context("serializing log entry")?;
        writeln!(file, "{}", line)
            .with_context(|| format!("writing to {}", self.path.display()))?;
        Ok(())
    }

    /// Read all entries from the log.
    pub fn read_all(&self) -> Result<Vec<LogEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = std::fs::File::open(&self.path)
            .with_context(|| format!("opening {}", self.path.display()))?;
        let reader = std::io::BufReader::new(file);

        let mut entries = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line.with_context(|| format!("reading line {} of {}", i + 1, self.path.display()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<LogEntry>(trimmed) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    // JSONL resilience: skip malformed lines, don't fail entirely
                    eprintln!(
                        "warning: skipping malformed log entry at {}:{}: {}",
                        self.path.display(),
                        i + 1,
                        e
                    );
                }
            }
        }
        Ok(entries)
    }

    /// Read the last N entries from the log.
    pub fn read_tail(&self, n: usize) -> Result<Vec<LogEntry>> {
        let all = self.read_all()?;
        let start = all.len().saturating_sub(n);
        Ok(all[start..].to_vec())
    }

    /// Count entries without fully parsing them.
    pub fn count(&self) -> Result<usize> {
        if !self.path.exists() {
            return Ok(0);
        }
        let file = std::fs::File::open(&self.path)
            .with_context(|| format!("opening {}", self.path.display()))?;
        let reader = std::io::BufReader::new(file);
        let count = reader
            .lines()
            .filter_map(|l| l.ok())
            .filter(|l| !l.trim().is_empty())
            .count();
        Ok(count)
    }

    /// Clear the log file (used after compaction).
    pub fn clear(&self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)
                .with_context(|| format!("removing {}", self.path.display()))?;
        }
        Ok(())
    }

    /// Check if the log file exists.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }
}

impl LogEntry {
    /// Create a new log entry with the current timestamp.
    pub fn new(
        agent: String,
        entry_type: EntryType,
        content: String,
        confidence: Confidence,
        anchors: Vec<String>,
        tags: Vec<String>,
    ) -> Self {
        Self {
            ts: Utc::now(),
            agent,
            entry_type,
            content,
            confidence,
            anchors,
            tags,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_serialize_entry() {
        let entry = LogEntry::new(
            "claude-code".into(),
            EntryType::Insight,
            "The redirect must use HTTP 303.".into(),
            Confidence::Observed,
            vec!["handleLogin".into()],
            vec!["http".into(), "bugfix".into()],
        );
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"type\":\"insight\""));
        assert!(json.contains("handleLogin"));
        // observed is skipped (default)
        assert!(!json.contains("confidence"));
    }

    #[test]
    fn test_serialize_inferred() {
        let entry = LogEntry::new(
            "claude-code".into(),
            EntryType::Insight,
            "Likely a CORS issue.".into(),
            Confidence::Inferred,
            vec![],
            vec![],
        );
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"confidence\":\"inferred\""));
    }

    #[test]
    fn test_deserialize_entry() {
        let json = r#"{"ts":"2026-02-25T10:30:00Z","agent":"claude-code","type":"insight","content":"test","anchors":["foo"],"tags":["bar"]}"#;
        let entry: LogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.agent, "claude-code");
        assert_eq!(entry.entry_type, EntryType::Insight);
        assert_eq!(entry.confidence, Confidence::Observed); // default
    }

    #[test]
    fn test_roundtrip_file() {
        let dir = std::env::temp_dir().join("agit_test_log");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.jsonl");

        let log = LogFile::new(path.clone());

        let e1 = LogEntry::new(
            "agent1".into(),
            EntryType::Decision,
            "Use cookies".into(),
            Confidence::Observed,
            vec![],
            vec![],
        );
        let e2 = LogEntry::new(
            "agent2".into(),
            EntryType::Failure,
            "JWT didn't work".into(),
            Confidence::Inferred,
            vec!["auth".into()],
            vec!["jwt".into()],
        );

        log.append(&e1).unwrap();
        log.append(&e2).unwrap();

        assert_eq!(log.count().unwrap(), 2);

        let all = log.read_all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].agent, "agent1");
        assert_eq!(all[1].agent, "agent2");

        let tail = log.read_tail(1).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].agent, "agent2");

        log.clear().unwrap();
        assert!(!log.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
