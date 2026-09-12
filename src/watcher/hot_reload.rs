//! Hot-reload watcher for external index changes.
//!
//! Polls for changes to the index made by external processes (CI/CD, other terminals)
//! and hot-reloads them without restarting the server.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::info;

use crate::indexing::facade::IndexFacade;
use crate::mcp::notifications::{FileChangeEvent, NotificationBroadcaster};
use crate::{IndexPersistence, Settings};

/// Watches for external index changes and hot-reloads them.
///
/// This watcher polls `meta.json` and `state.json` to detect when the index
/// is modified by external processes (e.g., `codanna index` in another terminal,
/// CI/CD pipelines). It does NOT watch source files - that's handled by UnifiedWatcher.
pub struct HotReloadWatcher {
    index_path: PathBuf,
    facade: Arc<RwLock<IndexFacade>>,
    settings: Arc<Settings>,
    last_modified: Option<SystemTime>,
    last_doc_modified: Option<SystemTime>,
    check_interval: Duration,
    broadcaster: Option<Arc<NotificationBroadcaster>>,
}

impl HotReloadWatcher {
    /// Create a new hot-reload watcher.
    pub fn new(
        facade: Arc<RwLock<IndexFacade>>,
        settings: Arc<Settings>,
        check_interval: Duration,
    ) -> Self {
        let index_path = settings.index_path.clone();

        // Get initial modification time of the index metadata file
        let meta_file_path = index_path.join("tantivy").join("meta.json");
        let last_modified = std::fs::metadata(&meta_file_path)
            .ok()
            .and_then(|meta| meta.modified().ok());

        // Get initial modification time of document store state.json
        let doc_state_path = index_path.join("documents").join("state.json");
        let last_doc_modified = std::fs::metadata(&doc_state_path)
            .ok()
            .and_then(|meta| meta.modified().ok());

        Self {
            index_path,
            facade,
            settings,
            last_modified,
            last_doc_modified,
            check_interval,
            broadcaster: None,
        }
    }

    /// Set the notification broadcaster.
    pub fn with_broadcaster(mut self, broadcaster: Arc<NotificationBroadcaster>) -> Self {
        self.broadcaster = Some(broadcaster);
        self
    }

    /// Start watching for external index changes.
    pub async fn watch(mut self) {
        let mut ticker = interval(self.check_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;

            if let Err(e) = self.check_and_reload().await {
                tracing::error!("Error checking/reloading index: {e}");
            }
        }
    }

    /// Read filesystem state off the executor, then atomically replace the facade
    /// under the same mutation lane used by source watchers and MCP writes.
    async fn check_and_reload(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.index_path.clone();
        let (index_time, document_time) = crate::runtime::blocking(move || {
            fn modified(path: PathBuf) -> std::io::Result<Option<SystemTime>> {
                match std::fs::metadata(path) {
                    Ok(meta) => meta.modified().map(Some),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(e),
                }
            }
            Ok::<_, std::io::Error>((
                modified(path.join("tantivy/meta.json"))?,
                modified(path.join("documents/state.json"))?,
            ))
        })
        .await??;
        let doc_changed = document_time.is_some() && document_time != self.last_doc_modified;
        if let Some(observed) = index_time {
            if Some(observed) != self.last_modified {
                let settings = Arc::clone(&self.settings);
                let path = self.index_path.clone();
                crate::runtime::mutate(&self.facade, move |facade| {
                    let mut loaded = IndexPersistence::new(path.clone()).load_facade(settings)?;
                    if !loaded.has_semantic_search() && !loaded.is_semantic_incompatible() {
                        loaded.load_semantic_search(&path.join("semantic"))?;
                    }
                    // Do not replace live state if loading or validation failed.
                    *facade = loaded;
                    Ok::<_, crate::IndexError>(())
                })
                .await??;
                self.last_modified = Some(observed);
                if let Some(broadcaster) = &self.broadcaster {
                    broadcaster.send(FileChangeEvent::IndexReloaded);
                }
            }
        }
        self.last_doc_modified = document_time;
        if doc_changed {
            info!("Document store changed, notifying watchers");
            if let Some(broadcaster) = &self.broadcaster {
                broadcaster.send(FileChangeEvent::IndexReloaded);
            }
        }
        Ok(())
    }

    /// Statistics are fallible: a worker failure is not an empty successful index.
    pub async fn get_stats(&self) -> Result<IndexStats, crate::IndexError> {
        let last_modified = self.last_modified;
        let index_path = self.index_path.clone();
        crate::runtime::read(&self.facade, move |facade| IndexStats {
            symbol_count: facade.symbol_count(),
            last_modified,
            index_path,
        })
        .await
    }
}

/// Statistics about the watched index.
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub symbol_count: usize,
    pub last_modified: Option<SystemTime>,
    pub index_path: PathBuf,
}
