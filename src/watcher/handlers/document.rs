//! Handler for document file changes.
//!
//! Reconciles configured collections when documents or discovery policy change.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::documents::{ChunkingConfig, CollectionConfig, DocumentStore, DocumentsConfig};
use crate::watcher::{WatchAction, WatchError, WatchHandler};

/// Handler for document file changes.
///
/// Tracks files that are in the document index and returns reindex/remove
/// actions when they change.
pub struct DocumentFileHandler {
    /// Shared reference to the document store.
    store: Arc<RwLock<DocumentStore>>,
    /// Cached set of indexed paths for fast lookup.
    cached_paths: RwLock<HashSet<PathBuf>>,
    /// Workspace root for path resolution.
    workspace_root: PathBuf,
    /// Startup collection policy, shared by discovery and incremental updates.
    collections: Vec<(String, CollectionConfig)>,
    defaults: ChunkingConfig,
}

impl DocumentFileHandler {
    /// Create a new document file handler.
    pub fn new(store: Arc<RwLock<DocumentStore>>, workspace_root: PathBuf) -> Self {
        Self {
            store,
            cached_paths: RwLock::new(HashSet::new()),
            workspace_root,
            collections: Vec::new(),
            defaults: ChunkingConfig::default(),
        }
    }

    /// Track configured roots even while empty, using collection overrides on
    /// every update. Relative roots are resolved against the same workspace.
    pub fn with_config(mut self, config: &DocumentsConfig) -> Self {
        self.defaults = config.defaults.clone();
        self.collections = config
            .collections
            .iter()
            .map(|(name, collection)| {
                let mut collection = collection.clone();
                collection.paths = collection
                    .paths
                    .iter()
                    .map(|path| self.to_absolute(path))
                    .collect();
                (name.clone(), collection)
            })
            .collect();
        self.collections.sort_by(|a, b| a.0.cmp(&b.0));
        self
    }

    /// Initialize the cached paths from the store.
    pub async fn init_cache(&self) {
        let store = self.store.read().await;
        let paths: HashSet<PathBuf> = store
            .get_indexed_paths()
            .into_iter()
            .map(|p| self.to_absolute(&p))
            .collect();

        let mut cache = self.cached_paths.write().await;
        *cache = paths;
    }

    /// Convert a path to absolute using workspace root.
    fn to_absolute(&self, path: &Path) -> PathBuf {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        };
        crate::documents::store::normalize_source_path(&absolute)
    }

    fn affected_collections(&self, path: &Path) -> Vec<(String, CollectionConfig)> {
        let path = self.to_absolute(path);
        let policy_parent = path
            .file_name()
            .filter(|name| *name == ".codannaignore")
            .and_then(|_| path.parent());
        self.collections
            .iter()
            .filter(|(_, collection)| {
                collection.paths.iter().any(|root| {
                    // Reconcile an entire configured collection: the common
                    // discovery path applies patterns and .codannaignore.
                    path == *root
                        || root.starts_with(&path)
                        || policy_parent.is_some_and(|parent| {
                            parent.starts_with(root) || root.starts_with(parent)
                        })
                        || (path.starts_with(root) && Self::event_is_discoverable(root, &path))
                })
            })
            .cloned()
            .collect()
    }

    /// Apply the walker's exact path policy without walking the collection.
    /// A fresh matcher observes policy edits immediately and does not retain an
    /// unbounded cache of directories from historical event paths. Root and
    /// policy events bypass this source filter in affected_collections.
    fn event_is_discoverable(root: &Path, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(root) else {
            return false;
        };
        let mut walk = ignore::WalkBuilder::new(root);
        walk.hidden(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .follow_links(false)
            .add_custom_ignore_filename(".codannaignore");
        let Some(mut matcher) = walk.build_matchers().pop() else {
            return true;
        };
        let (matched, error) = matcher.matched_with_errors(relative, path.is_dir());
        // A policy read failure must reach the collection's normal error and
        // retry path instead of silently dropping a potentially relevant event.
        error.is_some() || !matched.is_ignore()
    }

    fn reconcile_action(&self, path: &Path) -> WatchAction {
        let collections = self.affected_collections(path);
        if collections.is_empty() {
            WatchAction::None
        } else {
            WatchAction::ReconcileDocuments {
                path: self.to_absolute(path),
                collections,
                defaults: self.defaults.clone(),
            }
        }
    }
}

#[async_trait]
impl WatchHandler for DocumentFileHandler {
    fn name(&self) -> &str {
        "document"
    }

    fn coalesces_document_events(&self) -> bool {
        !self.collections.is_empty()
    }

    fn matches(&self, path: &Path) -> bool {
        let absolute = self.to_absolute(path);
        // Discovery inherits parent policy. The root's parent watch must also
        // route policy edits, including while the configured root is absent.
        if absolute
            .file_name()
            .is_some_and(|name| name == ".codannaignore")
            && !self.affected_collections(&absolute).is_empty()
        {
            return true;
        }
        if let Ok(cache) = self.cached_paths.try_read() {
            if cache.contains(&absolute) {
                return true;
            }
        }
        self.collections.iter().any(|(_, collection)| {
            collection.paths.iter().any(|root| {
                if &absolute == root {
                    return true;
                }
                let Ok(relative) = absolute.strip_prefix(root) else {
                    return false;
                };
                absolute
                    .file_name()
                    .is_some_and(|name| name == ".codannaignore")
                    || collection.effective_patterns().iter().any(|pattern| {
                        glob::Pattern::new(pattern)
                            .is_ok_and(|pattern| pattern.matches_path(relative))
                    })
            })
        })
    }

    async fn tracked_paths(&self) -> Vec<PathBuf> {
        self.cached_paths.read().await.iter().cloned().collect()
    }

    async fn watch_roots(&self) -> Vec<PathBuf> {
        let collections = self.collections.clone();
        let workspace_root = self.workspace_root.clone();
        crate::runtime::blocking(move || {
            let mut roots = Vec::new();
            for (_, collection) in collections {
                for path in collection.paths {
                    // A watch on the root disappears with the root itself.
                    // Keep its parent observable before deletion, so a later
                    // recreation or directory move can reinstall the subtree.
                    if let Some(parent) = path.ancestors().skip(1).find(|p| p.is_dir()) {
                        roots.push(parent.to_path_buf());
                        // Custom ignore files are inherited by discovery. For
                        // nested roots, observe policy at every workspace
                        // ancestor, not only the immediate root parent.
                        roots.extend(
                            parent
                                .ancestors()
                                .skip(1)
                                .take_while(|ancestor| ancestor.starts_with(&workspace_root))
                                .map(Path::to_path_buf),
                        );
                    }
                    if path.is_file() || !path.exists() {
                        continue;
                    }
                    // Empty nested directories must be watched before their
                    // first document arrives. Ignore rules prune whole trees,
                    // while per-file exclusions still leave policy observable.
                    let mut walk = ignore::WalkBuilder::new(&path);
                    walk.hidden(false)
                        .ignore(false)
                        .git_ignore(false)
                        .git_global(false)
                        .git_exclude(false)
                        .follow_links(false)
                        .add_custom_ignore_filename(".codannaignore");
                    for entry in walk.build() {
                        match entry {
                            Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_dir()) => {
                                roots.push(entry.into_path())
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::warn!("[document] watch discovery failed: {error}")
                            }
                        }
                    }
                }
            }
            roots.sort();
            roots.dedup();
            roots
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!("[document] watch discovery failed: {error}");
            Vec::new()
        })
    }

    async fn on_modify(&self, path: &Path) -> Result<WatchAction, WatchError> {
        let action = self.reconcile_action(path);
        if !matches!(action, WatchAction::None) {
            return Ok(action);
        }
        // DocumentStore.file_states uses absolute paths, so pass absolute
        Ok(WatchAction::ReindexDocument {
            path: self.to_absolute(path),
        })
    }

    async fn on_delete(&self, path: &Path) -> Result<WatchAction, WatchError> {
        let action = self.reconcile_action(path);
        if !matches!(action, WatchAction::None) {
            return Ok(action);
        }
        let path = self.to_absolute(path);
        // Remove from cache
        {
            let mut cache = self.cached_paths.write().await;
            cache.remove(&path);
        }

        // DocumentStore.file_states uses absolute paths, so pass absolute
        Ok(WatchAction::RemoveDocument { path })
    }

    async fn refresh_paths(&self) -> Result<(), WatchError> {
        // Configured roots already match new files. Keep this cache tied to
        // indexed truth, avoiding another collection discovery after each
        // successful batch; overflow reconciliation explicitly scans roots.
        self.init_cache().await;
        Ok(())
    }

    async fn on_directory_change(&self, path: &Path) -> Result<WatchAction, WatchError> {
        Ok(self.reconcile_action(path))
    }
}
