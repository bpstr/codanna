//! Reverse dependencies from persisted import evidence, including unresolved
//! imports and import-only barrels. Matching is deliberately conservative: it
//! schedules a source reparse, never manufactures a symbol relationship.

use super::PipelineResult;
use crate::{FileId, Settings, storage::DocumentIndex};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};

pub(super) fn absolute(path: &Path, settings: &Settings) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(root) = &settings.workspace_root {
        root.join(path)
    } else {
        path.to_path_buf()
    }
}

fn lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn without_extension(path: &str, settings: &Settings) -> String {
    if let Some((base, extension)) = path.rsplit_once('.') {
        if settings.languages.values().any(|language| {
            language
                .extensions
                .iter()
                .any(|candidate| candidate == extension)
        }) {
            return base.to_owned();
        }
    }
    path.to_owned()
}

fn module_text(path: &str, settings: &Settings) -> String {
    without_extension(path, settings)
        .replace("::", "/")
        .replace(['\\', '.'], "/")
        .trim_matches('/')
        .to_owned()
}

fn module_keys(path: &Path, settings: &Settings) -> BTreeSet<String> {
    let absolute_path = absolute(path, settings);
    let mut keys = BTreeSet::new();
    keys.insert(format!(
        "path:{}",
        lexical(&absolute_path.with_extension("")).display()
    ));
    let mut roots: Vec<&Path> = settings
        .indexed_paths_cache
        .iter()
        .map(PathBuf::as_path)
        .collect();
    if let Some(root) = settings.workspace_root.as_deref() {
        roots.push(root);
    }
    for root in roots {
        let Ok(relative) = absolute_path.strip_prefix(root) else {
            continue;
        };
        let key = module_text(&relative.to_string_lossy(), settings);
        if !key.is_empty() {
            keys.insert(format!("module:{key}"));
        }
        // Package entry files define the directory's public namespace.
        if matches!(
            relative.file_stem().and_then(|s| s.to_str()),
            Some("__init__" | "index" | "mod")
        ) || relative
            .extension()
            .is_some_and(|extension| extension == "go")
        {
            if let Some(parent) = relative.parent() {
                let parent = module_text(&parent.to_string_lossy(), settings);
                if !parent.is_empty() {
                    keys.insert(format!("module:{parent}"));
                }
            }
        }
    }
    // Relative imports of a directory refer to index/__init__/mod files.
    if matches!(
        path.file_stem().and_then(|s| s.to_str()),
        Some("__init__" | "index" | "mod")
    ) {
        if let Some(parent) = absolute_path.parent() {
            keys.insert(format!("path:{}", lexical(parent).display()));
        }
    }
    keys
}

/// Return the transitive importer closure in stable path order. This performs
/// one import-metadata scan and at most one file-path lookup per importing file;
/// only affected source files are subsequently read and parsed.
pub(super) fn import_dependents(
    index: &DocumentIndex,
    settings: &Settings,
    changed: &HashSet<PathBuf>,
    processed: &HashSet<PathBuf>,
) -> PipelineResult<Vec<PathBuf>> {
    if changed.is_empty() {
        return Ok(Vec::new());
    }
    let mut paths: HashMap<FileId, PathBuf> = HashMap::new();
    let mut reverse: HashMap<String, HashSet<PathBuf>> = HashMap::new();
    for import in index.get_all_imports()? {
        let path = match paths.entry(import.file_id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let Some(path) = index.get_file_path(import.file_id)? else {
                    continue;
                };
                entry.insert(PathBuf::from(path))
            }
        };
        if import.path.starts_with("./") || import.path.starts_with("../") {
            let absolute_path = absolute(path, settings);
            if let Some(parent) = absolute_path.parent() {
                let target = without_extension(&import.path, settings);
                reverse
                    .entry(format!("path:{}", lexical(&parent.join(target)).display()))
                    .or_default()
                    .insert(path.clone());
            }
        } else {
            let module = module_text(&import.path, settings);
            let parts: Vec<&str> = module.split('/').filter(|part| !part.is_empty()).collect();
            // Imports may name a module, a symbol in that module, or a package
            // with a provider prefix. Prefix/suffix matches can over-invalidate
            // a namesake; the actual resolver still enforces source identity.
            for start in 0..parts.len() {
                for end in start + 1..=parts.len() {
                    reverse
                        .entry(format!("module:{}", parts[start..end].join("/")))
                        .or_default()
                        .insert(path.clone());
                }
            }
        }
    }
    let mut visited = changed.clone();
    let mut queue: VecDeque<PathBuf> = changed.iter().cloned().collect();
    let mut affected = BTreeSet::new();
    while let Some(path) = queue.pop_front() {
        for key in module_keys(&path, settings) {
            for importer in reverse.get(&key).into_iter().flatten() {
                if !visited.insert(importer.clone()) {
                    continue;
                }
                queue.push_back(importer.clone());
                if !processed.contains(importer)
                    && std::fs::symlink_metadata(absolute(importer, settings))
                        .is_ok_and(|metadata| metadata.is_file())
                {
                    affected.insert(importer.clone());
                }
            }
        }
    }
    Ok(affected.into_iter().collect())
}

/// Durably invalidate outputs before reopening affected source. If a bounded
/// run cannot reopen it, or a later parse fails, the pending record survives
/// and queries cannot keep serving the old dependency-derived relationships.
pub(super) fn invalidate_importers(
    index: &DocumentIndex,
    paths: &[PathBuf],
) -> PipelineResult<HashSet<crate::SymbolId>> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let mut symbols = HashSet::new();
    for path in paths {
        if let Some((file_id, _, _)) = index.get_file_info(&path.to_string_lossy())? {
            symbols.extend(
                index
                    .find_symbols_by_file(file_id)?
                    .into_iter()
                    .map(|symbol| symbol.id),
            );
        }
    }
    index.start_batch()?;
    let staged = (|| -> PipelineResult<()> {
        for path in paths {
            index.store_pending_resolution(path)?;
        }
        for symbol in &symbols {
            index.delete_outgoing_relationships(*symbol)?;
        }
        Ok(())
    })();
    if let Err(error) = staged {
        let _ = index.rollback_batch();
        return Err(error);
    }
    index.commit_batch()?;
    Ok(symbols)
}

pub(super) fn clear_pending_paths(
    index: &DocumentIndex,
    paths: &HashSet<PathBuf>,
) -> PipelineResult<()> {
    if paths.is_empty() {
        return Ok(());
    }
    index.start_batch()?;
    for path in paths {
        if let Err(error) = index.clear_pending_resolution(path) {
            let _ = index.rollback_batch();
            return Err(error.into());
        }
    }
    index.commit_batch()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_entry_and_regular_module_have_distinct_dependency_keys() {
        let mut settings = Settings {
            workspace_root: Some(PathBuf::from("/repo")),
            ..Settings::default()
        };
        settings.indexed_paths_cache = vec![PathBuf::from("/repo")];
        assert!(module_keys(Path::new("pkg/__init__.py"), &settings).contains("module:pkg"));
        assert!(module_keys(Path::new("pkg/helper.py"), &settings).contains("module:pkg/helper"));
        assert!(!module_keys(Path::new("pkg/helper.py"), &settings).contains("module:pkg"));
    }
}
