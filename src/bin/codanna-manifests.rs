//! Fast local package-manifest dependency discovery.
#[path = "../knowledge/manifests.rs"]
mod manifests;
#[path = "../knowledge/mod.rs"]
mod knowledge;
use clap::Parser;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Scan package manifests and link local cross-repository dependencies", version)]
struct Cli {
    #[arg(long="root", value_parser=parse_root, required = true)]
    roots: Vec<(String, PathBuf)>,
}

fn parse_root(value: &str) -> Result<(String, PathBuf), String> {
    let (repo, path) = value.split_once('=').ok_or("expected REPO=PATH")?;
    knowledge::validate_repo(repo).map_err(|e| e.to_string())?;
    Ok((repo.into(), PathBuf::from(path)))
}

fn run() -> knowledge::Result<()> {
    let roots: BTreeMap<_, _> = Cli::parse().roots.into_iter().collect();
    let report = manifests::scan(&roots)?;
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &report)?;
    writeln!(out)?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe) {
                return std::process::ExitCode::SUCCESS;
            }
            eprintln!("manifests: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
