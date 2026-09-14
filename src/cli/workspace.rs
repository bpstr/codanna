//! Workspace administration and isolated launch adapters for the existing CLI.
//!
//! Selection runs before providers, models, configuration fallback, or indexes.
//! Cross-workspace MCP access is explicit through `workspace serve`.

use crate::IndexError;
use crate::init::workspaces::{Workspace, WorkspaceRegistry, confined_index_path, read_settings};
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "automatic/mcp.rs"]
pub mod mcp;

fn resolve_root(start: &Path, home: Option<&Path>) -> Result<(PathBuf, &'static str), IndexError> {
    let discovery = crate::cli::automatic::inspect(start, home)?;
    Ok((discovery.root, discovery.reason))
}

fn config_path(root: &Path) -> PathBuf {
    root.join(crate::init::local_dir_name()).join("settings.toml")
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceAction {
    /// Serve locally registered workspaces through one read-only stdio MCP connection
    Serve,
    /// Explain automatic root selection and list observed repositories; no writes
    Discover {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Register an initialized workspace without indexing or rewriting its config
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List registrations without loading indexes or embedding models
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show routing metadata for an exact workspace ID or alias
    Show {
        workspace: String,
        #[arg(long)]
        json: bool,
    },
    /// Unregister only; does not delete files or stop independent serve processes
    Remove {
        workspace: String,
        #[arg(long)]
        json: bool,
    },
    /// Change an alias without changing workspace identity
    Rename {
        workspace: String,
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Rebind after moving files yourself; refuses a second live copy
    Move {
        workspace: String,
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Check configuration and index directory presence, not index completeness
    Doctor {
        workspace: String,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(action: &WorkspaceAction) -> Result<i32, IndexError> {
    if matches!(action, WorkspaceAction::Serve) {
        // Do not hold the synchronous stdout lock while serving MCP. This mode
        // explicitly opts into local registry-wide access; ordinary serve remains bound.
        let cwd = std::env::current_dir().map_err(|error| IndexError::General(error.to_string()))?;
        let home = dirs::home_dir();
        return tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(mcp::run(&cwd, home.as_deref()))
        });
    }
    let registry = WorkspaceRegistry::default();
    let mut out = io::stdout().lock();
    match action {
        WorkspaceAction::Serve => unreachable!("handled before stdout locking"),
        WorkspaceAction::Discover { path, json } => {
            let home = dirs::home_dir();
            let discovery = crate::cli::automatic::inspect(path, home.as_deref())?;
            if *json {
                write_json(&mut out, &discovery)?;
            } else {
                writeln!(
                    out,
                    "Workspace root: {}\nSelection: {}\nConfigured: {}",
                    discovery.root.display(),
                    discovery.reason,
                    discovery.configured
                )
                .map_err(output_error)?;
                for repository in discovery.repositories {
                    writeln!(out, "Repository: {}", repository.path.display())
                        .map_err(output_error)?;
                }
                if discovery.inventory_truncated {
                    writeln!(out, "Repository inventory truncated by discovery budget; this is not an index plan.").map_err(output_error)?;
                }
            }
        }
        WorkspaceAction::List { json } => {
            let workspaces = registry.list()?;
            if *json {
                write_json(&mut out, &serde_json::json!({"workspaces": workspaces}))?;
            } else {
                writeln!(out, "NAME\tID\tROOT").map_err(output_error)?;
                for workspace in workspaces {
                    writeln!(
                        out,
                        "{}\t{}\t{}",
                        workspace.name,
                        workspace.id,
                        workspace.root.display()
                    )
                    .map_err(output_error)?;
                }
            }
        }
        WorkspaceAction::Doctor { workspace, json } => {
            let diagnostic = registry.doctor(workspace)?;
            if *json {
                write_json(&mut out, &diagnostic)?;
            } else {
                write_workspace(&mut out, &diagnostic.workspace, false)?;
                writeln!(out, "Configuration: {}", diagnostic.status).map_err(output_error)?;
                writeln!(
                    out,
                    "Index directory exists: {} (completeness not checked)",
                    diagnostic.index_directory_exists
                )
                .map_err(output_error)?;
                if let Some(detail) = &diagnostic.detail {
                    writeln!(out, "{detail}").map_err(output_error)?;
                }
            }
            return Ok(i32::from(diagnostic.status != "configured"));
        }
        WorkspaceAction::Add { path, name, json } => {
            write_workspace(&mut out, &registry.add(path, name.as_deref())?, *json)?;
        }
        WorkspaceAction::Show { workspace, json } => {
            write_workspace(&mut out, &registry.get(workspace)?, *json)?;
        }
        WorkspaceAction::Remove { workspace, json } => {
            write_workspace(&mut out, &registry.remove(workspace)?, *json)?;
        }
        WorkspaceAction::Rename {
            workspace,
            name,
            json,
        } => {
            write_workspace(&mut out, &registry.rename(workspace, name)?, *json)?;
        }
        WorkspaceAction::Move {
            workspace,
            path,
            json,
        } => {
            write_workspace(&mut out, &registry.relocate(workspace, path)?, *json)?;
        }
    }
    Ok(0)
}

fn write_workspace(
    out: &mut impl Write,
    workspace: &Workspace,
    json: bool,
) -> Result<(), IndexError> {
    if json {
        write_json(out, &serde_json::json!({"workspace": workspace}))
    } else {
        writeln!(
            out,
            "Workspace: {} [{}]\nRoot: {}\nConfiguration: {}",
            workspace.name,
            workspace.id,
            workspace.root.display(),
            workspace.config_path.display()
        )
        .map_err(output_error)
    }
}

fn write_json(out: &mut impl Write, value: &impl Serialize) -> Result<(), IndexError> {
    serde_json::to_writer_pretty(&mut *out, value)
        .map_err(|error| IndexError::General(format!("Cannot encode workspace output: {error}")))?;
    writeln!(out).map_err(output_error)
}

fn output_error(error: io::Error) -> IndexError {
    IndexError::General(format!("Cannot write workspace output: {error}"))
}

/// An immutable launch plan is inspectable without spawning a worker in tests.
#[derive(Debug)]
pub struct WorkspaceLaunch {
    pub workspace: Workspace,
    pub arguments: Vec<OsString>,
}

impl WorkspaceLaunch {
    pub fn prepare(
        registry: &WorkspaceRegistry,
        selector: &str,
        arguments: &[OsString],
    ) -> Result<Self, IndexError> {
        let workspace = registry.get(selector)?;
        let settings = read_settings(&workspace.root)?;
        confined_index_path(&workspace.root, &settings)?;
        // Do not allow a copied configuration to pull in unrelated code roots.
        // Explicit external repository membership is deferred to the ownership slice.
        for source in &settings.indexing.indexed_paths {
            let path = workspace.root.join(source);
            let canonical = path.canonicalize().map_err(|source| IndexError::FileRead {
                path: path.clone(),
                source,
            })?;
            if !canonical.starts_with(&workspace.root) {
                return Err(IndexError::General(format!(
                    "Indexed path {} is outside selected workspace {}. External membership is not supported by --workspace yet.",
                    path.display(),
                    workspace.name
                )));
            }
        }
        let arguments = without_selector(arguments)?;
        validate_source_arguments(&workspace, &settings, &arguments)?;
        Ok(Self {
            workspace,
            arguments,
        })
    }

    /// Configure only the child. Never mutate the parent's cwd or environment.
    pub fn command(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        command
            .current_dir(&self.workspace.root)
            .arg("--config")
            .arg(&self.workspace.config_path)
            .args(&self.arguments);
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("CI_")
            {
                command.env_remove(key);
            }
        }
        // No workspace-specific recall binding exists yet. Disabling inherited
        // recall is safer than combining one workspace's code with another's memory.
        command
            .env_remove("CODANNA_RECALL_WORKSPACE")
            .env_remove("CODANNA_RECALL_INDEX");
        command
    }
}

fn validate_source_arguments(
    workspace: &Workspace,
    settings: &crate::Settings,
    arguments: &[OsString],
) -> Result<(), IndexError> {
    use crate::cli::{Cli, Commands, DocumentAction};
    let parsed = Cli::try_parse_from(
        std::iter::once(OsString::from("codanna")).chain(arguments.iter().cloned()),
    )
    .map_err(|error| IndexError::General(error.to_string()))?;
    let source = match &parsed.command {
        Commands::Index { paths, .. } if !paths.is_empty() => {
            // The sole exception is an initial full-root index for a fresh
            // empty configuration. This is not a partial rebuild of old data.
            let initial_root = settings.indexing.indexed_paths.is_empty()
                && paths.len() == 1
                && workspace
                    .root
                    .join(&paths[0])
                    .canonicalize()
                    .ok()
                    .as_deref()
                    == Some(workspace.root.as_path())
                && crate::cli::automatic::is_uninitialized_index(&confined_index_path(
                    &workspace.root,
                    settings,
                )?)?;
            if !initial_root {
                return Err(IndexError::General(
                    "With --workspace, configure roots using add-dir and run index without paths. Partial workspace rebuilds are not supported yet.".to_owned(),
                ));
            }
            None
        }
        Commands::AddDir { path } | Commands::RemoveDir { path } => Some(path),
        Commands::Documents {
            action: DocumentAction::AddCollection { path, .. },
        } => Some(path),
        _ => None,
    };
    if let Some(source) = source {
        let path = workspace.root.join(source);
        let canonical = path.canonicalize().map_err(|source| IndexError::FileRead {
            path: path.clone(),
            source,
        })?;
        if !canonical.starts_with(&workspace.root) {
            return Err(IndexError::General("Source must be inside the selected workspace; external membership is not implemented yet.".to_owned()));
        }
    }
    Ok(())
}

pub fn launch(selector: &str, arguments: &[OsString]) -> Result<i32, IndexError> {
    let plan = WorkspaceLaunch::prepare(&WorkspaceRegistry::default(), selector, arguments)?;
    let executable = std::env::current_exe().map_err(|error| {
        IndexError::General(format!("Cannot locate Codanna executable: {error}"))
    })?;
    let mut command = plan.command(&executable);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Preserve the PID, signals, stdio pipes, and exit status for MCP clients.
        let error = command.exec();
        Err(IndexError::General(format!(
            "Cannot launch workspace {}: {error}",
            plan.workspace.name
        )))
    }
    #[cfg(not(unix))]
    {
        let status = command.status().map_err(|error| {
            IndexError::General(format!(
                "Cannot launch workspace {}: {error}",
                plan.workspace.name
            ))
        })?;
        Ok(status.code().unwrap_or(1))
    }
}

/// Strip only the parsed long selector; preserve all arguments after `--` and
/// preserve non-UTF-8 paths. No shell interpolation or lossy reconstruction.
fn without_selector(arguments: &[OsString]) -> Result<Vec<OsString>, IndexError> {
    let mut forwarded = Vec::with_capacity(arguments.len());
    let mut iter = arguments.iter();
    let mut removed = false;
    while let Some(argument) = iter.next() {
        if argument == OsStr::new("--") {
            forwarded.push(argument.clone());
            forwarded.extend(iter.cloned());
            break;
        }
        if argument == OsStr::new("--workspace") {
            if iter.next().is_none() {
                return Err(IndexError::General(
                    "--workspace requires a selector".to_owned(),
                ));
            }
            removed = true;
        } else if argument
            .to_str()
            .is_some_and(|a| a.starts_with("--workspace="))
        {
            removed = true;
        } else {
            forwarded.push(argument.clone());
        }
    }
    if !removed {
        return Err(IndexError::General(
            "Workspace selector was not present in the parsed command".to_owned(),
        ));
    }
    Ok(forwarded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(|value| OsString::from(*value)).collect()
    }

    #[test]
    fn workspace_selector_preserves_positionals_and_terminator() {
        assert_eq!(
            without_selector(&args(&["--workspace", "assign", "index", "src"])).unwrap(),
            args(&["index", "src"])
        );
        assert_eq!(
            without_selector(&args(&[
                "index",
                "--workspace=assign",
                "--",
                "--workspace",
                "literal"
            ]))
            .unwrap(),
            args(&["index", "--", "--workspace", "literal"])
        );
        assert!(without_selector(&args(&["index"])).is_err());
        assert!(without_selector(&args(&["--workspace"])).is_err());
    }
}
