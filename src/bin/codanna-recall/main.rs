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
    /// Explicit privacy/retrieval scope, shared by both clients (for example assign).
    #[arg(long)]
    workspace: String,
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
    validate_workspace(&cli.workspace)?;
    let path = cli.index.map(Ok).unwrap_or_else(|| {
        dirs::data_local_dir()
            .map(|p| p.join("codanna").join("recall-v1"))
            .context("no local data directory; supply --index")
    })?;
    let create = matches!(&cli.command, Command::Import { .. });
    let store = store::Store::open(&path, create)?;
    let output = match cli.command {
        Command::Import { provider, file } => store.import(&cli.workspace, provider, &file)?,
        Command::Search {
            query,
            limit,
            role,
            provider,
        } => store.search(&cli.workspace, &query, limit, role.as_deref(), provider)?,
        Command::Read { id } => store.read(&cli.workspace, &id)?,
        Command::Forget { source_id } => {
            store.forget(&cli.workspace, &source_id)?;
            json!({"forgotten_source": source_id, "original_file_modified": false})
        }
        Command::Serve => {
            server::serve(store, cli.workspace).await?;
            return Ok(());
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
mod tests;
