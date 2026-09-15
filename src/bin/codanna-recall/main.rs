//! Opt-in, model-free conversation recall. Does not open code/document indexes.
mod adapter;
mod server;
mod store;

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Shared local conversation recall for Codex and Claude Code")]
struct Cli {
    /// Dedicated index directory; defaults to the user's local data directory.
    #[arg(long, global = true)]
    index: Option<PathBuf>,
    /// Optional legacy namespace. Normally derived from the opened project.
    #[arg(long, conflicts_with = "project_path")]
    workspace: Option<String>,
    /// Optional project directory; defaults to the process working directory.
    #[arg(long, conflicts_with = "workspace")]
    project_path: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Import ONE explicitly selected local JSONL transcript (no discovery/network).
    Import {
        #[arg(long, value_enum)]
        provider: adapter::Provider,
        #[arg(long)]
        file: PathBuf,
    },
    /// Search original message text. All query tokens must match.
    Search {
        query: String,
        #[arg(long, default_value_t = 8)]
        limit: usize,
        #[arg(long)]
        role: Option<String>,
        #[arg(long, value_enum)]
        provider: Option<adapter::Provider>,
    },
    /// Read one complete indexed message using its returned opaque ID.
    Read { id: String },
    /// Remove a source and all its messages from recall (not its original file).
    Forget { source_id: String },
    /// Serve read-only recall tools over stdio, pinned to this workspace.
    Serve,
}

fn validate_workspace(workspace: &str) -> Result<()> {
    ensure!(
        !workspace.trim().is_empty()
            && workspace.len() <= 128
            && !workspace.chars().any(char::is_control),
        "workspace must contain 1-128 bytes without control characters"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let workspace = match cli.workspace {
        Some(workspace) => workspace,
        None => {
            let directory = cli
                .project_path
                .map(Ok)
                .unwrap_or_else(std::env::current_dir)?;
            codanna::mcp::tools::recall::scope_for_directory(&directory)?
        }
    };
    validate_workspace(&workspace)?;
    let path = cli.index.map(Ok).unwrap_or_else(|| {
        dirs::data_local_dir()
            .map(|p| p.join("codanna").join("recall-v1"))
            .context("no local data directory; supply --index")
    })?;
    let create = matches!(&cli.command, Command::Import { .. });
    let store = store::Store::open(&path, create)?;
    let output = match cli.command {
        Command::Import { provider, file } => store.import(&workspace, provider, &file)?,
        Command::Search {
            query,
            limit,
            role,
            provider,
        } => store.search(&workspace, &query, limit, role.as_deref(), provider)?,
        Command::Read { id } => store.read(&workspace, &id)?,
        Command::Forget { source_id } => {
            store.forget(&workspace, &source_id)?;
            json!({"forgotten_source": source_id, "original_file_modified": false})
        }
        Command::Serve => {
            server::serve(store, workspace).await?;
            return Ok(());
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
mod tests;
