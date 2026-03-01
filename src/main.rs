mod cli;
mod config;
mod core;
mod git;
mod llm;
mod mcp;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init => cli::init::run(),

        Commands::Read { ref file, deep, depth } => {
            cli::read::run(file, deep, depth)
        }

        Commands::Write {
            ref file,
            ref agent,
            ref r#type,
            ref content,
            ref confidence,
            ref anchors,
            ref tags,
        } => cli::write::run(
            file,
            agent,
            r#type,
            content,
            confidence,
            anchors.clone(),
            tags.clone(),
        ),

        Commands::Compact {
            ref file,
            stale,
            dry_run,
            llm,
            ref provider,
            ref model,
            ref api_key,
        } => cli::compact::run(
            file.as_deref(),
            stale,
            dry_run,
            llm,
            provider.as_deref(),
            model.as_deref(),
            api_key.as_deref(),
        ),

        Commands::Status => cli::status::run(),

        Commands::Diff { ref file } => cli::diff::run(file),

        Commands::Seed {
            from_git,
            from_comments,
        } => cli::seed::run(from_git, from_comments),

        Commands::Mv { ref old, ref new } => cli::mv::run(old, new),

        Commands::Sync => cli::sync::run(),

        Commands::Serve => mcp::serve(),
    };

    if let Err(e) = result {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}
