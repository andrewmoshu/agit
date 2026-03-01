use anyhow::{Context, Result};
use chrono::Utc;
use crate::core::knowledge::KnowledgeFile;
use crate::core::log::LogFile;
use crate::core::shadow::{Shadow, TargetScope};

// --- Compaction prompts ---

fn build_file_compaction_prompt(
    source_content: &str,
    knowledge_body: &str,
    log_entries_text: &str,
    file_path: &str,
) -> String {
    format!(
        r#"You are compacting knowledge for {file_path}.

CURRENT SOURCE FILE:
```
{source_content}
```

EXISTING KNOWLEDGE (.md):
{knowledge_body}

LOG ENTRIES (.jsonl):
{log_entries_text}

Produce a new knowledge file body (markdown, no frontmatter) that:
1. Retains every insight STILL TRUE based on the current code
2. Resolves contradictions (current code is ground truth)
3. Removes entries about code that no longer exists
4. Removes trivial entries with no lasting value
5. Merges duplicate/overlapping entries
6. Promotes validated log insights into structured sections
7. Uses these sections as appropriate: Intent, Decisions, Constraints, Lessons Learned, Relationships, Gotchas, Conventions

IMPORTANT:
- When in doubt, KEEP the entry. Better to retain a possibly-useful insight than lose a definitely-useful one. Compaction is conservative.
- Prioritize CURRENT CODE REALITY over agent theories. If a log entry says "this doesn't work because of X" but the current code does X and passes tests, the entry was likely wrong — prune or demote it.
- Weight entries with confidence:"observed" higher than confidence:"inferred". An observed fact (test failed, function returned null) is reliable. An inferred theory (probably because of CORS) may be wrong.
- Start with: # {file_path}
"#
    )
}

fn build_dir_compaction_prompt(
    child_summaries: &str,
    knowledge_body: &str,
    log_entries_text: &str,
    dir_path: &str,
) -> String {
    format!(
        r#"You are compacting directory-level knowledge for {dir_path}/.

CHILD FILE KNOWLEDGE:
{child_summaries}

EXISTING DIRECTORY KNOWLEDGE (.md):
{knowledge_body}

LOG ENTRIES (.jsonl):
{log_entries_text}

Produce a new directory knowledge body (markdown, no frontmatter) that:
1. Summarizes cross-cutting patterns across files in this directory
2. Captures module-level architecture decisions and conventions
3. Notes gotchas that span multiple files
4. Integrates log entries about directory-level concerns
5. Does NOT repeat file-level details — focus on the bigger picture

Use these sections as appropriate: Purpose, Architecture Decisions, Conventions, Gotchas
Start with: # {dir_path}/
"#
    )
}

fn build_root_compaction_prompt(
    child_summaries: &str,
    knowledge_body: &str,
    log_entries_text: &str,
    project_name: &str,
) -> String {
    format!(
        r#"You are compacting root-level project knowledge for {project_name}.

DIRECTORY SUMMARIES:
{child_summaries}

EXISTING PROJECT KNOWLEDGE (.md):
{knowledge_body}

LOG ENTRIES (.jsonl):
{log_entries_text}

Produce a new project knowledge body (markdown, no frontmatter) that:
1. Captures project-wide conventions and patterns
2. Notes the technology stack and key architectural decisions
3. Documents known landmines and cross-cutting concerns
4. Integrates log entries about project-level insights

Use these sections as appropriate: Stack, Architecture, Conventions, Known Landmines
Start with: # Project Knowledge
"#
    )
}

// --- Context ---

/// Context returned by prepare_compaction — contains the prompt for the LLM.
pub struct CompactionContext {
    pub prompt: String,
}

// --- Prepare ---

/// Prepare compaction for any target: file, directory, or root.
///
/// Detects scope from the target path:
///   "src/auth/login.ts" → file compaction
///   "src/auth/"          → directory compaction
///   "/"                  → root compaction
pub fn prepare_compaction(
    target: &str,
    shadow: &Shadow,
) -> Result<CompactionContext> {
    match TargetScope::parse(target) {
        TargetScope::File(ref f) => prepare_file_compaction(f, shadow),
        TargetScope::Dir(ref d) => prepare_dir_compaction(d, shadow),
        TargetScope::Root => prepare_root_compaction(shadow),
    }
}

fn prepare_file_compaction(source_rel: &str, shadow: &Shadow) -> Result<CompactionContext> {
    let source_path = shadow.project_root.join(source_rel);
    let knowledge_path = shadow.knowledge_path(source_rel);
    let log_path = shadow.log_path(source_rel);

    let source_content = std::fs::read_to_string(&source_path)
        .with_context(|| format!("reading source file {}", source_path.display()))?;

    let existing_knowledge = if knowledge_path.exists() {
        KnowledgeFile::load(&knowledge_path)?
    } else {
        KnowledgeFile::new_for_file(source_rel, knowledge_path.clone())
    };

    let log = LogFile::new(log_path.clone());
    let entries = log.read_all()?;
    let log_text = format_log_entries(&entries);

    let prompt = build_file_compaction_prompt(
        &source_content,
        &existing_knowledge.body,
        &log_text,
        source_rel,
    );

    Ok(CompactionContext { prompt })
}

fn prepare_dir_compaction(dir_rel: &str, shadow: &Shadow) -> Result<CompactionContext> {
    let knowledge_path = shadow.dir_knowledge_path(dir_rel);
    let log_path = shadow.dir_log_path(dir_rel);

    let existing_knowledge = if knowledge_path.exists() {
        KnowledgeFile::load(&knowledge_path)?
    } else {
        KnowledgeFile::new_for_dir(dir_rel, knowledge_path.clone())
    };

    let log = LogFile::new(log_path.clone());
    let entries = log.read_all()?;
    let log_text = format_log_entries(&entries);

    let child_summaries = collect_child_file_summaries(dir_rel, shadow)?;

    let prompt = build_dir_compaction_prompt(
        &child_summaries,
        &existing_knowledge.body,
        &log_text,
        dir_rel,
    );

    Ok(CompactionContext { prompt })
}

fn prepare_root_compaction(shadow: &Shadow) -> Result<CompactionContext> {
    let knowledge_path = shadow.root_knowledge_path();
    let log_path = shadow.root_log_path();

    let project_name = shadow.project_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    let existing_knowledge = if knowledge_path.exists() {
        KnowledgeFile::load(&knowledge_path)?
    } else {
        KnowledgeFile::new_root(&project_name, knowledge_path.clone())
    };

    let log = LogFile::new(log_path.clone());
    let entries = log.read_all()?;
    let log_text = format_log_entries(&entries);

    let child_summaries = collect_child_dir_summaries(shadow)?;

    let prompt = build_root_compaction_prompt(
        &child_summaries,
        &existing_knowledge.body,
        &log_text,
        &project_name,
    );

    Ok(CompactionContext { prompt })
}

// --- Finish ---

/// Finish compaction: write the new knowledge file, archive the old one, clear the log.
///
/// Takes the target path directly (not a CompactionContext) so that MCP's
/// two-step flow doesn't need to re-prepare — avoiding a race where log
/// entries written between prepare and finish would be silently cleared.
pub fn finish_compaction(
    target: &str,
    new_body: &str,
    shadow: &Shadow,
) -> Result<()> {
    let scope = TargetScope::parse(target);

    // Resolve paths
    let (knowledge_path, log_path) = match &scope {
        TargetScope::File(f) => (shadow.knowledge_path(f), shadow.log_path(f)),
        TargetScope::Dir(d) => (shadow.dir_knowledge_path(d), shadow.dir_log_path(d)),
        TargetScope::Root => (shadow.root_knowledge_path(), shadow.root_log_path()),
    };

    // 1. Archive old knowledge + log
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let archive_key = match &scope {
        TargetScope::File(f) => f.clone(),
        TargetScope::Dir(d) => format!("{}/_dir", d),
        TargetScope::Root => "_root".to_string(),
    };
    let archive_md = shadow.archive_path(&archive_key, &date);
    if knowledge_path.exists() {
        if let Some(parent) = archive_md.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&knowledge_path, &archive_md)
            .with_context(|| "archiving old knowledge")?;
    }
    // Archive log alongside the .md (tiny, enables full audit trail)
    if log_path.exists() {
        let archive_log = archive_md.with_extension("jsonl");
        if let Some(parent) = archive_log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&log_path, &archive_log)
            .with_context(|| "archiving log")?;
    }

    // 2. Write new knowledge file (pure markdown, no frontmatter)
    let kf = KnowledgeFile {
        body: new_body.to_string(),
        path: knowledge_path,
    };
    kf.save()?;

    // 3. Clear log
    let log = LogFile::new(log_path);
    log.clear()?;

    Ok(())
}

// --- Helpers ---

fn format_log_entries(entries: &[crate::core::log::LogEntry]) -> String {
    entries
        .iter()
        .map(|e| serde_json::to_string(e).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Collect child file .md summaries in a directory (non-recursive).
fn collect_child_file_summaries(dir_rel: &str, shadow: &Shadow) -> Result<String> {
    let agit_dir = shadow.agit_dir().join(dir_rel);
    if !agit_dir.is_dir() {
        return Ok("(no child knowledge files yet)".to_string());
    }

    let mut summaries = Vec::new();
    for entry in std::fs::read_dir(&agit_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() { continue; }

        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".md") || name == "_dir.md" || name == "_root.md" {
            continue;
        }

        if let Ok(kf) = KnowledgeFile::load(&path) {
            let source_name = name.trim_end_matches(".md");
            summaries.push(format!("### {}/{}\n{}", dir_rel, source_name, kf.body.trim()));
        }
    }

    // Also include child subdirectory _dir.md files
    for entry in std::fs::read_dir(&agit_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() { continue; }

        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }

        let dir_md = path.join("_dir.md");
        if dir_md.exists() {
            if let Ok(kf) = KnowledgeFile::load(&dir_md) {
                summaries.push(format!("### {}/{}/\n{}", dir_rel, name, kf.body.trim()));
            }
        }
    }

    summaries.sort();
    if summaries.is_empty() {
        Ok("(no child knowledge files yet)".to_string())
    } else {
        Ok(summaries.join("\n\n"))
    }
}

/// Collect top-level directory _dir.md summaries for root compaction.
fn collect_child_dir_summaries(shadow: &Shadow) -> Result<String> {
    let agit_dir = shadow.agit_dir();
    if !agit_dir.is_dir() {
        return Ok("(no directory summaries yet)".to_string());
    }

    let mut summaries = Vec::new();
    collect_dir_summaries_recursive(&agit_dir, &agit_dir, &mut summaries)?;

    summaries.sort();
    if summaries.is_empty() {
        Ok("(no directory summaries yet)".to_string())
    } else {
        Ok(summaries.join("\n\n"))
    }
}

fn collect_dir_summaries_recursive(
    base: &std::path::Path,
    current: &std::path::Path,
    out: &mut Vec<String>,
) -> Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() { continue; }

        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }

        let dir_md = path.join("_dir.md");
        if dir_md.exists() {
            if let Ok(kf) = KnowledgeFile::load(&dir_md) {
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                out.push(format!("### {}/\n{}", rel, kf.body.trim()));
            }
        }

        // Recurse into subdirectories
        collect_dir_summaries_recursive(base, &path, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_file_compaction_prompt() {
        let prompt = build_file_compaction_prompt(
            "fn main() {}",
            "## Intent\n\nA simple app.\n",
            r#"{"ts":"2026-01-01","agent":"test","type":"insight","content":"hello"}"#,
            "src/main.rs",
        );
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("fn main()"));
        assert!(prompt.contains("simple app"));
        assert!(prompt.contains("hello"));
        assert!(prompt.contains("confidence:\"observed\""));
    }

    #[test]
    fn test_build_dir_compaction_prompt() {
        let prompt = build_dir_compaction_prompt(
            "### src/auth/login.ts\nHandles login.",
            "## Purpose\n\nAuth module.\n",
            "",
            "src/auth",
        );
        assert!(prompt.contains("src/auth/"));
        assert!(prompt.contains("Handles login"));
        assert!(prompt.contains("Auth module"));
    }

    #[test]
    fn test_build_root_compaction_prompt() {
        let prompt = build_root_compaction_prompt(
            "### src/\nSource code.",
            "## Stack\n\nRust.\n",
            "",
            "my-project",
        );
        assert!(prompt.contains("my-project"));
        assert!(prompt.contains("Source code"));
        assert!(prompt.contains("Rust"));
    }
}
