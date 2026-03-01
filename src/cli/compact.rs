use anyhow::{Context, Result};
use std::env;

use crate::config::Config;
use crate::core::compact::{finish_compaction, prepare_compaction};
use crate::core::shadow::Shadow;
use crate::core::staleness::check_all_staleness;
use crate::llm::{LlmConfig, ProviderKind};

pub fn run(
    file: Option<&str>,
    stale: bool,
    dry_run: bool,
    llm: bool,
    provider: Option<&str>,
    model: Option<&str>,
    api_key: Option<&str>,
) -> Result<()> {
    let cwd = env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project (run `agit init` first)")?;
    let config = Config::load(&project_root)?;
    let shadow = Shadow::new(project_root);

    // Collect targets to compact
    let targets: Vec<String> = if stale {
        let reports = check_all_staleness(&shadow, &config)?;
        let stale_targets: Vec<_> = reports.iter().filter(|r| r.is_stale()).collect();

        if stale_targets.is_empty() {
            println!("Nothing stale. Everything is up to date.");
            return Ok(());
        }

        println!("Found {} stale targets:", stale_targets.len());
        for report in &stale_targets {
            println!("  {}", report.target);
        }
        println!();

        stale_targets.iter().map(|r| r.target.clone()).collect()
    } else if let Some(file) = file {
        vec![file.to_string()]
    } else {
        anyhow::bail!("specify a target to compact, or use --stale to compact all stale targets");
    };

    if dry_run {
        // Just print prompts
        for t in &targets {
            let ctx = prepare_compaction(t, &shadow)?;
            println!("=== {} ===\n{}\n", t, ctx.prompt);
        }
        return Ok(());
    }

    if llm {
        // LLM-powered compaction
        let llm_config = resolve_llm_config(&config, provider, model, api_key)?;
        let llm_provider = llm_config.create_provider()?;

        println!("Compacting with {} ({})...\n", llm_config.provider, llm_config.model);

        let mut compacted = 0;
        let mut failed = 0;

        for (i, t) in targets.iter().enumerate() {
            print!("  [{}/{}] {} ", i + 1, targets.len(), t);
            let ctx = prepare_compaction(t, &shadow)?;

            match llm_provider.complete(&ctx.prompt) {
                Ok(new_body) => {
                    finish_compaction(t, &new_body, &shadow)?;
                    println!("✓");
                    compacted += 1;
                }
                Err(e) => {
                    println!("✗ ({})", e);
                    failed += 1;
                }
            }
        }

        println!("\nDone! {} compacted, {} failed.", compacted, failed);
    } else {
        // No --llm: explain what to do
        println!("To compact from the terminal, add --llm:");
        println!("  agit compact {} --llm --provider anthropic",
            if stale { "--stale" } else { targets.first().map(|s| s.as_str()).unwrap_or("<target>") });
        println!();
        println!("Or let your agent handle it via MCP (agit_compact → agit_compact_finish).");
        println!("To see the raw prompt: add --dry-run");
    }

    Ok(())
}

fn resolve_llm_config(
    config: &Config,
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
    cli_api_key: Option<&str>,
) -> Result<LlmConfig> {
    let provider_str = cli_provider
        .map(|s| s.to_string())
        .or_else(|| config.llm.provider.clone())
        .context("LLM provider required. Use --provider or set in config.yaml")?;

    let provider_kind: ProviderKind = provider_str.parse()?;

    let model = cli_model
        .map(|s| s.to_string())
        .or_else(|| config.llm.model.clone())
        .unwrap_or_else(|| LlmConfig::default_model(&provider_kind).to_string());

    let env_key_name = LlmConfig::env_key_name(&provider_kind);
    let api_key = cli_api_key
        .map(|s| s.to_string())
        .or_else(|| env::var(env_key_name).ok())
        .or_else(|| env::var("AGIT_API_KEY").ok())
        .context(format!(
            "API key required. Use --api-key or set {} environment variable",
            env_key_name
        ))?;

    let base_url = config.llm.base_url.clone();

    Ok(LlmConfig {
        provider: provider_kind,
        model,
        api_key,
        base_url,
        concurrency: config.llm.concurrency,
    })
}
