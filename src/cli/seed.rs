use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::core::log::{Confidence, EntryType, LogEntry, LogFile};
use crate::core::shadow::Shadow;

pub fn run(from_git: bool, from_comments: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project (run `agit init` first)")?;
    let config = Config::load(&project_root)?;
    let shadow = Shadow::new(project_root.clone());

    if !from_git && !from_comments {
        println!("Specify a seeding source:");
        println!("  agit seed --from-git          Extract signals from git history");
        println!("  agit seed --from-comments     Extract TODO/HACK/FIXME comments");
        println!();
        println!("Or let your agent seed via MCP (agit_seed tool).");
        return Ok(());
    }

    if from_git {
        seed_from_git(&shadow, &config)?;
    }

    if from_comments {
        seed_from_comments(&shadow, &config)?;
    }

    Ok(())
}

// --- Mechanical seed: git history ---

fn seed_from_git(shadow: &Shadow, config: &Config) -> Result<()> {
    println!("Seeding from git history...\n");

    let mut seeded = 0;

    // 1. Find revert commits and their affected files
    let output = std::process::Command::new("git")
        .args(["log", "--all", "--grep=revert", "-i", "--format=%H", "-50"])
        .current_dir(&shadow.project_root)
        .output()
        .context("running git log for reverts")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let revert_hashes: Vec<&str> = stdout.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();

    for hash in &revert_hashes {
        let msg_output = std::process::Command::new("git")
            .args(["log", "-1", "--format=%s", hash])
            .current_dir(&shadow.project_root)
            .output();
        let msg = match msg_output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(_) => continue,
        };

        let files_output = std::process::Command::new("git")
            .args(["diff-tree", "--no-commit-id", "-r", "--name-only", hash])
            .current_dir(&shadow.project_root)
            .output();
        let files_str = match files_output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => continue,
        };

        for file in files_str.lines() {
            let file = file.trim();
            if file.is_empty() || should_ignore(file, config) || !shadow.project_root.join(file).exists() {
                continue;
            }

            let entry = LogEntry::new(
                "agit-seed".to_string(),
                EntryType::Failure,
                format!("[seeded] Reverted commit: {}", msg),
                Confidence::Observed,
                vec![],
                vec!["seeded".to_string(), "revert".to_string()],
            );
            LogFile::new(shadow.log_path(file)).append(&entry)?;
            seeded += 1;
        }
    }

    // 2. Find fix/workaround/hack commits
    let output = std::process::Command::new("git")
        .args(["log", "--all", "--format=%H %s", "-200"])
        .current_dir(&shadow.project_root)
        .output()
        .context("running git log for insights")?;

    let log_output = String::from_utf8_lossy(&output.stdout).to_string();
    let interesting_prefixes = ["fix:", "fix(", "revert:", "workaround:", "hack:", "hotfix:", "bugfix:"];

    for line in log_output.lines() {
        let Some(space_idx) = line.find(' ') else { continue };
        let hash = &line[..space_idx];
        let msg = &line[space_idx + 1..];

        let lower_msg = msg.to_lowercase();
        if !interesting_prefixes.iter().any(|p| lower_msg.starts_with(p)) {
            continue;
        }

        let files_output = std::process::Command::new("git")
            .args(["diff-tree", "--no-commit-id", "-r", "--name-only", hash])
            .current_dir(&shadow.project_root)
            .output();
        let files_str = match files_output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => continue,
        };

        let entry_type = if lower_msg.starts_with("revert") {
            EntryType::Failure
        } else if lower_msg.starts_with("hack") || lower_msg.starts_with("workaround") {
            EntryType::Constraint
        } else {
            EntryType::Insight
        };

        for file in files_str.lines() {
            let file = file.trim();
            if file.is_empty() || should_ignore(file, config) || !shadow.project_root.join(file).exists() {
                continue;
            }

            let entry = LogEntry::new(
                "agit-seed".to_string(),
                entry_type.clone(),
                format!("[seeded] {}", msg),
                Confidence::Observed,
                vec![],
                vec!["seeded".to_string(), "git-history".to_string()],
            );
            LogFile::new(shadow.log_path(file)).append(&entry)?;
            seeded += 1;
        }
    }

    // 3. Flag high-churn files
    let output = std::process::Command::new("git")
        .args(["log", "--format=", "--name-only", "--diff-filter=M"])
        .current_dir(&shadow.project_root)
        .output()
        .context("running git log for churn")?;

    let churn_output = String::from_utf8_lossy(&output.stdout).to_string();
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    for line in churn_output.lines() {
        let line = line.trim();
        if !line.is_empty() {
            *file_counts.entry(line.to_string()).or_default() += 1;
        }
    }

    let mut high_churn: Vec<_> = file_counts.iter().filter(|(_, count)| **count >= 20).collect();
    high_churn.sort_by(|a, b| b.1.cmp(a.1));

    for (file, count) in high_churn.iter().take(20) {
        if should_ignore(file, config) || !shadow.project_root.join(file).exists() {
            continue;
        }

        let log = LogFile::new(shadow.log_path(file));
        if log.exists() {
            continue;
        }

        let entry = LogEntry::new(
            "agit-seed".to_string(),
            EntryType::Insight,
            format!("[seeded] High-churn file ({} modifications)", count),
            Confidence::Inferred,
            vec![],
            vec!["seeded".to_string(), "high-churn".to_string()],
        );
        log.append(&entry)?;
        seeded += 1;
    }

    println!("  Seeded {} entries from git history.", seeded);

    Ok(())
}

// --- Mechanical seed: code comments ---

fn seed_from_comments(shadow: &Shadow, config: &Config) -> Result<()> {
    println!("Seeding from code comments...\n");

    let patterns = &config.seed.comment_patterns;
    let pattern_regex = patterns.join("|");

    let output = std::process::Command::new("git")
        .args([
            "grep", "-n", "-E", &format!("({})", pattern_regex),
            "--",
            "*.rs", "*.ts", "*.tsx", "*.js", "*.jsx",
            "*.py", "*.go", "*.java", "*.rb", "*.c",
            "*.cpp", "*.h", "*.cs", "*.swift", "*.kt",
        ])
        .current_dir(&shadow.project_root)
        .output()
        .context("running git grep")?;

    let matches = String::from_utf8_lossy(&output.stdout);

    let mut file_comments: HashMap<String, Vec<String>> = HashMap::new();
    for line in matches.lines() {
        if let Some(colon_idx) = line.find(':') {
            let file = &line[..colon_idx];
            let rest = &line[colon_idx + 1..];
            if let Some(colon2) = rest.find(':') {
                let comment = rest[colon2 + 1..].trim().to_string();
                file_comments.entry(file.to_string()).or_default().push(comment);
            }
        }
    }

    let mut seeded = 0;
    for (file, comments) in &file_comments {
        if should_ignore(file, config) {
            continue;
        }

        for comment in comments {
            let entry = LogEntry::new(
                "agit-seed".to_string(),
                EntryType::Insight,
                format!("[seeded] {}", comment),
                Confidence::Observed,
                vec![],
                vec!["seeded".to_string(), "comment".to_string()],
            );
            LogFile::new(shadow.log_path(file)).append(&entry)?;
            seeded += 1;
        }
    }

    println!("  Seeded {} entries from {} files.", seeded, file_comments.len());

    Ok(())
}

// --- MCP seed data gathering ---

/// Gather raw seed data for the MCP tool. Returns structured data that the
/// agent can analyze and turn into real insights via agit_write() calls.
pub fn gather_seed_data(shadow: &Shadow, source: &str) -> Result<String> {
    match source {
        "git" => gather_git_seed_data(shadow),
        "comments" => gather_comment_seed_data(shadow),
        "scan" => gather_scan_data(shadow),
        other => anyhow::bail!("unknown seed source: {}. Use 'git', 'comments', or 'scan'.", other),
    }
}

fn gather_git_seed_data(shadow: &Shadow) -> Result<String> {
    let mut output = String::new();
    output.push_str("# Git History Analysis\n\n");
    output.push_str("Analyze this data and call agit_write() for each meaningful insight.\n\n");

    // Reverts
    output.push_str("## Reverted Commits\n\n");
    let cmd_output = std::process::Command::new("git")
        .args(["log", "--all", "--grep=revert", "-i", "--format=%H|%s|%b", "-30"])
        .current_dir(&shadow.project_root)
        .output()
        .context("git log for reverts")?;

    let reverts = String::from_utf8_lossy(&cmd_output.stdout);
    if reverts.trim().is_empty() {
        output.push_str("(none found)\n\n");
    } else {
        for line in reverts.lines() {
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            if parts.len() >= 2 {
                let hash = parts[0];
                let subject = parts[1];
                let body = parts.get(2).unwrap_or(&"");

                let files_output = std::process::Command::new("git")
                    .args(["diff-tree", "--no-commit-id", "-r", "--name-only", hash])
                    .current_dir(&shadow.project_root)
                    .output();
                let files = files_output
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();

                output.push_str(&format!("### {}\n", subject));
                if !body.is_empty() {
                    output.push_str(&format!("{}\n", body));
                }
                output.push_str(&format!("Files: {}\n\n", files.replace('\n', ", ")));
            }
        }
    }

    // Fix/workaround commits
    output.push_str("## Fix and Workaround Commits\n\n");
    let cmd_output = std::process::Command::new("git")
        .args(["log", "--all", "--format=%H|%s", "-200"])
        .current_dir(&shadow.project_root)
        .output()
        .context("git log")?;

    let interesting_prefixes = ["fix:", "fix(", "revert:", "workaround:", "hack:", "hotfix:", "bugfix:"];
    for line in String::from_utf8_lossy(&cmd_output.stdout).lines() {
        let parts: Vec<&str> = line.splitn(2, '|').collect();
        if parts.len() < 2 { continue; }
        let hash = parts[0];
        let msg = parts[1];

        if !interesting_prefixes.iter().any(|p| msg.to_lowercase().starts_with(p)) {
            continue;
        }

        let files_output = std::process::Command::new("git")
            .args(["diff-tree", "--no-commit-id", "-r", "--name-only", hash])
            .current_dir(&shadow.project_root)
            .output();
        let files = files_output
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        output.push_str(&format!("- **{}** (files: {})\n", msg, files.replace('\n', ", ")));
    }

    // High-churn
    output.push_str("\n## High-Churn Files\n\n");
    let cmd_output = std::process::Command::new("git")
        .args(["log", "--format=", "--name-only", "--diff-filter=M"])
        .current_dir(&shadow.project_root)
        .output()
        .context("git log for churn")?;

    let mut file_counts: HashMap<String, usize> = HashMap::new();
    for line in String::from_utf8_lossy(&cmd_output.stdout).lines() {
        let line = line.trim();
        if !line.is_empty() {
            *file_counts.entry(line.to_string()).or_default() += 1;
        }
    }

    let mut sorted: Vec<_> = file_counts.iter().filter(|(_, c)| **c >= 10).collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (file, count) in sorted.iter().take(30) {
        output.push_str(&format!("- {} ({} modifications)\n", file, count));
    }

    Ok(output)
}

fn gather_comment_seed_data(shadow: &Shadow) -> Result<String> {
    let config = Config::load(&shadow.project_root)?;
    let patterns = &config.seed.comment_patterns;
    let pattern_regex = patterns.join("|");

    let cmd_output = std::process::Command::new("git")
        .args([
            "grep", "-n", "-E", &format!("({})", pattern_regex),
            "--", "*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.rb",
        ])
        .current_dir(&shadow.project_root)
        .output()
        .context("git grep")?;

    let matches = String::from_utf8_lossy(&cmd_output.stdout);

    let mut output = String::new();
    output.push_str("# Code Comments\n\n");
    output.push_str("Analyze these and call agit_write() for comments with real insights.\n\n");
    output.push_str("```\n");
    output.push_str(&matches);
    output.push_str("```\n");

    Ok(output)
}

fn gather_scan_data(shadow: &Shadow) -> Result<String> {
    let config = Config::load(&shadow.project_root)?;
    let source_files = list_source_files(&shadow.project_root)?;
    let tracked = shadow.tracked_files().unwrap_or_default();
    let tracked_set: std::collections::HashSet<&str> = tracked.iter().map(|s| s.as_str()).collect();

    let mut has_log: std::collections::HashSet<String> = std::collections::HashSet::new();
    for file in &source_files {
        if shadow.log_path(file).exists() {
            has_log.insert(file.clone());
        }
    }

    let total = source_files.len();
    let with_knowledge = tracked.len();
    let with_logs = has_log.iter().filter(|f| !tracked_set.contains(f.as_str())).count();
    let bare = total - with_knowledge - with_logs;

    let mut output = String::new();
    output.push_str("# Project Scan\n\n");
    output.push_str(&format!("{} source files: {} with knowledge, {} with logs only, {} bare\n\n", total, with_knowledge, with_logs, bare));

    output.push_str("## Files needing knowledge\n\n");
    for file in &source_files {
        if should_ignore(file, &config) { continue; }
        let status = if tracked_set.contains(file.as_str()) {
            "ok"
        } else if has_log.contains(file) {
            "COMPACT"
        } else {
            "BOOTSTRAP"
        };
        if status != "ok" {
            let full_path = shadow.project_root.join(file);
            let size = std::fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
            let size_str = if size > 10_000 { format!(" ({}KB)", size / 1024) } else { String::new() };
            output.push_str(&format!("- [{}] `{}`{}\n", status, file, size_str));
        }
    }

    output.push_str("\nFor [BOOTSTRAP] files: read the source, call agit_write() with insights, then agit_compact().\n");
    output.push_str("For [COMPACT] files: call agit_compact() directly.\n");

    Ok(output)
}

// --- Utilities ---

fn list_source_files(project_root: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(project_root)
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let files: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with(".agit/"))
                .filter(|l| is_source_file(l))
                .collect();
            if !files.is_empty() {
                return Ok(files);
            }
        }
    }

    let mut files = Vec::new();
    walk_source_files(project_root, project_root, &mut files)?;
    Ok(files)
}

pub fn is_source_file(path: &str) -> bool {
    let source_extensions = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java",
        ".rb", ".c", ".cpp", ".h", ".hpp", ".cs", ".swift", ".kt",
        ".scala", ".clj", ".ex", ".exs", ".zig", ".nim", ".lua",
        ".sh", ".bash", ".zsh", ".fish",
        ".yaml", ".yml", ".toml", ".json", ".xml",
        ".sql", ".graphql", ".proto",
        ".md", ".txt", ".rst",
        ".html", ".css", ".scss", ".less", ".vue", ".svelte",
        ".dockerfile", ".tf", ".hcl",
    ];

    let lower = path.to_lowercase();

    if source_extensions.iter().any(|ext| lower.ends_with(ext)) {
        return true;
    }

    let filename = std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(filename.as_str(),
        "dockerfile" | "makefile" | "rakefile" | "gemfile" |
        "cargo.toml" | "package.json" | "go.mod" | "build.gradle"
    )
}

fn walk_source_files(root: &Path, current: &Path, out: &mut Vec<String>) -> Result<()> {
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();

        if path.is_dir() {
            if name.starts_with('.') || matches!(name.as_ref(),
                "node_modules" | "target" | "dist" | "build" | "__pycache__" | "vendor"
            ) {
                continue;
            }
            walk_source_files(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if is_source_file(&rel) {
                out.push(rel);
            }
        }
    }
    Ok(())
}

fn should_ignore(file: &str, config: &Config) -> bool {
    config.ignore.iter().any(|pattern| {
        let pattern = pattern.trim_end_matches('/').trim_end_matches('*');
        file.contains(pattern)
    })
}
