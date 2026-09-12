//! Opt-in companion CLI; never changes Codanna's existing indexes.
#[path = "../knowledge/context.rs"]
mod context;
#[path = "../knowledge/contracts.rs"]
mod contracts;
#[path = "../knowledge/mod.rs"]
mod knowledge;
#[path = "../knowledge/service.rs"]
mod service;
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;
#[derive(Parser)]
#[command(about = "Evidence-linked code and documentation snapshots", version)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Index {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        dump: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    Links {
        #[arg(long, default_value = ".codanna/knowledge.json")]
        graph: PathBuf,
        entity: String,
    },
    Context {
        #[arg(long, default_value = ".codanna/knowledge.json")]
        graph: PathBuf,
        #[arg(default_value = "")]
        query: String,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long = "file")]
        files: Vec<String>,
        #[arg(long = "entity")]
        entities: Vec<String>,
        #[arg(long, default_value_t = 24000)]
        max_bytes: usize,
        #[arg(long, default_value_t = 40)]
        max_nodes: usize,
        #[arg(long, default_value_t = 2)]
        max_depth: usize,
    },
    Path {
        #[arg(long, default_value = ".codanna/knowledge.json")]
        graph: PathBuf,
        source: String,
        target: String,
        #[arg(long, default_value_t = 6)]
        max_depth: usize,
    },
    Serve {
        #[arg(long, default_value = ".codanna/knowledge.json")]
        graph: PathBuf,
    },
    /// Merge repository snapshots and optionally attach OpenAPI 3.x JSON contracts.
    Workspace {
        #[arg(long = "graph", required = true)]
        graphs: Vec<PathBuf>,
        #[arg(long)]
        openapi_repo: Option<String>,
        #[arg(long)]
        openapi_path: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
}
fn run() -> knowledge::Result<()> {
    let result = match Cli::parse().command {
        Action::Index {
            root,
            repo,
            dump,
            out,
        } => {
            let input = knowledge::io::input(&root, &repo, dump.as_deref())?;
            let graph = knowledge::links::build(&input)?;
            let out = out.unwrap_or_else(|| root.join(".codanna/knowledge.json"));
            knowledge::io::save(&graph, &out)?;
            serde_json::json!({"nodes":graph.nodes.len(),"edges":graph.edges.len(),"unresolved":graph.unresolved.len(),"snapshot":out,"limitations":graph.limitations})
        }
        Action::Links { graph, entity } => {
            let graph = knowledge::io::load(&graph)?;
            let node = graph.resolve(&entity)?;
            serde_json::json!({"node":node,"incoming":graph.edges.iter().filter(|e|e.to==node.id).collect::<Vec<_>>(),"outgoing":graph.edges.iter().filter(|e|e.from==node.id).collect::<Vec<_>>(),"unresolved":graph.unresolved.iter().filter(|r|r.from==node.id).collect::<Vec<_>>(),"limitations":graph.limitations})
        }
        Action::Context {
            graph,
            query,
            repo,
            files,
            entities,
            max_bytes,
            max_nodes,
            max_depth,
        } => {
            let graph = knowledge::io::load(&graph)?;
            serde_json::to_value(context::get(
                &graph,
                &context::Request {
                    query,
                    repo,
                    files,
                    entities,
                    max_bytes,
                    max_nodes,
                    max_depth,
                },
            )?)?
        }
        Action::Path {
            graph,
            source,
            target,
            max_depth,
        } => {
            let graph = knowledge::io::load(&graph)?;
            serde_json::to_value(context::path(&graph, &source, &target, max_depth)?)?
        }
        Action::Serve { graph } => return service::serve(&graph),
        Action::Workspace {
            graphs,
            openapi_repo,
            openapi_path,
            out,
        } => {
            let loaded = graphs
                .iter()
                .map(|p| knowledge::io::load(p))
                .collect::<knowledge::Result<Vec<_>>>()?;
            let mut graph = contracts::merge(loaded)?;
            match (openapi_repo, openapi_path) {
                (Some(repo), Some(path)) => {
                    let text = String::from_utf8(knowledge::io::read_bounded(
                        &path,
                        knowledge::MAX_FILE_BYTES as u64,
                    )?)?;
                    let relative = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .ok_or("invalid OpenAPI filename")?;
                    contracts::add_openapi(&mut graph, &repo, relative, &text)?;
                }
                (None, None) => {}
                _ => return Err("openapi-repo and openapi-path must be supplied together".into()),
            };
            knowledge::io::save(&graph, &out)?;
            serde_json::json!({"repositories":graph.repositories.keys().collect::<Vec<_>>(),"nodes":graph.nodes.len(),"edges":graph.edges.len(),"unresolved":graph.unresolved.len(),"snapshot":out})
        }
    };
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &result)?;
    writeln!(out)?;
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe)
            {
                return std::process::ExitCode::SUCCESS;
            }
            eprintln!("knowledge: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
