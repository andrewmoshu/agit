use anyhow::{Context, Result};
use serde_json::Value;
use std::env;
use std::path::Path;

use crate::config::Config;
use crate::core::knowledge::KnowledgeFile;
use crate::core::shadow::Shadow;

pub fn run() -> Result<()> {
    let cwd = env::current_dir().context("getting current directory")?;

    let shadow = Shadow::new(cwd.clone());

    if shadow.is_initialized() {
        println!("agit is already initialized in this project.");
        let registered = register_mcp_servers(&cwd);
        if !registered.is_empty() {
            println!("\nRegistered agit MCP server with:");
            for name in &registered {
                println!("  {}", name);
            }
        }
        return Ok(());
    }

    // --- Detect project context ---
    let project_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    let is_git_repo = cwd.join(".git").is_dir();
    let git_stats = if is_git_repo {
        detect_git_stats(&cwd)
    } else {
        None
    };

    // --- Create .agit/ ---
    shadow.init()?;

    let config = Config::default();
    config.save(&cwd)?;

    let root_kf = KnowledgeFile::new_root(&project_name, shadow.root_knowledge_path());
    root_kf.save()?;

    if is_git_repo {
        write_gitattributes(&cwd)?;
        install_post_commit_hook(&cwd)?;
    }

    let registered = register_mcp_servers(&cwd);

    // --- Output ---
    println!("Initialized agit in .agit/");

    if let Some(stats) = &git_stats {
        if stats.commit_count > 0 || stats.file_count > 0 {
            println!("\n  {} commits, {} tracked files", stats.commit_count, stats.file_count);
            if stats.revert_count > 0 {
                println!("  {} reverts", stats.revert_count);
            }
            if stats.fix_count > 0 {
                println!("  {} fixes/workarounds", stats.fix_count);
            }
            if stats.high_churn_count > 0 {
                println!("  {} high-churn files", stats.high_churn_count);
            }
        }
    }

    if !registered.is_empty() {
        println!("\nMCP server registered with:");
        for name in &registered {
            println!("  {}", name);
        }
    }

    // --- Auto-seed from git if there's history ---
    let has_git_signals = git_stats.as_ref().is_some_and(|s| s.has_seedable_content());

    if has_git_signals {
        println!("\nSeeding from git history...");
        crate::cli::seed::run(true, true)?;
    }

    println!("\nKnowledge will accumulate as your agents work.");
    println!("Run `agit status` anytime to check.");

    Ok(())
}

// --- Project detection ---

struct GitStats {
    commit_count: usize,
    file_count: usize,
    revert_count: usize,
    fix_count: usize,
    high_churn_count: usize,
}

impl GitStats {
    fn has_seedable_content(&self) -> bool {
        self.commit_count > 5 || self.revert_count > 0 || self.fix_count > 0
    }
}

fn detect_git_stats(project_root: &Path) -> Option<GitStats> {
    let commit_count = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(project_root)
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0);

    let file_count = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(project_root)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0);

    let revert_count = std::process::Command::new("git")
        .args(["log", "--all", "--grep=revert", "-i", "--oneline"])
        .current_dir(project_root)
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count()
        })
        .unwrap_or(0);

    let fix_count = std::process::Command::new("git")
        .args(["log", "--all", "--oneline", "-200"])
        .current_dir(project_root)
        .output()
        .ok()
        .map(|o| {
            let prefixes = ["fix:", "fix(", "workaround:", "hack:", "hotfix:", "bugfix:"];
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|line| {
                    let lower = line.to_lowercase();
                    if let Some(msg_start) = lower.find(' ') {
                        let msg = lower[msg_start..].trim();
                        prefixes.iter().any(|p| msg.starts_with(p))
                    } else {
                        false
                    }
                })
                .count()
        })
        .unwrap_or(0);

    let high_churn_count = std::process::Command::new("git")
        .args(["log", "--format=", "--name-only", "--diff-filter=M"])
        .current_dir(project_root)
        .output()
        .ok()
        .map(|o| {
            let mut counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let line = line.trim();
                if !line.is_empty() {
                    *counts.entry(line.to_string()).or_default() += 1;
                }
            }
            counts.values().filter(|c| **c >= 20).count()
        })
        .unwrap_or(0);

    Some(GitStats {
        commit_count,
        file_count,
        revert_count,
        fix_count,
        high_churn_count,
    })
}

// --- .gitattributes ---

fn write_gitattributes(project_root: &Path) -> Result<()> {
    let gitattr_path = project_root.join(".gitattributes");

    let agit_entries = "\n# agit: mark log files as generated (reduces PR noise)\n.agit/**/*.jsonl linguist-generated=true\n# agit: union-merge knowledge files (avoids conflicts across branches)\n.agit/**/*.md merge=union\n";

    if gitattr_path.exists() {
        let contents = std::fs::read_to_string(&gitattr_path)?;
        if contents.contains(".agit/") {
            return Ok(());
        }
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&gitattr_path)?;
        std::io::Write::write_all(&mut file, agit_entries.as_bytes())?;
    } else {
        std::fs::write(&gitattr_path, agit_entries.trim_start())?;
    }

    Ok(())
}

// --- Post-commit hook ---

fn install_post_commit_hook(project_root: &Path) -> Result<()> {
    let hooks_dir = project_root.join(".git").join("hooks");
    if !hooks_dir.is_dir() {
        return Ok(());
    }

    let hook_path = hooks_dir.join("post-commit");
    let agit_line = "agit sync 2>/dev/null || true";

    if hook_path.exists() {
        let contents = std::fs::read_to_string(&hook_path)?;
        if contents.contains("agit sync") {
            return Ok(()); // already installed
        }
        // Append to existing hook
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&hook_path)?;
        std::io::Write::write_all(&mut file, format!("\n# agit: auto-sync renames/deletes\n{}\n", agit_line).as_bytes())?;
    } else {
        std::fs::write(&hook_path, format!("#!/bin/sh\n# agit: auto-sync renames/deletes\n{}\n", agit_line))?;
        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    Ok(())
}

// --- MCP registration ---

fn agit_mcp_entry() -> Value {
    serde_json::json!({
        "command": "agit",
        "args": ["serve"]
    })
}

struct AgentConfig {
    name: &'static str,
    config_dir: &'static str,
    config_file: &'static str,
    mcp_key: &'static str,
}

const AGENT_CONFIGS: &[AgentConfig] = &[
    AgentConfig {
        name: "Claude Code",
        config_dir: ".claude",
        config_file: "settings.json",
        mcp_key: "mcpServers",
    },
    AgentConfig {
        name: "Cursor",
        config_dir: ".cursor",
        config_file: "mcp.json",
        mcp_key: "mcpServers",
    },
    AgentConfig {
        name: "VS Code (Copilot)",
        config_dir: ".vscode",
        config_file: "mcp.json",
        mcp_key: "servers",
    },
    AgentConfig {
        name: "Windsurf",
        config_dir: ".windsurf",
        config_file: "mcp.json",
        mcp_key: "mcpServers",
    },
];

fn register_mcp_servers(project_root: &Path) -> Vec<String> {
    let mut registered = Vec::new();

    for agent in AGENT_CONFIGS {
        let config_dir = project_root.join(agent.config_dir);
        if !config_dir.is_dir() {
            continue;
        }

        match register_with_agent(project_root, agent) {
            Ok(true) => registered.push(agent.name.to_string()),
            Ok(false) => {}
            Err(e) => {
                eprintln!("warning: failed to register with {}: {}", agent.name, e);
            }
        }
    }

    registered
}

fn register_with_agent(project_root: &Path, agent: &AgentConfig) -> Result<bool> {
    let config_path = project_root
        .join(agent.config_dir)
        .join(agent.config_file);

    let mut config: Value = if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("parsing {}", config_path.display()))?
    } else {
        serde_json::json!({})
    };

    let servers = config
        .as_object_mut()
        .context("config is not an object")?
        .entry(agent.mcp_key)
        .or_insert_with(|| serde_json::json!({}));

    let servers_obj = servers
        .as_object_mut()
        .context("mcpServers is not an object")?;

    if servers_obj.contains_key("agit") {
        return Ok(false);
    }

    servers_obj.insert("agit".to_string(), agit_mcp_entry());

    let formatted = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, formatted)
        .with_context(|| format!("writing {}", config_path.display()))?;

    Ok(true)
}
