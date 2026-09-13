//! Handler for code file changes.
//!
//! Watches indexed source code files and triggers re-indexing on change.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::indexing::facade::IndexFacade;
use crate::watcher::{WatchAction, WatchError, WatchHandler};

/// Handler for code file changes.
///
/// Tracks files that are in the code index and returns reindex/remove
/// actions when they change.
pub struct CodeFileHandler {
    /// Shared reference to the facade.
    facade: Arc<RwLock<IndexFacade>>,
    /// Cached set of indexed paths for fast lookup.
    cached_paths: RwLock<HashSet<PathBuf>>,
    /// Cheap eligibility gates for paths not yet in the index.
    eligibility: RwLock<Eligibility>,
    /// Workspace root for path resolution.
    workspace_root: PathBuf,
}

/// Cheap gates for created files: enabled extensions and registered
/// roots. Over-approximates on purpose -- the exact ignore-chain check
/// runs post-debounce via `IndexFacade::discoverable_files`.
#[derive(Default)]
struct Eligibility {
    extensions: HashSet<String>,
    roots: Vec<PathBuf>,
}

impl CodeFileHandler {
    /// Create a new code file handler.
    pub fn new(facade: Arc<RwLock<IndexFacade>>, workspace_root: PathBuf) -> Self {
        Self {
            facade,
            cached_paths: RwLock::new(HashSet::new()),
            eligibility: RwLock::new(Eligibility::default()),
            workspace_root,
        }
    }

    /// Initialize the cached paths and eligibility gates from the facade.
    pub async fn init_cache(&self) {
        if let Err(error) = self.refresh_cache().await {
            tracing::error!("Code watch cache refresh failed: {error}");
        }
    }

    async fn refresh_cache(&self) -> Result<(), WatchError> {
        let workspace = self.workspace_root.clone();
        let snapshot = crate::runtime::read(&self.facade, move |facade| {
            let paths = facade.document_index().get_all_indexed_paths()?;
            let paths: HashSet<PathBuf> = paths
                .into_iter()
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        workspace.join(path)
                    }
                })
                .collect();
            let settings = facade.settings();
            let roots = settings.indexed_paths_cache.clone();
            let registry = crate::parsing::get_registry()
                .lock()
                .map_err(|_| crate::IndexError::MutexPoisoned)?;
            let extensions = registry
                .enabled_extensions(settings)
                .map(str::to_owned)
                .collect();
            Ok::<_, crate::IndexError>((paths, Eligibility { extensions, roots }))
        })
        .await
        .and_then(|value| value)
        .map_err(|error| WatchError::HandlerFailed {
            handler: "code cache".into(),
            path: self.workspace_root.clone(),
            reason: error.to_string(),
        })?;
        *self.cached_paths.write().await = snapshot.0;
        *self.eligibility.write().await = snapshot.1;
        Ok(())
    }

    /// Cheap gates for a path the index does not know yet: registered
    /// extension, not a dot-file, inside a registered root. Canonical
    /// fallback covers event paths arriving in a non-canonical form.
    fn eligible_unknown(&self, path: &Path) -> bool {
        let Ok(elig) = self.eligibility.try_read() else {
            return false;
        };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        if name.starts_with('.') {
            return false;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        if !elig.extensions.contains(ext) {
            return false;
        }
        if elig.roots.iter().any(|r| path.starts_with(r)) {
            return true;
        }
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        elig.roots.iter().any(|r| canonical.starts_with(r))
    }

    /// Convert an absolute path to relative for the indexer.
    fn to_relative(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.workspace_root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

#[async_trait]
impl WatchHandler for CodeFileHandler {
    fn name(&self) -> &str {
        "code"
    }

    fn matches(&self, path: &Path) -> bool {
        // Use cached paths for O(1) lookup
        if let Ok(cache) = self.cached_paths.try_read() {
            if cache.contains(path) {
                return true;
            }
        } else {
            // Cache locked, fall back to false
            // This is safe because we'll catch the event on retry
            return false;
        }
        // Unknown path: a created file is unknown by definition.
        self.eligible_unknown(path)
    }

    async fn tracked_paths(&self) -> Vec<PathBuf> {
        self.cached_paths.read().await.iter().cloned().collect()
    }

    async fn watch_roots(&self) -> Vec<PathBuf> {
        self.eligibility.read().await.roots.clone()
    }

    fn covered_by_batch_sync(&self) -> bool {
        // The batch incremental lane over a registered root replays this
        // handler's reindex/remove actions from disk-vs-index truth, and
        // is the only lane whose discovery pairs renames.
        true
    }

    async fn on_modify(&self, path: &Path) -> Result<WatchAction, WatchError> {
        let known = self.cached_paths.read().await.contains(path);
        if !known {
            // Created file: matches() only ran the cheap gates. Hold the
            // exact line here -- reindex only what the index walk itself
            // would discover (ignore chains included).
            let target = path.to_path_buf();
            let discoverable = crate::runtime::read(&self.facade, move |facade| {
                facade.discoverable_files(&target)
            })
            .await
            .and_then(|result| result)
            .map_err(|error| WatchError::HandlerFailed {
                handler: "code discovery".into(),
                path: path.to_path_buf(),
                reason: error.to_string(),
            })?;
            let discoverable = !discoverable.is_empty();
            if !discoverable {
                return Ok(WatchAction::None);
            }
            // Keep later edits of this file on the fast path.
            self.cached_paths.write().await.insert(path.to_path_buf());
        }
        Ok(WatchAction::ReindexCode {
            path: self.to_relative(path),
            created: !known,
        })
    }

    async fn on_delete(&self, path: &Path) -> Result<WatchAction, WatchError> {
        // Remove from cache
        {
            let mut cache = self.cached_paths.write().await;
            cache.remove(path);
        }

        Ok(WatchAction::RemoveCode {
            path: self.to_relative(path),
        })
    }

    async fn refresh_paths(&self) -> Result<(), WatchError> {
        self.refresh_cache().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    async fn handler_over(dir: &Path, root: &Path) -> CodeFileHandler {
        let mut settings = Settings {
            index_path: dir.join("index"),
            workspace_root: None,
            ..Default::default()
        };
        settings
            .add_indexed_path(root.to_path_buf())
            .expect("register indexed path");
        let facade = IndexFacade::new(Arc::new(settings)).unwrap();
        let handler = CodeFileHandler::new(Arc::new(RwLock::new(facade)), dir.to_path_buf());
        handler.init_cache().await;
        handler
    }

    // A created file is unknown to the index by definition; matches()
    // must accept it on the cheap gates (enabled extension, containment
    // in a registered root, not a dot-file) so the event reaches the
    // debouncer. The exact ignore-chain check runs later in on_modify.
    #[tokio::test]
    async fn matches_accepts_unknown_but_eligible_created_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let handler = handler_over(dir.path(), &root).await;
        let canonical_root = root.canonicalize().unwrap();

        assert!(
            handler.matches(&canonical_root.join("new_file.py")),
            "unknown file with enabled extension under a registered root"
        );
        assert!(
            !handler.matches(&canonical_root.join("notes.txt")),
            "extension not registered to any enabled language"
        );
        assert!(
            !handler.matches(&canonical_root.join(".hidden.py")),
            "dot-files never index"
        );
        assert!(
            !handler.matches(&dir.path().join("elsewhere/new_file.py")),
            "outside every registered root"
        );
    }

    // matches() over-approximates on cheap gates; on_modify holds the
    // exact line: an unknown path only produces ReindexCode when the
    // index walk itself would discover it (ignore chains included).
    #[tokio::test]
    async fn on_modify_refuses_walk_ineligible_created_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("generated")).unwrap();
        std::fs::write(root.join(".gitignore"), "generated/\n").unwrap();
        std::fs::write(root.join("a.py"), "def a():\n    pass\n").unwrap();
        std::fs::write(root.join("generated/b.py"), "def b():\n    pass\n").unwrap();
        let handler = handler_over(dir.path(), &root).await;
        let canonical_root = root.canonicalize().unwrap();

        let action = handler
            .on_modify(&canonical_root.join("a.py"))
            .await
            .unwrap();
        assert!(
            matches!(action, WatchAction::ReindexCode { .. }),
            "walk-discoverable created file must reindex, got {action:?}"
        );

        let action = handler
            .on_modify(&canonical_root.join("generated/b.py"))
            .await
            .unwrap();
        assert!(
            matches!(action, WatchAction::None),
            "gitignored created file must be refused, got {action:?}"
        );
    }
}
