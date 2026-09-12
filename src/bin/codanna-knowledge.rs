//! Opt-in companion CLI; never changes Codanna's existing indexes.
#[path = "../knowledge/mod.rs"] mod knowledge;
#[path = "../knowledge/context.rs"] mod context;
#[path = "../knowledge/service.rs"] mod service;
#[path = "../knowledge/contracts.rs"] mod contracts;
#[path = "../knowledge/drift.rs"] mod drift;
use clap::{Parser, Subcommand}; use std::path::PathBuf; use std::io::Write; use std::collections::BTreeMap;
#[derive(Parser)] #[command(about = "Evidence-linked code and documentation snapshots", version)] struct Cli { #[command(subcommand)] command: Action }
#[derive(Subcommand)] enum Action {
Index { #[arg(long, default_value = ".")] root: PathBuf, #[arg(long)] repo: String, #[arg(long)] dump: Option<PathBuf>, #[arg(long)] out: Option<PathBuf> },
Links { #[arg(long, default_value = ".codanna/knowledge.json")] graph: PathBuf, entity: String },
Context { #[arg(long, default_value = ".codanna/knowledge.json")] graph: PathBuf, #[arg(default_value = "")] query: String, #[arg(long)] repo: Option<String>, #[arg(long="file")] files: Vec<String>, #[arg(long="entity")] entities: Vec<String>, #[arg(long, default_value_t=24000)] max_bytes: usize, #[arg(long, default_value_t=40)] max_nodes: usize, #[arg(long, default_value_t=2)] max_depth: usize },
Path { #[arg(long, default_value = ".codanna/knowledge.json")] graph: PathBuf, source: String, target: String, #[arg(long, default_value_t=6)] max_depth: usize },
Serve { #[arg(long, default_value = ".codanna/knowledge.json")] graph: PathBuf },
Workspace { #[arg(long="graph", required=true)] graphs: Vec<PathBuf>, #[arg(long)] openapi_repo: Option<String>, #[arg(long)] openapi_path: Option<PathBuf>, #[arg(long)] out: PathBuf },
/// Compare indexed evidence with current local sources. --root uses repo=path.
Check { #[arg(long, default_value = ".codanna/knowledge.json")] graph: PathBuf, #[arg(long="root", value_parser=parse_root)] roots: Vec<(String,String)>, #[arg(long)] fail_on_review: bool }
}
fn parse_root(value:&str)->Result<(String,String),String>{let (repo,path)=value.split_once('=').ok_or("expected REPO=PATH")?; knowledge::validate_repo(repo).map_err(|e|e.to_string())?; Ok((repo.into(),path.into()))}
fn run() -> knowledge::Result<()> { let cli=Cli::parse(); let mut exit_failure=false; let result = match cli.command {
Action::Index { root, repo, dump, out } => { let input=knowledge::io::input(&root,&repo,dump.as_deref())?; let graph=knowledge::links::build(&input)?; let out=out.unwrap_or_else(||root.join(".codanna/knowledge.json")); knowledge::io::save(&graph,&out)?; serde_json::json!({"nodes":graph.nodes.len(),"edges":graph.edges.len(),"unresolved":graph.unresolved.len(),"snapshot":out,"limitations":graph.limitations}) },
Action::Links { graph, entity } => { let graph=knowledge::io::load(&graph)?; let node=graph.resolve(&entity)?; serde_json::json!({"node":node,"incoming":graph.edges.iter().filter(|e|e.to==node.id).collect::<Vec<_>>(),"outgoing":graph.edges.iter().filter(|e|e.from==node.id).collect::<Vec<_>>(),"unresolved":graph.unresolved.iter().filter(|r|r.from==node.id).collect::<Vec<_>>(),"limitations":graph.limitations}) },
Action::Context { graph, query, repo, files, entities, max_bytes, max_nodes, max_depth } => { let graph=knowledge::io::load(&graph)?; serde_json::to_value(context::get(&graph,&context::Request{query,repo,files,entities,max_bytes,max_nodes,max_depth})?)? },
Action::Path { graph, source, target, max_depth } => { let graph=knowledge::io::load(&graph)?; serde_json::to_value(context::path(&graph,&source,&target,max_depth)?)? },
Action::Serve { graph } => return service::serve(&graph),
Action::Workspace { graphs, openapi_repo, openapi_path, out } => { let loaded=graphs.iter().map(|p|knowledge::io::load(p)).collect::<knowledge::Result<Vec<_>>>()?; let mut graph=contracts::merge(loaded)?; match (openapi_repo,openapi_path) { (Some(repo),Some(path)) => { let text=String::from_utf8(knowledge::io::read_bounded(&path,knowledge::MAX_FILE_BYTES as u64)?)?; let relative=path.file_name().and_then(|s|s.to_str()).ok_or("invalid OpenAPI filename")?; contracts::add_openapi(&mut graph,&repo,relative,&text)?; }, (None,None)=>{}, _=>return Err("openapi-repo and openapi-path must be supplied together".into()) }; knowledge::io::save(&graph,&out)?; serde_json::json!({"repositories":graph.repositories.keys().collect::<Vec<_>>(),"nodes":graph.nodes.len(),"edges":graph.edges.len(),"unresolved":graph.unresolved.len(),"snapshot":out}) },
Action::Check { graph, roots, fail_on_review } => { let graph=knowledge::io::load(&graph)?; let roots:BTreeMap<_,_>=roots.into_iter().collect(); let report=drift::check(&graph,&roots)?; exit_failure=report.errors>0 || (fail_on_review && report.reviews>0); serde_json::to_value(report)? }
}; let mut out=std::io::stdout().lock(); serde_json::to_writer(&mut out,&result)?; writeln!(out)?; if exit_failure { return Err("knowledge drift policy failed".into()); } Ok(()) }
fn main()->std::process::ExitCode{match run(){Ok(())=>std::process::ExitCode::SUCCESS,Err(error)=>{if error.downcast_ref::<std::io::Error>().is_some_and(|e|e.kind()==std::io::ErrorKind::BrokenPipe){return std::process::ExitCode::SUCCESS;} eprintln!("knowledge: {error}"); std::process::ExitCode::FAILURE}}}
