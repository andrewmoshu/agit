pub mod init;
pub mod read;
pub mod write;
pub mod compact;
pub mod status;
pub mod diff;
pub mod seed;
pub mod mv;
pub mod sync;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "agit",
    about = "Knowledge layer for AI agents — tracks why code is the way it is",
    long_about = "\
Git tracks what changed. agit tracks why it's like this.

Human workflow:
  agit init                              Setup + auto-seed from git
  agit status                            What needs attention?
  agit compact --stale --llm             Update stale knowledge

Agent workflow (via MCP):
  Agents read/write/compact knowledge automatically through agit serve.
  Run `agit init` in your repo — it registers the MCP server with your agent.",
    version,
    after_help = "\
Examples:
  agit init                                          Setup (auto-seeds from git history)
  agit status                                        Check what's stale
  agit compact --stale --llm --provider anthropic    Fix stale knowledge
  agit read src/main.rs                              View knowledge for a file"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize agit in the current project
    Init,

    /// Check which files have stale knowledge
    Status,

    /// Update stale knowledge by re-analyzing source files
    Compact {
        /// Source file path, or --stale to compact all stale files
        file: Option<String>,

        /// Compact all stale files
        #[arg(long)]
        stale: bool,

        /// Don't run LLM — just print the compaction prompt
        #[arg(long)]
        dry_run: bool,

        /// Use LLM to compact (required for CLI compaction)
        #[arg(long)]
        llm: bool,

        /// LLM provider (anthropic, openai, custom)
        #[arg(long)]
        provider: Option<String>,

        /// LLM model name
        #[arg(long)]
        model: Option<String>,

        /// API key (or use env var)
        #[arg(long)]
        api_key: Option<String>,
    },

    /// Extract signals from git history and code comments
    Seed {
        /// Seed from git history (reverts, fixes, churn)
        #[arg(long)]
        from_git: bool,

        /// Seed from code comments (TODO, HACK, FIXME, etc.)
        #[arg(long)]
        from_comments: bool,
    },

    /// Show what the last compaction changed for a file
    Diff {
        /// Source file path
        file: String,
    },

    /// Read knowledge for a file, directory, or root
    Read {
        /// Target path: file (src/main.rs), directory (src/auth/), or root (/)
        file: String,

        /// Include all log entries (default: last 5)
        #[arg(long)]
        deep: bool,

        /// Depth 0: knowledge .md only, no log entries
        #[arg(long = "depth")]
        depth: Option<u32>,
    },

    /// Log an observation about a file (mostly used by agents)
    Write {
        /// Source file path (relative to project root)
        file: String,

        /// Which agent is writing
        #[arg(long)]
        agent: String,

        /// Entry type: insight, decision, failure, constraint, relationship
        #[arg(long, rename_all = "snake_case")]
        r#type: String,

        /// The insight content
        #[arg(long)]
        content: String,

        /// Confidence: observed (default) or inferred
        #[arg(long, default_value = "observed")]
        confidence: String,

        /// Anchors (code element names), comma-separated
        #[arg(long, value_delimiter = ',')]
        anchors: Option<Vec<String>>,

        /// Tags, comma-separated
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
    },

    /// Move knowledge when a file is renamed
    Mv {
        /// Old source file path
        old: String,

        /// New source file path
        new: String,
    },

    /// Sync shadow files with the latest git commit (auto-detects renames/deletes)
    Sync,

    /// Start the MCP server for AI agents (JSON-RPC over stdio)
    Serve,
}
