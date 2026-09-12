//! Opt-in companion CLI; never changes Codanna's existing indexes.
#[path = "../knowledge/mod.rs"]
mod knowledge;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::io::Write;

#[derive(Parser)]
#[command(about = "Evidence-linked code and documentation snapshots", version)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Build a replacement snapshot from the existing Codanna symbol index.
    Index {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        repo: String,
        /// Complete `codanna dump` JSONL file; otherwise invoke `codanna dump`.
        #[arg(long)]
        dump: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Return incoming/outgoing references; ambiguous names require an entity id.
    Links {
        #[arg(long, default_value = ".codanna/knowledge.json")]
        graph: PathBuf,
        entity: String,
    },
}

fn run() -> knowledge::Result<()> {
    let result = match Cli::parse().command {
        Action::Index { root, repo, dump, out } => {
            let input = knowledge::io::input(&root, &repo, dump.as_deref())?;
            let graph = knowledge::links::build(&input)?;
            let out = out.unwrap_or_else(|| root.join(".codanna/knowledge.json"));
            knowledge::io::save(&graph, &out)?;
            serde_json::json!({"nodes":graph.nodes.len(),"edges":graph.edges.len(),"unresolved":graph.unresolved.len(),"snapshot":out,"limitations":graph.limitations})
        }
        Action::Links { graph, entity } => {
            let graph = knowledge::io::load(&graph)?;
            let node = graph.resolve(&entity)?;
            serde_json::json!({"node":node,"incoming":graph.edges.iter().filter(|e| e.to == node.id).collect::<Vec<_>>(),"outgoing":graph.edges.iter().filter(|e| e.from == node.id).collect::<Vec<_>>(),"unresolved":graph.unresolved.iter().filter(|r| r.from == node.id).collect::<Vec<_>>(),"limitations":graph.limitations})
        }
    };
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &result)?;
    writeln!(out)?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe) { return std::process::ExitCode::SUCCESS; }
            eprintln!("knowledge: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
