//! Local document storage and provenance must fit the same opened workspace as
//! code. Validate before loading a model; query snapshots also enforce each hit.
use crate::Settings;
use crate::indexing::facade::IndexFacade;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

type Documents = Option<Arc<RwLock<crate::documents::DocumentStore>>>;

#[derive(Deserialize)]
struct SourceState {
    file_states: HashMap<PathBuf, serde::de::IgnoredAny>,
}

fn prepare(settings: &Settings) -> Result<Option<PathBuf>, String> {
    if !settings.documents.enabled {
        return Ok(None);
    }
    let root = settings
        .workspace_root
        .as_deref()
        .ok_or("Missing document workspace scope")?;
    let base = settings.index_path.join("documents");
    match std::fs::symlink_metadata(&base) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(_) => {}
    }
    let base = IndexFacade::contained_source(root, &base).map_err(|error| error.to_string())?;
    if !base.is_dir() {
        return Err("Document index must be a directory".into());
    }
    // Refuse partial indexes rather than allowing the legacy create-or-open
    // constructor to silently manufacture replacements during a query.
    for relative in ["tantivy/meta.json", "state.json"] {
        let path = IndexFacade::contained_source(root, &base.join(relative))
            .map_err(|error| error.to_string())?;
        if !path.is_file() {
            return Err(
                "Document index is incomplete; index the configured collection before querying"
                    .into(),
            );
        }
    }
    for relative in ["vectors", "clusters.json"] {
        IndexFacade::contained_source(root, &base.join(relative))
            .map_err(|error| error.to_string())?;
    }
    for collection in settings.documents.collections.values() {
        for path in &collection.paths {
            IndexFacade::contained_source(root, path).map_err(|error| error.to_string())?;
        }
    }
    const MAX_STATE: u64 = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(base.join("state.json"))
        .map_err(|error| error.to_string())?
        .take(MAX_STATE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_STATE {
        return Err("Document state exceeds the local workspace metadata budget".into());
    }
    let state: SourceState = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    for path in state.file_states.keys() {
        if path.as_os_str().is_empty() {
            return Err("Document state has no source provenance".into());
        }
        IndexFacade::contained_source(root, path).map_err(|error| error.to_string())?;
    }
    Ok(Some(base))
}

pub(super) fn load(settings: &Settings) -> Result<Documents, String> {
    if prepare(settings)?.is_none() {
        return Ok(None);
    }
    let root = settings
        .workspace_root
        .as_deref()
        .ok_or("Missing document workspace scope")?;
    let store = crate::documents::load_from_settings(settings)
        .ok_or("Configured document index failed to load")?;
    store
        .try_write()
        .map_err(|_| "Document store initialization is busy")?
        .restrict_workspace(root)
        .map_err(|error| error.to_string())?;
    Ok(Some(store))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn settings(root: &Path) -> Settings {
        let mut settings = Settings {
            workspace_root: Some(root.canonicalize().unwrap()),
            index_path: root.join("index"),
            ..Settings::default()
        };
        settings.documents.enabled = true;
        settings
    }

    #[test]
    fn hardening_workspace_documents_validate_stored_sources_before_model_loading() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("one");
        let other = temp.path().join("two");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&other).unwrap();
        let settings = settings(&root);
        assert!(prepare(&settings).unwrap().is_none());
        assert!(!settings.index_path.exists());
        let base = settings.index_path.join("documents");
        std::fs::create_dir_all(base.join("tantivy")).unwrap();
        std::fs::write(base.join("tantivy/meta.json"), "{}").unwrap();
        let state = |path: &Path| serde_json::json!({"file_states": {path.to_str().unwrap(): {}}});
        std::fs::write(
            base.join("state.json"),
            state(&other.join("guide.md")).to_string(),
        )
        .unwrap();
        assert!(prepare(&settings).is_err());
        std::fs::write(
            base.join("state.json"),
            state(&root.join("guide.md")).to_string(),
        )
        .unwrap();
        assert!(prepare(&settings).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn hardening_workspace_documents_refuse_another_projects_store_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("one");
        let other = temp.path().join("two");
        std::fs::create_dir_all(root.join("index")).unwrap();
        std::fs::create_dir(&other).unwrap();
        std::os::unix::fs::symlink(&other, root.join("index/documents")).unwrap();
        assert!(load(&settings(&root)).is_err());
        assert_eq!(std::fs::read_dir(&other).unwrap().count(), 0);
    }
}
