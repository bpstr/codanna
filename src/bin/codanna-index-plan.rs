//! Read-only source and embedding-cache inventory, isolated from indexing startup.
use clap::Parser;
use codanna::{Settings, rebuild_plan};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(about = "Plan code rebuild inputs offline; never initializes an index or provider", version)]
struct Cli {
    /// Existing settings file. No discovery, registration, or auto-initialization.
    #[arg(long, value_name = "FILE")]
    config: PathBuf,
    /// Source roots within this workspace; omitted paths use configured roots.
    paths: Vec<PathBuf>,
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = cli.config.canonicalize()?;
    if !config.is_file() {
        return Err("--config must name an existing settings file".into());
    }
    let mut settings = Settings::load_from(&config)?;
    settings.index_path = codanna::init::resolve_index_path(&settings, Some(&config));
    let report = rebuild_plan::inspect(settings, &cli.paths)?;
    let mut output = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut output, &report)?;
    writeln!(output)?;
    // Partial coverage and rejected inputs must not look like a successful full
    // cost assessment to shell scripts. The JSON report is still available.
    Ok(report.status == "complete")
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(3),
        Err(error) => {
            eprintln!("index-plan: {error}");
            ExitCode::FAILURE
        }
    }
}
