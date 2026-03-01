use anyhow::{Context, Result};
use std::env;
use crate::config::Config;
use crate::core::knowledge::KnowledgeFile;
use crate::core::log::{Confidence, LogFile};
use crate::core::shadow::{Shadow, TargetScope};

pub fn run(target: &str, deep: bool, depth: Option<u32>) -> Result<()> {
    let cwd = env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project (run `agit init` first)")?;
    let config = Config::load(&project_root)?;
    let shadow = Shadow::new(project_root);

    let is_knowledge_only = depth == Some(0);
    let is_deep = deep;
    let scope = TargetScope::parse(target);

    let mut output = String::new();

    match &scope {
        TargetScope::File(_) => {
            // Hierarchical knowledge (root → dirs → file)
            let hierarchy = shadow.knowledge_hierarchy(target);
            for path in &hierarchy {
                if path.exists() {
                    let kf = KnowledgeFile::load(path)?;
                    if !output.is_empty() {
                        output.push_str("\n---\n\n");
                    }
                    output.push_str(&kf.body);
                }
            }
        }
        TargetScope::Dir(d) => {
            let path = shadow.dir_knowledge_path(d);
            if path.exists() {
                let kf = KnowledgeFile::load(&path)?;
                output.push_str(&kf.body);
            }
        }
        TargetScope::Root => {
            let path = shadow.root_knowledge_path();
            if path.exists() {
                let kf = KnowledgeFile::load(&path)?;
                output.push_str(&kf.body);
            }
        }
    }

    // Add log entries (unless depth=0)
    if !is_knowledge_only {
        let log_path = shadow.resolve_log_path(target);
        let log = LogFile::new(log_path);

        if log.exists() {
            let entries = if is_deep {
                log.read_all()?
            } else {
                log.read_tail(config.read.shallow_log_entries)?
            };

            if !entries.is_empty() {
                output.push_str("\n---\n\n## Recent Log\n\n");
                for entry in &entries {
                    let confidence_marker = if entry.confidence == Confidence::Inferred {
                        " [inferred]"
                    } else {
                        ""
                    };
                    let anchors = if entry.anchors.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", entry.anchors.join(", "))
                    };
                    output.push_str(&format!(
                        "- **[{}]** {}{}{}\n",
                        entry.entry_type, entry.content, anchors, confidence_marker
                    ));
                }
            }
        }
    }

    if output.trim().is_empty() {
        println!("No knowledge found for {}.", target);
        println!("Use `agit write {} ...` to record knowledge.", target);
    } else {
        print!("{}", output);
    }

    Ok(())
}
