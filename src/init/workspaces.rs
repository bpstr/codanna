//! Metadata-only workspace registration over the existing v1 project registry.
//!
//! A workspace may contain several indexed roots. This module deliberately does
//! not open indexes, load embedding models, migrate rows, or start an MCP router.

use super::{ProjectId, ProjectInfo, ProjectRegistry, local_dir_name, projects_file};
use crate::{IndexError, Settings};
use fs4::fs_std::FileExt;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};

/// Reuse existing stable project IDs without rewriting a user's registry.
pub type WorkspaceId = ProjectId;

/// One registered product workspace, not one entry per child repository.
#[derive(Debug, Clone, Serialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub root: PathBuf,
    pub config_path: PathBuf,
}

impl Workspace {
    fn from_entry(id: &str, info: &ProjectInfo) -> Self {
        Self {
            id: ProjectId::from_string(id.to_owned()),
            name: info.name.clone(),
            root: info.path.clone(),
            config_path: info.path.join(local_dir_name()).join("settings.toml"),
        }
    }
}

/// Configuration diagnostics only; `configured` is not an index health claim.
#[derive(Debug, Serialize)]
pub struct WorkspaceDiagnostic {
    pub workspace: Workspace,
    pub status: &'static str,
    pub index_path: Option<PathBuf>,
    pub index_directory_exists: bool,
    pub detail: Option<String>,
}

/// Handle to one registry file. Reads never create directories or locks.
#[derive(Debug, Clone)]
pub struct WorkspaceRegistry {
    path: PathBuf,
}

impl Default for WorkspaceRegistry {
    fn default() -> Self {
        Self::new(projects_file())
    }
}

impl WorkspaceRegistry {
    /// An explicit path makes tests independent of HOME and process-global state.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// List registrations in stable alias/ID order without opening their indexes.
    pub fn list(&self) -> Result<Vec<Workspace>, IndexError> {
        let registry = ProjectRegistry::load_from_path(&self.path)?;
        let mut workspaces: Vec<_> = registry
            .projects
            .iter()
            .map(|(id, info)| Workspace::from_entry(id, info))
            .collect();
        workspaces.sort_by(|a, b| {
            a.name.cmp(&b.name).then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        Ok(workspaces)
    }

    /// Resolve an exact ID or alias. Legacy duplicate aliases are errors, not guesses.
    pub fn get(&self, selector: &str) -> Result<Workspace, IndexError> {
        let registry = ProjectRegistry::load_from_path(&self.path)?;
        let id = resolve_id(&registry, selector)?;
        Ok(Workspace::from_entry(&id, &registry.projects[&id]))
    }

    /// Register the nearest configured workspace; never initialize or index it.
    pub fn add(&self, path: &Path, name: Option<&str>) -> Result<Workspace, IndexError> {
        let root = discover(path)?.ok_or_else(|| {
            failure("No workspace configuration found. Run 'codanna init' in the intended workspace root first.")
        })?;
        let settings = read_settings(&root)?;
        confined_index_path(&root, &settings)?;
        let mut info = ProjectRegistry::create_project_info(&root);
        if let Some(name) = name {
            validate_alias(name)?;
            info.name = name.to_owned();
        }
        with_registry(&self.path, |registry| {
            if let Some(id) = id_for_root(registry, &root)? {
                let existing = &registry.projects[&id];
                if name.is_some_and(|name| name != existing.name) {
                    return Err(failure("Workspace is already registered under a different alias. Use 'codanna workspace rename' explicitly."));
                }
                return Ok(Workspace::from_entry(&id, existing));
            }
            validate_alias(&info.name)?;
            ensure_alias_available(registry, &info.name, None)?;
            let id = fresh_id(registry);
            registry.projects.insert(id.clone(), info);
            Ok(Workspace::from_entry(&id, &registry.projects[&id]))
        })
    }

    /// Rename routing metadata while preserving identity, counters, and storage.
    pub fn rename(&self, selector: &str, name: &str) -> Result<Workspace, IndexError> {
        validate_alias(name)?;
        with_registry(&self.path, |registry| {
            let id = resolve_id(registry, selector)?;
            ensure_alias_available(registry, name, Some(&id))?;
            let info = registry.projects.get_mut(&id).ok_or_else(|| failure("Workspace disappeared"))?;
            info.name = name.to_owned();
            Ok(Workspace::from_entry(&id, info))
        })
    }

    /// Unregister only. No source, configuration, or index files are deleted.
    /// Existing independent serve processes are not managed by this registry.
    pub fn remove(&self, selector: &str) -> Result<Workspace, IndexError> {
        with_registry(&self.path, |registry| {
            let id = resolve_id(registry, selector)?;
            let info = registry.projects.remove(&id).ok_or_else(|| failure("Workspace disappeared"))?;
            if registry.default_project.as_deref() == Some(id.as_str()) {
                registry.default_project = None;
            }
            Ok(Workspace::from_entry(&id, &info))
        })
    }

    /// Rebind a registration after a filesystem move. Never move files implicitly.
    /// Refuse a second live checkout instead of treating a copy as the original.
    pub fn relocate(&self, selector: &str, path: &Path) -> Result<Workspace, IndexError> {
        let root = discover(path)?.ok_or_else(|| failure("New location has no workspace configuration"))?;
        let settings = read_settings(&root)?;
        confined_index_path(&root, &settings)?;
        with_registry(&self.path, |registry| {
            let id = resolve_id(registry, selector)?;
            let old = &registry.projects[&id].path;
            if old != &root && old.try_exists().map_err(|source| IndexError::FileRead {
                path: old.clone(), source,
            })? {
                return Err(failure("Original workspace still exists. A copy or another worktree needs a separate registration."));
            }
            if id_for_root(registry, &root)?.is_some_and(|other| other != id) {
                return Err(failure("New location belongs to another registered workspace"));
            }
            let info = registry.projects.get_mut(&id).ok_or_else(|| failure("Workspace disappeared"))?;
            info.path = root;
            Ok(Workspace::from_entry(&id, info))
        })
    }

    /// Inspect configuration and directory presence, not index completeness.
    pub fn doctor(&self, selector: &str) -> Result<WorkspaceDiagnostic, IndexError> {
        let workspace = self.get(selector)?;
        let inspected = read_settings(&workspace.root)
            .and_then(|settings| confined_index_path(&workspace.root, &settings));
        let (status, index_path, exists, detail) = match inspected {
            Ok(path) => {
                let exists = path.is_dir();
                ("configured", Some(path), exists, None)
            }
            Err(error) => ("unavailable", None, false, Some(error.to_string())),
        };
        Ok(WorkspaceDiagnostic {
            workspace,
            status,
            index_path,
            index_directory_exists: exists,
            detail,
        })
    }
}

/// Discover from a supplied path without changing cwd. A cache-only `.codanna`
/// directory is skipped; a present but invalid nearest config stops discovery.
pub fn discover(start: &Path) -> Result<Option<PathBuf>, IndexError> {
    let canonical = fs::canonicalize(start).map_err(|source| IndexError::FileRead {
        path: start.to_path_buf(), source,
    })?;
    let directory = if canonical.is_dir() {
        canonical.as_path()
    } else {
        canonical.parent().ok_or_else(|| failure("Path has no parent directory"))?
    };
    for root in directory.ancestors() {
        let path = root.join(local_dir_name()).join("settings.toml");
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                read_settings(root)?;
                return Ok(Some(root.to_path_buf()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(IndexError::FileRead { path, source }),
        }
    }
    Ok(None)
}

/// Read and validate the exact workspace config, without environment overlays.
/// The launch boundary removes CI_ overrides so worker selection agrees with this.
pub fn read_settings(root: &Path) -> Result<Settings, IndexError> {
    let root = fs::canonicalize(root).map_err(|source| IndexError::FileRead {
        path: root.to_path_buf(), source,
    })?;
    let config = root.join(local_dir_name()).join("settings.toml");
    let real_config = fs::canonicalize(&config).map_err(|source| IndexError::FileRead {
        path: config.clone(), source,
    })?;
    if real_config != config {
        return Err(failure("Workspace configuration must be local, not a symlink to another configuration"));
    }
    let text = fs::read_to_string(&config).map_err(|source| IndexError::FileRead {
        path: config.clone(), source,
    })?;
    let settings: Settings = toml::from_str(&text)
        .map_err(|error| failure(format!("Invalid workspace configuration {}: {error}", config.display())))?;
    if let Some(configured) = &settings.workspace_root {
        let configured = fs::canonicalize(root.join(configured)).map_err(|source| IndexError::FileRead {
            path: root.join(configured), source,
        })?;
        if configured != root {
            return Err(failure("Configuration workspace_root points to another workspace. Correct it before selecting this workspace."));
        }
    }
    Ok(settings)
}

/// Resolve a possibly not-yet-created index through its nearest existing ancestor.
/// This first selection release supports indexes beneath local `.codanna` only.
/// A symlink or `..` must not redirect a selected workspace into another index.
pub fn confined_index_path(root: &Path, settings: &Settings) -> Result<PathBuf, IndexError> {
    if settings.index_path.components().any(|c| c == Component::ParentDir) {
        return Err(failure("Workspace index_path cannot contain parent traversal"));
    }
    let state = root.join(local_dir_name());
    let mut existing = root.join(&settings.index_path);
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = existing.file_name().ok_or_else(|| failure("Invalid index path"))?.to_owned();
                suffix.push(component);
                existing = existing.parent().ok_or_else(|| failure("Invalid index path"))?.to_path_buf();
            }
            Err(source) => return Err(IndexError::FileRead { path: existing, source }),
        }
    }
    let mut resolved = fs::canonicalize(&existing).map_err(|source| IndexError::FileRead {
        path: existing, source,
    })?;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    if resolved == state || !resolved.starts_with(&state) {
        return Err(failure("Selected workspaces require a project-local index beneath .codanna. External/shared index paths are not supported by --workspace yet."));
    }
    if resolved.exists() && !resolved.is_dir() {
        return Err(failure("Workspace index_path is not a directory"));
    }
    Ok(resolved)
}

/// The lock file is stable and never deleted. Locking the replaced JSON inode
/// would not serialize subsequent writers opening the new inode.
pub(super) fn registry_lock(path: &Path) -> Result<File, IndexError> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|source| IndexError::FileWrite {
        path: parent.to_path_buf(), source,
    })?;
    let lock_path = path.with_extension("lock");
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(&lock_path).map_err(|source| IndexError::FileWrite {
            path: lock_path.clone(), source,
        })?;
    FileExt::lock_exclusive(&lock).map_err(|source| IndexError::FileWrite {
        path: lock_path, source,
    })?;
    Ok(lock)
}

pub(super) fn with_registry<T>(
    path: &Path,
    operation: impl FnOnce(&mut ProjectRegistry) -> Result<T, IndexError>,
) -> Result<T, IndexError> {
    let _lock = registry_lock(path)?;
    let mut registry = ProjectRegistry::load_from_path(path)?;
    let value = operation(&mut registry)?;
    registry.save_to_path(path)?;
    Ok(value)
}

pub(super) fn id_for_root(registry: &ProjectRegistry, root: &Path) -> Result<Option<String>, IndexError> {
    let ids: Vec<_> = registry.projects.iter().filter_map(|(id, info)| {
        let path = info.path.canonicalize().unwrap_or_else(|_| info.path.clone());
        (path == root).then_some(id.clone())
    }).collect();
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(id.clone())),
        _ => Err(failure("Multiple registry entries refer to this workspace root. Remove duplicates by ID before continuing.")),
    }
}

pub(super) fn fresh_id(registry: &ProjectRegistry) -> String {
    loop {
        let id = ProjectId::new().to_string();
        if !registry.projects.contains_key(&id)
            && !registry.projects.values().any(|info| info.name == id)
        {
            return id;
        }
    }
}

fn resolve_id(registry: &ProjectRegistry, selector: &str) -> Result<String, IndexError> {
    let mut matches = registry.projects.iter()
        .filter(|(id, info)| id.as_str() == selector || info.name == selector);
    let (id, _) = matches.next().ok_or_else(|| {
        failure(format!("Workspace '{selector}' is not registered. Use 'codanna workspace list'."))
    })?;
    if matches.next().is_some() {
        return Err(failure(format!("Workspace alias '{selector}' is ambiguous. Select an exact ID from 'codanna workspace list'.")));
    }
    Ok(id.clone())
}

fn validate_alias(name: &str) -> Result<(), IndexError> {
    if name.is_empty() || name.len() > 64 || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(failure("Workspace aliases must be 1-64 ASCII letters, digits, '.', '_' or '-', starting with a letter or digit. Supply --name explicitly."));
    }
    Ok(())
}

fn ensure_alias_available(registry: &ProjectRegistry, name: &str, own_id: Option<&str>) -> Result<(), IndexError> {
    if registry.projects.iter().any(|(id, info)| {
        Some(id.as_str()) != own_id && (info.name == name || id == name)
    }) {
        return Err(failure(format!("Workspace alias '{name}' is already in use. Choose a distinct --name.")));
    }
    Ok(())
}

fn failure(message: impl Into<String>) -> IndexError {
    IndexError::General(message.into())
}

#[cfg(test)]
mod tests;
