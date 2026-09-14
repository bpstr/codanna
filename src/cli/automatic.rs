//! Local workspace discovery and setup. Query routing never inventories descendants.

use crate::cli::{Cli, Commands};
use crate::init::workspaces::{Workspace, WorkspaceRegistry, confined_index_path, read_settings};
use crate::{IndexError, Settings};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const CONFIG: &str = "settings.toml";
const MAX_DIRECTORIES: usize = 4096;
const MAX_DEPTH: usize = 6;
const MAX_REPOSITORIES: usize = 256;
const MAX_ENTRIES: usize = 50_000;

#[derive(Debug, Serialize)]
pub struct RepositoryLocation {
    pub path: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceDiscovery {
    pub root: PathBuf,
    pub reason: &'static str,
    pub configured: bool,
    pub repositories: Vec<RepositoryLocation>,
    pub inventory_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupMode {
    Index,
    Existing,
    /// Authorized local MCP first use; bootstrap remains code-only.
    Bootstrap,
}

pub fn startup_mode(cli: &Cli) -> Option<StartupMode> {
    if cli.config.is_some() || cli.workspace_selector.is_some() {
        return None;
    }
    match &cli.command {
        Commands::Index { paths, dry_run: false, .. } if paths.is_empty() => Some(StartupMode::Index),
        Commands::Serve { http: false, https: false, .. }
        | Commands::Mcp { .. } | Commands::Retrieve { .. } | Commands::Dump { .. } => Some(StartupMode::Existing),
        _ => None,
    }
}

pub fn try_run(cli: &Cli) -> Result<Option<i32>, IndexError> {
    let Some(mode) = startup_mode(cli) else { return Ok(None); };
    if !auto_setup_enabled(std::env::var("CODANNA_AUTO_SETUP").ok().as_deref())? {
        return Ok(None);
    }
    let start = std::env::current_dir().map_err(|error| failure(error.to_string()))?;
    let home = dirs::home_dir();
    let registry = WorkspaceRegistry::default();
    let workspace = prepare(&registry, &start, home.as_deref(), mode)?;
    let settings = read_settings(&workspace.root)?;
    if matches!(cli.command, Commands::Serve { .. }) && settings.server.mode == "http" {
        return Err(failure("Automatic selection is local-only. Start this network server with --config or --workspace explicitly."));
    }
    let mut arguments = vec![OsString::from("--workspace"), workspace.id.as_str().into()];
    arguments.extend(std::env::args_os().skip(1));
    if mode == StartupMode::Index && settings.indexing.indexed_paths.is_empty() {
        if !is_uninitialized_index(&confined_index_path(&workspace.root, &settings)?)? {
            return Err(failure("The source list is empty but an index already exists. Set the intended roots explicitly; automatic setup will not broaden an existing index."));
        }
        arguments.push(OsString::from("."));
    }
    crate::cli::workspace::launch(workspace.id.as_str(), &arguments).map(Some)
}

fn auto_setup_enabled(value: Option<&str>) -> Result<bool, IndexError> {
    match value {
        None | Some("1" | "true" | "on") => Ok(true),
        Some("0" | "false" | "off") => Ok(false),
        _ => Err(failure("CODANNA_AUTO_SETUP must be true/false or 1/0")),
    }
}

pub fn prepare(registry: &WorkspaceRegistry, start: &Path, home: Option<&Path>, mode: StartupMode) -> Result<Workspace, IndexError> {
    let (root, _) = resolve_root(start, home)?;
    prepare_root(registry, &root, mode)
}

/// The caller has already selected an authorized local session root. Do not
/// rediscover an ancestor here: a broad parent index is not session ownership.
pub(crate) fn prepare_root(registry: &WorkspaceRegistry, root: &Path, mode: StartupMode) -> Result<Workspace, IndexError> {
    if !config_path(root).try_exists().map_err(|e| read_error(root, e))? {
        if mode == StartupMode::Existing {
            return Err(failure(format!("Workspace {} is not indexed yet. Run 'codanna index' or use 'codanna workspace serve' for automatic first use.", root.display())));
        }
        registry.list()?;
        create_configuration(root, mode == StartupMode::Bootstrap)?;
    }
    let settings = read_settings(root)?;
    let index = confined_index_path(root, &settings)?;
    if mode == StartupMode::Existing {
        if !index.join("tantivy/meta.json").is_file() {
            return Err(failure(format!("Workspace {} has no readable index. Run 'codanna index' once in its root.", root.display())));
        }
        crate::storage::IndexMetadata::load(&index)?;
    }
    ensure_registration(registry, root)
}

/// Explicit diagnostic inventory, not the query path.
pub fn inspect(start: &Path, home: Option<&Path>) -> Result<WorkspaceDiscovery, IndexError> {
    let (root, reason) = resolve_root(start, home)?;
    let (repositories, inventory_truncated) = inventory(&root)?;
    Ok(WorkspaceDiscovery { configured: config_path(&root).is_file(), root, reason, repositories, inventory_truncated })
}

pub(crate) fn config_path(root: &Path) -> PathBuf {
    root.join(crate::init::local_dir_name()).join(CONFIG)
}

fn start_directory(start: &Path, home: Option<&Path>) -> Result<PathBuf, IndexError> {
    let start = fs::canonicalize(start).map_err(|e| read_error(start, e))?;
    let start = if start.is_dir() { start } else {
        start.parent().ok_or_else(|| failure("Source path has no parent"))?.to_path_buf()
    };
    let home = home.and_then(|path| path.canonicalize().ok());
    if start.parent().is_none() || home.as_ref() == Some(&start)
        || ["/usr", "/usr/local", "/etc", "/var", "/tmp", "/opt", "/bin", "/sbin"].iter().any(|path| start == Path::new(path)) {
        return Err(failure("Open a project directory, not HOME or a system directory. Codanna will not guess which unrelated project you mean."));
    }
    Ok(start)
}

/// Nearest applicable boundary wins. No descendant scan and no ancestor
/// `indexed_paths = ["."]` override of an independently opened checkout.
pub(crate) fn resolve_root(start: &Path, home: Option<&Path>) -> Result<(PathBuf, &'static str), IndexError> {
    let start = start_directory(start, home)?;
    let home = home.and_then(|path| path.canonicalize().ok());
    for root in start.ancestors().take(64) {
        if root.parent().is_none() || home.as_deref() == Some(root) { break; }
        match fs::symlink_metadata(config_path(root)) {
            Ok(_) => { read_settings(root)?; return Ok((root.to_path_buf(), "nearest-workspace")); }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(read_error(root, error)),
        }
        if git_marker(root)? { return Ok((root.to_path_buf(), "nearest-git-checkout")); }
        if has_manifest(root) { return Ok((root.to_path_buf(), "nearest-project-manifest")); }
    }
    Ok((start, "opened-directory"))
}

/// A client root is an opened directory, not a request to adopt a containing
/// index. An ancestor Git/manifest boundary may resolve a source subdirectory;
/// an unrelated ancestor's Codanna settings alone may not widen the session.
pub(crate) fn resolve_session_root(start: &Path, home: Option<&Path>) -> Result<PathBuf, IndexError> {
    let start = start_directory(start, home)?;
    if config_path(&start).try_exists().map_err(|e| read_error(&start, e))? {
        read_settings(&start)?;
        return Ok(start);
    }
    let home = home.and_then(|path| path.canonicalize().ok());
    for root in start.ancestors().take(64) {
        if root.parent().is_none() || home.as_deref() == Some(root) { break; }
        if git_marker(root)? || has_manifest(root) {
            if config_path(root).try_exists().map_err(|e| read_error(root, e))? { read_settings(root)?; }
            return Ok(root.to_path_buf());
        }
    }
    Ok(start)
}

fn has_manifest(root: &Path) -> bool {
    ["Cargo.toml", "go.mod", "package.json", "composer.json", "pyproject.toml", "pom.xml", "build.gradle", "build.gradle.kts", "Package.swift"]
        .iter().any(|name| root.join(name).is_file())
}

fn git_marker(root: &Path) -> Result<bool, IndexError> {
    match fs::symlink_metadata(root.join(".git")) {
        Ok(metadata) => Ok(metadata.is_dir() || metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(read_error(root, error)),
    }
}

fn inventory(root: &Path) -> Result<(Vec<RepositoryLocation>, bool), IndexError> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut visited = 0;
    let mut entries_seen = 0;
    let mut truncated = false;
    while let Some((directory, depth)) = pending.pop() {
        if visited >= MAX_DIRECTORIES || found.len() >= MAX_REPOSITORIES { truncated = true; break; }
        visited += 1;
        if git_marker(&directory)? {
            let relative = directory.strip_prefix(root).unwrap_or(&directory);
            found.push(RepositoryLocation { path: if relative.as_os_str().is_empty() { PathBuf::from(".") } else { relative.to_path_buf() } });
            if directory != root { continue; }
        }
        let mut directories = Vec::new();
        for entry in fs::read_dir(&directory).map_err(|e| read_error(&directory, e))? {
            entries_seen += 1;
            if entries_seen > MAX_ENTRIES { return Ok((found, true)); }
            let entry = entry.map_err(|e| read_error(&directory, e))?;
            if !entry.file_type().map_err(|e| read_error(&entry.path(), e))?.is_dir() || ignored_directory(&entry.file_name()) { continue; }
            if depth >= MAX_DEPTH { truncated = true; continue; }
            directories.push(entry.path());
            if directories.len() + pending.len() >= MAX_DIRECTORIES { truncated = true; break; }
        }
        directories.sort();
        pending.extend(directories.into_iter().rev().map(|path| (path, depth + 1)));
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((found, truncated))
}

fn ignored_directory(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| name.starts_with('.') || matches!(name, "node_modules" | "vendor" | "target" | "build" | "dist" | "coverage" | "venv" | "__pycache__"))
}

fn create_configuration(root: &Path, code_only: bool) -> Result<(), IndexError> {
    let state = root.join(crate::init::local_dir_name());
    match fs::symlink_metadata(&state) {
        Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => return Err(failure("Local .codanna state must be a real directory, not a symlink")),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&state).or_else(|error| if error.kind() == std::io::ErrorKind::AlreadyExists { Ok(()) } else { Err(error) }).map_err(|e| read_error(&state, e))?;
        }
        Err(error) => return Err(read_error(&state, error)),
    }
    if state.canonicalize().map_err(|e| read_error(&state, e))? != state { return Err(failure("Local .codanna state escaped the workspace")); }
    create_ignore_file(root)?;
    let mut settings = Settings { index_path: PathBuf::from(crate::init::local_dir_name()).join("index"), workspace_root: None, ..Settings::default() };
    settings.indexing.indexed_paths = vec![PathBuf::from(".")];
    if code_only { settings.semantic_search.enabled = false; settings.indexing.parallelism = 2; }
    let text = toml::to_string_pretty(&settings).map_err(|e| failure(e.to_string()))?;
    let mut file = tempfile::NamedTempFile::new_in(&state).map_err(|e| read_error(&state, e))?;
    file.write_all(text.as_bytes()).map_err(|e| read_error(&state, e))?;
    file.as_file().sync_all().map_err(|e| read_error(&state, e))?;
    match file.persist_noclobber(config_path(root)) {
        Ok(_) => {}
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(read_error(&state, error.error)),
    }
    read_settings(root)?;
    Ok(())
}

pub(crate) fn is_uninitialized_index(index: &Path) -> Result<bool, IndexError> {
    let entries = match fs::read_dir(index) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(read_error(index, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| read_error(index, e))?;
        if entry.file_name() != "tantivy" || !entry.file_type().map_err(|e| read_error(&entry.path(), e))?.is_dir() { return Ok(false); }
        if let Some(entry) = fs::read_dir(entry.path()).map_err(|e| read_error(index, e))?.next() { entry.map_err(|e| read_error(index, e))?; return Ok(false); }
    }
    Ok(true)
}

fn create_ignore_file(root: &Path) -> Result<(), IndexError> {
    let content = "# Codanna automatic setup (gitignore syntax)\n.codanna/\n.git/\nnode_modules/\nvendor/\ntarget/\nbuild/\ndist/\ncoverage/\n.venv/\nvenv/\n__pycache__/\n.cargo/\n";
    let mut temporary = tempfile::NamedTempFile::new_in(root).map_err(|e| read_error(root, e))?;
    temporary.write_all(content.as_bytes()).map_err(|e| read_error(root, e))?;
    temporary.as_file().sync_all().map_err(|e| read_error(root, e))?;
    match temporary.persist_noclobber(root.join(".codannaignore")) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(read_error(root, error.error)),
    }
}

fn ensure_registration(registry: &WorkspaceRegistry, root: &Path) -> Result<Workspace, IndexError> {
    let known = registry.list()?;
    if let Some(workspace) = known.iter().find(|w| w.root.canonicalize().ok().as_deref() == Some(root)) { return registry.get(workspace.id.as_str()); }
    let basename = root.file_name().unwrap_or_default().to_string_lossy();
    let mut alias: String = basename.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    alias = alias.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).chars().take(48).collect();
    if alias.is_empty() { alias = "workspace".to_owned(); }
    if known.iter().any(|w| w.name == alias || w.id.as_str() == alias) {
        alias.push('-'); alias.push_str(&hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()))[..12]);
    }
    match registry.add(root, Some(&alias)) {
        Ok(workspace) => Ok(workspace),
        Err(error) => {
            let known = registry.list()?;
            if let Some(workspace) = known.iter().find(|w| w.root == root) { return registry.get(workspace.id.as_str()); }
            if known.iter().any(|w| w.name == alias || w.id.as_str() == alias) {
                let base: String = alias.chars().take(48).collect();
                return registry.add(root, Some(&format!("{base}-{}", &hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()))[..12])));
            }
            Err(error)
        }
    }
}

fn read_error(path: &Path, source: std::io::Error) -> IndexError { IndexError::FileRead { path: path.to_path_buf(), source } }
fn failure(message: impl Into<String>) -> IndexError { IndexError::General(message.into()) }

#[cfg(test)]
mod tests;
