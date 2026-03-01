use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// agit configuration — loaded from .agit/config.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,

    #[serde(default)]
    pub compaction: CompactionConfig,

    #[serde(default)]
    pub read: ReadConfig,

    #[serde(default)]
    pub seed: SeedConfig,

    #[serde(default)]
    pub llm: LlmSeedConfig,

    #[serde(default)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Recommend compaction after N log entries
    #[serde(default = "default_log_threshold")]
    pub log_threshold: usize,

    /// Keep compaction archives for N days
    #[serde(default = "default_archive_retention_days")]
    pub archive_retention_days: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadConfig {
    /// Default read depth: "shallow", "deep", or "knowledge_only"
    #[serde(default = "default_depth")]
    pub default_depth: ReadDepth,

    /// How many recent log entries to include in shallow read
    #[serde(default = "default_shallow_log_entries")]
    pub shallow_log_entries: usize,

    /// Max hierarchy depth (root + N dir levels + file)
    #[serde(default = "default_max_hierarchy_depth")]
    pub max_hierarchy_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedConfig {
    #[serde(default = "default_true")]
    pub from_git: bool,

    #[serde(default = "default_true")]
    pub from_comments: bool,

    #[serde(default = "default_comment_patterns")]
    pub comment_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSeedConfig {
    /// LLM provider: anthropic, openai, custom
    #[serde(default)]
    pub provider: Option<String>,

    /// Model name (provider-specific default if omitted)
    #[serde(default)]
    pub model: Option<String>,

    /// Custom base URL (for OpenAI-compatible endpoints)
    #[serde(default)]
    pub base_url: Option<String>,

    /// Max concurrent API requests during seed
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadDepth {
    Shallow,
    Deep,
    KnowledgeOnly,
}

// --- Defaults ---

fn default_version() -> u32 {
    1
}
fn default_log_threshold() -> usize {
    10
}
fn default_archive_retention_days() -> u64 {
    90
}
fn default_depth() -> ReadDepth {
    ReadDepth::Shallow
}
fn default_shallow_log_entries() -> usize {
    5
}
fn default_max_hierarchy_depth() -> usize {
    3
}
fn default_true() -> bool {
    true
}
fn default_concurrency() -> usize {
    3
}
fn default_comment_patterns() -> Vec<String> {
    vec![
        "TODO".into(),
        "HACK".into(),
        "FIXME".into(),
        "NOTE".into(),
        "IMPORTANT".into(),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: default_version(),
            compaction: CompactionConfig::default(),
            read: ReadConfig::default(),
            seed: SeedConfig::default(),
            llm: LlmSeedConfig::default(),
            ignore: vec![
                "node_modules/".into(),
                "dist/".into(),
                "*.generated.*".into(),
                "package-lock.json".into(),
            ],
        }
    }
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            log_threshold: default_log_threshold(),
            archive_retention_days: default_archive_retention_days(),
        }
    }
}

impl Default for ReadConfig {
    fn default() -> Self {
        Self {
            default_depth: default_depth(),
            shallow_log_entries: default_shallow_log_entries(),
            max_hierarchy_depth: default_max_hierarchy_depth(),
        }
    }
}

impl Default for SeedConfig {
    fn default() -> Self {
        Self {
            from_git: true,
            from_comments: true,
            comment_patterns: default_comment_patterns(),
        }
    }
}

impl Default for LlmSeedConfig {
    fn default() -> Self {
        Self {
            provider: None,
            model: None,
            base_url: None,
            concurrency: default_concurrency(),
        }
    }
}

impl Config {
    /// Load config from .agit/config.yaml, or return defaults if it doesn't exist.
    pub fn load(project_root: &Path) -> Result<Self> {
        let config_path = project_root.join(".agit").join("config.yaml");
        if config_path.exists() {
            let contents = std::fs::read_to_string(&config_path)
                .with_context(|| format!("reading {}", config_path.display()))?;
            let config: Config = serde_yaml::from_str(&contents)
                .with_context(|| format!("parsing {}", config_path.display()))?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Write config to .agit/config.yaml
    pub fn save(&self, project_root: &Path) -> Result<()> {
        let config_path = project_root.join(".agit").join("config.yaml");
        let contents = serde_yaml::to_string(self).context("serializing config")?;
        std::fs::write(&config_path, contents)
            .with_context(|| format!("writing {}", config_path.display()))?;
        Ok(())
    }

    /// Find the project root by walking up from cwd looking for .agit/
    pub fn find_project_root(from: &Path) -> Option<PathBuf> {
        let mut current = from.to_path_buf();
        loop {
            if current.join(".agit").is_dir() {
                return Some(current);
            }
            if !current.pop() {
                return None;
            }
        }
    }
}
