//! Unified file watcher that routes events to pluggable handlers.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::{RwLock, mpsc};
use tokio::time::Duration;

use crate::documents::config::ChunkingConfig;
use crate::documents::{CollectionConfig, DocumentStore};
use crate::indexing::facade::IndexFacade;
use crate::mcp::notifications::{FileChangeEvent, NotificationBroadcaster};

use super::debouncer::Debouncer;
use super::error::WatchError;
use super::handler::{WatchAction, WatchHandler};
use super::path_registry::PathRegistry;

#[path = "config_reload.rs"]
mod config_reload;
use config_reload::PendingConfigReload;

/// Above this size, reconcile a modification burst through the shared batch
/// lane. This avoids one semantic-index save per path after large filesystem
/// event bursts such as a macOS wake or branch checkout.
const BATCH_MODIFICATION_THRESHOLD: usize = 32;

/// Bound collection debounce even during a continuous stream of source events.
const MAX_DOCUMENT_BATCH_DELAY: Duration = Duration::from_secs(2);

/// One pending truth scan per configured collection, independent of burst size.
/// Retry deadlines use the regular drain tick; they never sleep in the event lane.
#[derive(Clone)]
struct PendingDocumentCollection {
    config: CollectionConfig,
    defaults: ChunkingConfig,
    refresh_watches: bool,
    first_event_at: Instant,
    ready_at: Instant,
    failures: u32,
    retry_at: Option<Instant>,
}

impl PendingDocumentCollection {
    fn retry(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let delay_ms = (250_u64 << self.failures.saturating_sub(1).min(7)).min(30_000);
        self.retry_at = Some(now + Duration::from_millis(delay_ms));
    }
}

#[derive(Default)]
struct DocumentReconciliations {
    pending: BTreeMap<String, PendingDocumentCollection>,
    debounce: Duration,
    /// Accepted reload policy gates even actions captured before a reload.
    active: Option<crate::documents::DocumentsConfig>,
}

impl DocumentReconciliations {
    fn new(debounce_ms: u64) -> Self {
        Self {
            pending: BTreeMap::new(),
            debounce: Duration::from_millis(debounce_ms).min(MAX_DOCUMENT_BATCH_DELAY),
            active: None,
        }
    }

    fn record(
        &mut self,
        path: &Path,
        collections: Vec<(String, CollectionConfig)>,
        defaults: ChunkingConfig,
        now: Instant,
    ) {
        let (collections, defaults) = if let Some(active) = &self.active {
            let collections = collections
                .into_iter()
                .filter_map(|(name, _)| {
                    active
                        .enabled
                        .then(|| active.collections.get(&name).cloned())
                        .flatten()
                        .map(|config| (name, config))
                })
                .collect();
            (collections, active.defaults.clone())
        } else {
            (collections, defaults)
        };
        let refresh_watches = path.is_dir()
            || !path.exists()
            || path
                .file_name()
                .is_some_and(|name| name == ".codannaignore");
        for (name, config) in collections {
            self.pending
                .entry(name)
                .and_modify(|pending| {
                    pending.config = config.clone();
                    pending.defaults = defaults.clone();
                    pending.refresh_watches |= refresh_watches;
                    pending.ready_at = (now + self.debounce)
                        .min(pending.first_event_at + MAX_DOCUMENT_BATCH_DELAY);
                    // New events must not defeat the backoff of a failing
                    // collection; its next truth scan includes these changes.
                })
                .or_insert_with(|| PendingDocumentCollection {
                    config,
                    defaults: defaults.clone(),
                    refresh_watches,
                    first_event_at: now,
                    ready_at: now + self.debounce,
                    failures: 0,
                    retry_at: None,
                });
        }
    }

    fn take_ready(&mut self, now: Instant) -> BTreeMap<String, PendingDocumentCollection> {
        // Detach before awaiting the worker. Never clear the live map after a
        // scan: events received during it belong to the following dispatch.
        let names: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| {
                now >= pending.ready_at && pending.retry_at.is_none_or(|deadline| now >= deadline)
            })
            .map(|(name, _)| name.clone())
            .collect();
        names
            .into_iter()
            .filter_map(|name| self.pending.remove(&name).map(|pending| (name, pending)))
            .collect()
    }

    fn retry(&mut self, name: String, mut failed: PendingDocumentCollection, now: Instant) {
        failed.retry(now);
        self.pending
            .entry(name)
            .and_modify(|pending| {
                // Retain newer policy/work recorded during the failed scan.
                pending.refresh_watches |= failed.refresh_watches;
                pending.failures = pending.failures.max(failed.failures);
                pending.retry_at = pending.retry_at.max(failed.retry_at);
            })
            .or_insert(failed);
    }
}

/// Deterministic work counters, excluding watch-directory traversal and failed
/// scans whose partial file work is not reported by DocumentStore.
#[derive(Debug, Default)]
struct DocumentBatchStats {
    collection_scans: usize,
    files_checked: usize,
    files_processed: usize,
    chunks_removed: usize,
}

/// Unified file watcher with pluggable handlers.
///
/// Provides a single `notify::RecommendedWatcher` that routes file events
/// to appropriate handlers based on path matching.
pub struct UnifiedWatcher {
    /// Registered handlers.
    handlers: Vec<Box<dyn WatchHandler>>,
    /// Path registry for tracking and directory computation.
    registry: PathRegistry,
    /// Shared debouncer for all file events.
    debouncer: Debouncer,
    /// Channel for receiving file events.
    event_rx: mpsc::Receiver<notify::Result<Event>>,
    /// Set by the native notify callback when the bounded queue is full.
    /// The async lane converts this into a filesystem-truth reconciliation.
    event_overflowed: Arc<AtomicBool>,
    /// The underlying file watcher.
    _watcher: notify::RecommendedWatcher,
    /// Notification broadcaster for MCP integration.
    broadcaster: Arc<NotificationBroadcaster>,
    /// Shared facade for executing code actions.
    facade: Arc<RwLock<IndexFacade>>,
    /// Document store for executing document actions (optional).
    document_store: Option<Arc<RwLock<DocumentStore>>>,
    /// Coalesced collection work, bounded by configured collection count.
    document_reconciliations: RwLock<DocumentReconciliations>,
    /// Latest proposed configuration; failures retry without another file event.
    pending_config: RwLock<Option<PendingConfigReload>>,
    /// Chunking config for document re-indexing.
    chunking_config: ChunkingConfig,
    /// Path for semantic search persistence.
    index_path: PathBuf,
    /// Workspace root for path resolution.
    workspace_root: PathBuf,
    /// Registered watch roots from handlers; scopes created-directory
    /// handling and stays watched even when a root holds no indexed
    /// file directly.
    handler_roots: Vec<PathBuf>,
    /// Roots whose owning handler is covered by the batch incremental
    /// lane. Removal waves batch-sync these so the shared discovery can
    /// pair renames (remove + create of identical content).
    batch_sync_roots: Vec<PathBuf>,
    /// Native watches and handler snapshots are installed before transport admission.
    prepared: bool,
}

impl UnifiedWatcher {
    /// Create a builder for configuring the watcher.
    pub fn builder() -> UnifiedWatcherBuilder {
        UnifiedWatcherBuilder::new()
    }

    /// Install native watches and handler snapshots before accepting client work.
    /// Idempotent after success. A caller must discard this instance on failure.
    pub async fn prepare(&mut self) -> Result<(), WatchError> {
        if self.prepared {
            return Ok(());
        }
        // Initialize all handlers
        for handler in &self.handlers {
            handler
                .refresh_paths()
                .await
                .map_err(|error| WatchError::InitFailed {
                    reason: format!("{} handler: {error}", handler.name()),
                })?;
        }

        // Collect all paths from handlers and register them
        let mut all_paths = Vec::new();
        for handler in &self.handlers {
            all_paths.extend(handler.tracked_paths().await);
        }

        let new_dirs = self.registry.add_paths(all_paths);
        let total_paths = self.registry.path_count();
        let total_dirs = self.registry.dir_count();

        if total_paths == 0 {
            tracing::warn!("[watcher] no files to watch - index some files first");
        } else {
            crate::log_event!(
                "watcher",
                "monitoring",
                "{total_paths} files in {total_dirs} directories"
            );
        }

        // FSEvents watches roots recursively; register those first so the
        // per-directory batch can skip paths they already cover.
        self.register_handler_roots().await?;
        self.watch_directories(&new_dirs, false)?;
        self.refresh_code_policy_watches().await?;

        self.prepared = true;
        Ok(())
    }

    /// Run the event loop, preparing first for callers that do not admit a
    /// transport separately. Servers explicitly await `prepare` before handshake.
    pub async fn watch(self) -> Result<(), WatchError> {
        self.watch_until(tokio_util::sync::CancellationToken::new())
            .await
    }

    /// Cooperative shutdown: finish the current mutation before returning. The
    /// caller may safely retain an OS writer lease until this future completes.
    pub async fn watch_until(
        mut self,
        stop: tokio_util::sync::CancellationToken,
    ) -> Result<(), WatchError> {
        self.prepare().await?;

        // Subscribe to broadcaster for IndexReloaded events
        let mut broadcast_rx = self.broadcaster.subscribe();

        crate::log_event!("watcher", "started");

        // The drain fires on a fixed cadence, never deferred by event
        // pressure: a per-iteration sleep resets on every received
        // event, so any sustained stream with sub-interval arrivals
        // starves the drain -- and with it every debounced reindex,
        // removal wave, and notification -- for as long as the stream
        // lasts. The debouncer's own per-path quiet windows decide
        // what each tick actually drains.
        let mut drain = tokio::time::interval(Duration::from_millis(100));
        drain.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            // Keep fair selection: an always-ready event queue must not starve
            // debounce drains, overflow reconciliation, or cancellation.
            tokio::select! {
                _ = stop.cancelled() => return Ok(()),
                // Handle incoming file events
                Some(res) = self.event_rx.recv() => {
                    match res {
                        Ok(event) => {
                            self.handle_event(event).await;
                        }
                        Err(e) => {
                            tracing::error!("[watcher] file watch error: {e}");
                        }
                    }
                }

                // Process debounced changes and recover queue overflow from
                // filesystem truth. The native callback never blocks.
                _ = drain.tick() => {
                    self.dispatch_ready_changes(Instant::now()).await;
                }

                // Handle broadcast notifications
                Ok(event) = broadcast_rx.recv() => {
                    if matches!(event, FileChangeEvent::IndexReloaded) {
                        self.handle_index_reloaded().await;
                    }
                }
            }
        }
    }

    /// Register a batch without restarting the platform event stream per path.
    fn watch_directories(
        &mut self,
        dirs: &[PathBuf],
        recursive_roots: bool,
    ) -> Result<(), WatchError> {
        let watch_paths: Vec<_> = dirs
            .iter()
            .map(|dir| {
                if dir.is_absolute() {
                    dir.clone()
                } else {
                    self.workspace_root.join(dir)
                }
            })
            .filter(|path| {
                recursive_roots
                    || !cfg!(target_os = "macos")
                    || !self.handler_roots.iter().any(|root| path.starts_with(root))
            })
            .collect();
        if watch_paths.is_empty() {
            return Ok(());
        }
        // FSEvents has a finite path list. Recursive roots avoid one native
        // watch per source directory; handlers still enforce ignore rules.
        let mode = if cfg!(target_os = "macos") && recursive_roots {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        let mut paths = self._watcher.paths_mut();
        for watch_path in watch_paths {
            match paths.add(&watch_path, mode) {
                Ok(_) => {
                    crate::debug_event!(
                        "watcher",
                        "watching",
                        "{}",
                        crate::parsing::paths::render_absolute_path(&watch_path).display()
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "[watcher] failed to watch {}: {e}",
                        crate::parsing::paths::render_absolute_path(&watch_path).display()
                    );
                    return Err(WatchError::PathWatchFailed {
                        path: watch_path,
                        reason: e.to_string(),
                    });
                }
            }
        }
        paths.commit()?;
        Ok(())
    }

    /// Handle an incoming file event.
    async fn handle_event(&mut self, event: Event) {
        self.handle_event_at(event, Instant::now()).await;
    }

    async fn handle_event_at(&mut self, event: Event, now: Instant) {
        // Access events observe state; they never change it. inotify
        // emits Access(Open) for every directory read -- including the
        // watcher's OWN catch-up walks -- so routing them into the
        // directory branch below livelocks: walk emits Open, Open
        // triggers walk. FSEvents emits no Access events, which is why
        // only Linux exhibits it. The file-level kind match already
        // discards Access; state-bearing kinds are untouched.
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        for path in event.paths {
            let path = if path.is_absolute() {
                path
            } else {
                self.workspace_root.join(path)
            };
            let path = crate::documents::store::normalize_source_path(&path);
            // Managed artifacts are never source events, including when the
            // configured index lives outside the conventional .codanna tree.
            // Parent policy edits and workspace/root recreation remain visible.
            if path.starts_with(&self.index_path) {
                continue;
            }
            crate::trace_event!(
                "watcher",
                "event",
                "{:?} {}",
                event.kind,
                crate::parsing::paths::render_absolute_path(&path).display()
            );
            let is_directory = path.is_dir();
            let removed_directory = !path.exists()
                && self
                    .registry
                    .watch_dirs()
                    .iter()
                    .any(|directory| directory.starts_with(&path));
            if matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                // An observed newer settings edit supersedes a failed proposal
                // immediately; its debounce must not let the old retry win.
                for handler in self
                    .handlers
                    .iter()
                    .filter(|handler| handler.reloads_config())
                {
                    if handler.matches(&path)
                        || ((is_directory || removed_directory)
                            && handler
                                .tracked_paths()
                                .await
                                .iter()
                                .any(|settings| settings.starts_with(&path)))
                    {
                        *self.pending_config.write().await = None;
                    }
                }
                for handler in &self.handlers {
                    if !handler.coalesces_document_events()
                        || !(is_directory || removed_directory || handler.matches(&path))
                    {
                        continue;
                    }
                    match handler.on_directory_change(&path).await {
                        Ok(WatchAction::ReconcileDocuments {
                            path,
                            collections,
                            defaults,
                        }) => {
                            if self.document_store.is_some() {
                                self.document_reconciliations.write().await.record(
                                    &path,
                                    collections,
                                    defaults,
                                    now,
                                );
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::error!("[{}] event routing failed: {error}", handler.name());
                            self.event_overflowed.store(true, Ordering::Release);
                        }
                    }
                }
            }
            // Ignore files alter the inventory, including already-indexed
            // files and currently empty subtrees. They are policy events even
            // though source handlers deliberately reject unknown dot-files.
            if matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some(".gitignore" | ".codannaignore")
            ) && self
                .batch_sync_roots
                .iter()
                .any(|root| path.starts_with(root))
            {
                // Use the settled-wave lane so policy changes reconcile roots
                // once after the whole save/rename burst has quieted.
                self.debouncer.record_removal(path);
                continue;
            }

            // A directory never matches a file handler (extension gate);
            // it is the watcher's own concern: extend the watch set and
            // catch up files that landed before the watch existed. Disk
            // truth decides, not event kind -- a dir rename's to-side
            // arrives as Modify(Name), never Create.
            if is_directory {
                if let Err(error) = self.handle_created_directory(&path).await {
                    tracing::error!("[watcher] created-directory discovery incomplete: {error}");
                }
                continue;
            }

            // A vanished path that prefixes watched directories is a
            // directory removal observation (dir-rename from-side or true
            // dir delete). It arrives as Modify(Name) or a stale Create
            // with NO per-file events following; one removal observation
            // stands in for the subtree and the wave's batch sync
            // re-derives the owning root.
            if removed_directory
                && self
                    .handlers
                    .iter()
                    .any(|handler| !handler.coalesces_document_events())
            {
                self.debouncer.record_removal(path);
                continue;
            }

            // Check if any handler cares about this path
            let matched = self
                .handlers
                .iter()
                .any(|handler| !handler.coalesces_document_events() && handler.matches(&path));
            if !matched {
                crate::trace_event!(
                    "watcher",
                    "unmatched",
                    "{:?} {}",
                    event.kind,
                    crate::parsing::paths::render_absolute_path(&path).display()
                );
                continue;
            }

            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    // Debounce creations and modifications alike; the
                    // exists() re-check in process_modification handles
                    // paths that vanish before the debounce fires.
                    self.debouncer.record(path);
                }
                EventKind::Remove(_) => {
                    // Deferred, not immediate: a rename arrives as
                    // remove(old) + create(new), and only a batch holding
                    // both sides lets the shared discovery pair them.
                    // Genuine deletions pay one debounce window before
                    // cleanup.
                    self.debouncer.record_removal(path);
                }
                _ => {}
            }
        }
    }

    /// Register handler watch roots: watched directly so directory
    /// creation at the top of a root is visible even when the root
    /// holds no indexed file directly.
    async fn register_handler_roots(&mut self) -> Result<(), WatchError> {
        self.register_handler_roots_with_reinstall(&[]).await
    }

    /// Reinstall affected document roots even when their path strings were
    /// registered before a delete/recreate replaced the underlying inode.
    async fn register_handler_roots_with_reinstall(
        &mut self,
        reinstall_roots: &[PathBuf],
    ) -> Result<(), WatchError> {
        let mut roots = Vec::new();
        let mut sync_roots = Vec::new();
        for handler in &self.handlers {
            let handler_roots = handler.watch_roots().await;
            if handler.covered_by_batch_sync() {
                sync_roots.extend(handler_roots.iter().cloned());
            }
            roots.extend(handler_roots);
        }
        roots.retain(|root| !root.starts_with(&self.index_path));
        let new_roots: Vec<_> = roots
            .iter()
            .filter(|root| {
                let reinstall = reinstall_roots
                    .iter()
                    .any(|affected| root.starts_with(affected) || affected.starts_with(root));
                // A shared code/document root can be absent after deletion.
                // Keep its parent's document watch and reconcile its empty
                // inventory; attempting a native watch would block cleanup.
                // Startup registration still fails on an invalid missing root.
                if reinstall && !root.exists() {
                    return false;
                }
                let new_dir = self.registry.add_watch_dir((*root).clone());
                new_dir || !self.handler_roots.contains(root) || reinstall
            })
            .cloned()
            .collect();
        self.watch_directories(&new_roots, true)?;
        self.handler_roots = roots;
        self.batch_sync_roots = sync_roots;
        Ok(())
    }

    /// Watch traversable source directories even when their ignore policy
    /// currently excludes every code file. This runs at preparation/reload,
    /// never on a query; a later nested policy edit can include files again.
    async fn refresh_code_policy_watches(&mut self) -> Result<(), WatchError> {
        let roots = self.batch_sync_roots.clone();
        let directories = crate::runtime::read(&self.facade, move |facade| {
            let mut directories = Vec::new();
            for root in roots {
                directories.extend(facade.discoverable_dirs(&root)?);
            }
            Ok::<_, crate::IndexError>(directories)
        })
        .await
        .map_err(|error| WatchError::EventError {
            details: error.to_string(),
        })?
        .map_err(|error| WatchError::EventError {
            details: error.to_string(),
        })?;
        let new: Vec<_> = directories
            .into_iter()
            .filter(|directory| self.registry.add_watch_dir(directory.clone()))
            .collect();
        self.watch_directories(&new, false)
    }

    /// A directory appeared under a registered root: watch every
    /// traversable directory of the new subtree (ignore chains anchored
    /// at the root prune ignored trees), then route the files already
    /// inside through the normal debounce -> eligibility -> reindex path.
    async fn handle_created_directory(&mut self, path: &Path) -> Result<(), WatchError> {
        if !self.handler_roots.iter().any(|r| path.starts_with(r)) {
            return Ok(());
        }

        // A directory move can contain documents before individual file
        // watches exist. Collection handlers reconcile through their own
        // discovery policy; the code lane below keeps its existing behavior.
        for handler in &self.handlers {
            if handler.coalesces_document_events() {
                // Ingress already queued document directory events.
                continue;
            }
            let action = handler.on_directory_change(path).await?;
            self.execute_action(action, handler.name()).await?;
        }
        // Document roots and their watches reconcile once on the next drain,
        // before scanning content. The parent watch remains active meanwhile.

        if !self
            .handlers
            .iter()
            .any(|handler| handler.covered_by_batch_sync())
        {
            return Ok(());
        }

        let path_owned = path.to_path_buf();
        let (dirs, files) = crate::runtime::read(&self.facade, move |facade| {
            Ok::<_, crate::IndexError>((
                facade.discoverable_dirs(&path_owned)?,
                facade.discoverable_files(&path_owned)?,
            ))
        })
        .await
        .map_err(|e| WatchError::EventError {
            details: e.to_string(),
        })?
        .map_err(|e| WatchError::EventError {
            details: e.to_string(),
        })?;

        let new_dirs: Vec<_> = dirs
            .into_iter()
            .filter(|dir| self.registry.add_watch_dir(dir.clone()))
            .collect();
        if let Err(e) = self.watch_directories(&new_dirs, false) {
            tracing::warn!("[watcher] failed to watch created directories: {e}");
        }
        if !files.is_empty() {
            crate::log_event!(
                "watcher",
                "created dir",
                "{} ({} files to catch up)",
                crate::parsing::paths::render_absolute_path(path).display(),
                files.len()
            );
        }
        for file in files {
            self.debouncer.record(file);
        }
        Ok(())
    }

    /// Recover from a full native-event queue by deriving state from the
    /// filesystem and handler snapshots instead of relying on dropped events.
    async fn reconcile_event_overflow(&mut self, now: Instant) {
        tracing::warn!(
            "[watcher] event queue overflowed; reconciling watched state from filesystem truth"
        );

        // Code handlers already have a robust incremental directory lane. Run
        // every covered root once; this observes creates, modifications,
        // deletions and renames regardless of which individual events were lost.
        let roots = self.batch_sync_roots.clone();
        if let Err(error) = self.synchronize_roots(roots).await {
            tracing::error!("[watcher] overflow sync failed: {error}");
        }

        // Handlers outside the shared code batch lane (documents/config) need
        // their own truth reconciliation. Compare tracked paths before/after
        // refresh so missed removals remain observable, then replay current
        // files through normal modify handling.
        for handler in &self.handlers {
            if handler.covered_by_batch_sync() {
                continue;
            }

            // Collection handlers can reconcile empty, deleted and newly
            // populated roots directly. Replaying every file would multiply
            // collection discovery work and miss untracked sources.
            let mut collection_handler = false;
            let mut observed_roots: Vec<PathBuf> = Vec::new();
            let mut watch_roots = handler.watch_roots().await;
            watch_roots.sort();
            for root in watch_roots {
                if observed_roots.iter().any(|parent| root.starts_with(parent)) {
                    continue;
                }
                observed_roots.push(root.clone());
                match handler.on_directory_change(&root).await {
                    Ok(WatchAction::ReconcileDocuments {
                        path,
                        collections,
                        defaults,
                    }) => {
                        collection_handler = true;
                        if self.document_store.is_some() {
                            self.document_reconciliations.write().await.record(
                                &path,
                                collections,
                                defaults,
                                now,
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(error) => tracing::error!(
                        "[{}] overflow reconciliation failed: {error}",
                        handler.name()
                    ),
                }
            }
            if collection_handler {
                continue;
            }

            let before = handler.tracked_paths().await;
            if let Err(e) = handler.refresh_paths().await {
                tracing::warn!(
                    "[watcher] overflow refresh failed for {}: {e}",
                    handler.name()
                );
                continue;
            }
            let after = handler.tracked_paths().await;
            let before_set: HashSet<PathBuf> = before.into_iter().collect();
            let after_set: HashSet<PathBuf> = after.into_iter().collect();

            for path in before_set.difference(&after_set) {
                match handler.on_delete(path).await {
                    Ok(action) => {
                        if let Err(e) = self.execute_action(action, handler.name()).await {
                            tracing::error!(
                                "[{}] overflow delete action error: {e}",
                                handler.name()
                            );
                        }
                    }
                    Err(e) => {
                        if handler.reloads_config() {
                            *self.pending_config.write().await = None;
                        }
                        tracing::error!("[{}] overflow delete error: {e}", handler.name());
                    }
                }
            }
            for path in &after_set {
                if !path.exists() && !handler.reloads_config() {
                    continue;
                }
                let action = if path.exists() {
                    handler.on_modify(path).await
                } else {
                    handler.on_delete(path).await
                };
                match action {
                    Ok(action) => {
                        if let Err(e) = self.execute_action(action, handler.name()).await {
                            tracing::error!(
                                "[{}] overflow modify action error: {e}",
                                handler.name()
                            );
                        }
                    }
                    Err(e) => {
                        if handler.reloads_config() {
                            *self.pending_config.write().await = None;
                        }
                        tracing::error!("[{}] overflow modify error: {e}", handler.name());
                    }
                }
            }
        }

        // Rebuild watcher registrations/caches after filesystem reconciliation.
        self.handle_index_reloaded().await;
        self.broadcaster.send(FileChangeEvent::IndexReloaded);
    }

    /// One fixed-cadence dispatch. Explicit time keeps retry tests independent
    /// of OS notification timing and real debounce sleeps.
    async fn dispatch_ready_changes(&mut self, now: Instant) -> DocumentBatchStats {
        if self.event_overflowed.swap(false, Ordering::AcqRel) {
            self.reconcile_event_overflow(now).await;
        }

        if self.debouncer.has_pending_removals() {
            // Hold both sides of a possible rename until the whole burst is
            // stable, so shared code discovery can pair remove and create.
            if let Some((removed, modified)) = self.debouncer.take_settled_burst() {
                self.process_change_wave(removed, modified).await;
            }
        } else {
            let ready = self.debouncer.take_ready();
            let ready_count = ready.len();
            let (vanished, alive): (Vec<PathBuf>, Vec<PathBuf>) =
                ready.into_iter().partition(|path| !path.exists());
            if ready_count >= BATCH_MODIFICATION_THRESHOLD {
                self.process_change_wave(vanished, alive).await;
            } else if vanished.is_empty() {
                for path in alive {
                    self.process_modification(&path).await;
                }
            } else {
                // Rename-as-modify: defer the vanished paths and surviving
                // create sides together to the next settled removal wave.
                for path in vanished {
                    self.debouncer.record_removal(path);
                }
                for path in alive {
                    self.debouncer.record(path);
                }
            }
        }
        self.flush_config_reload(now).await;
        // Keep the previous generation intact while a configuration transaction
        // is waiting to retry. Native source events remain bounded/coalesced.
        if self.pending_config.read().await.is_some() {
            return DocumentBatchStats::default();
        }
        // This also runs on otherwise idle ticks, making failed collections
        // retry without requiring another filesystem event.
        self.flush_document_reconciliations(now).await
    }

    /// Process a debounced file modification.
    async fn process_modification(&self, path: &Path) {
        // Vanished since the drain: the removal lane owns it -- the
        // caller recorded a removal observation, or the Remove event is
        // in flight.
        if !path.exists() {
            return;
        }

        for handler in &self.handlers {
            if handler.coalesces_document_events() || !handler.matches(path) {
                continue;
            }

            crate::log_event!(
                handler.name(),
                "modified",
                "{}",
                crate::parsing::paths::render_absolute_path(path).display()
            );

            match handler.on_modify(path).await {
                Ok(action) => {
                    if let Err(e) = self.execute_action(action, handler.name()).await {
                        tracing::error!("[{}] action error: {e}", handler.name());
                    }
                }
                Err(e) => {
                    if handler.reloads_config() {
                        *self.pending_config.write().await = None;
                    }
                    tracing::error!("[{}] handler error: {e}", handler.name());
                }
            }
        }
    }

    /// Process one settled removal wave or a large modification burst.
    ///
    /// Roots owned by a batch-sync-covered handler run the shared batch
    /// incremental lane: its discovery re-derives new/modified/deleted
    /// from disk-vs-index truth and pairs renames -- the one boundary
    /// all incremental entry points share. Paths outside every synced
    /// root keep per-file semantics.
    async fn process_change_wave(&mut self, removed: Vec<PathBuf>, modified: Vec<PathBuf>) {
        let mut roots: Vec<PathBuf> = Vec::new();
        for path in removed.iter().chain(modified.iter()) {
            if let Some(root) = self
                .batch_sync_roots
                .iter()
                .find(|root| path.starts_with(root))
            {
                if !roots.contains(root) {
                    roots.push(root.clone());
                }
            }
        }

        // Resolution defers across the covered roots so a burst whose
        // importing and imported files land in different roots binds
        // its cross-root edges regardless of loop order.
        if let Err(error) = self
            .synchronize_roots_after_removals(roots.clone(), removed.clone())
            .await
        {
            tracing::error!("[watcher] batch sync failed: {error}");
        }

        if !roots.is_empty() {
            // Handler caches and subscribers refresh through the same
            // event hot-reload uses; the sync may have relocated paths.
            self.broadcaster.send(FileChangeEvent::IndexReloaded);
        }

        // Per-file semantics for everything the batch sync does not
        // subsume: paths outside every synced root, and handlers not
        // covered by the batch lane even under one (document files can
        // live inside a code root).
        for path in &removed {
            let covered = roots.iter().any(|root| path.starts_with(root));
            self.process_wave_residual(path, covered, true).await;
        }
        for path in &modified {
            let covered = roots.iter().any(|root| path.starts_with(root));
            if !path.exists() {
                continue;
            }
            self.process_wave_residual(path, covered, false).await;
        }
    }

    /// Route one wave path through every handler the batch sync did not
    /// subsume.
    async fn process_wave_residual(&self, path: &Path, batch_covered: bool, is_removal: bool) {
        for handler in &self.handlers {
            if handler.coalesces_document_events() {
                continue;
            }
            if !handler.matches(path) {
                if is_removal && !handler.covered_by_batch_sync() {
                    match handler.on_directory_change(path).await {
                        Ok(action) => {
                            if let Err(error) = self.execute_action(action, handler.name()).await {
                                tracing::error!(
                                    "[{}] directory reconciliation failed: {error}",
                                    handler.name()
                                );
                            }
                        }
                        Err(error) => tracing::error!(
                            "[{}] directory reconciliation failed: {error}",
                            handler.name()
                        ),
                    }
                }
                continue;
            }
            if batch_covered && handler.covered_by_batch_sync() {
                continue;
            }

            let (verb, result) = if is_removal {
                ("deleted", handler.on_delete(path).await)
            } else {
                ("modified", handler.on_modify(path).await)
            };
            crate::log_event!(
                handler.name(),
                verb,
                "{}",
                crate::parsing::paths::render_absolute_path(path).display()
            );

            match result {
                Ok(action) => {
                    if let Err(e) = self.execute_action(action, handler.name()).await {
                        tracing::error!("[{}] action error: {e}", handler.name());
                    }
                }
                Err(e) => {
                    if handler.reloads_config() {
                        *self.pending_config.write().await = None;
                    }
                    tracing::error!("[{}] handler error: {e}", handler.name());
                }
            }
        }
    }

    /// Complete a multi-root code mutation in one serialized worker transaction.
    /// Already committed files can survive an error, but the error is not hidden.
    async fn synchronize_roots(&self, roots: Vec<PathBuf>) -> Result<(), WatchError> {
        self.synchronize_roots_after_removals(roots, Vec::new())
            .await
    }

    async fn synchronize_roots_after_removals(
        &self,
        roots: Vec<PathBuf>,
        removed: Vec<PathBuf>,
    ) -> Result<(), WatchError> {
        if roots.is_empty() {
            return Ok(());
        }
        crate::runtime::mutate(&self.facade, move |indexer| {
            let mut pending = crate::indexing::pipeline::PendingResolution::default();
            let mut failures = Vec::new();
            for root in roots {
                if removed.iter().any(|path| root.starts_with(path)) {
                    match indexer.remove_observed_directory(&root) {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(error) => {
                            failures.push(format!("{}: {error}", root.display()));
                            continue;
                        }
                    }
                }
                if let Err(error) = indexer.index_directory_deferred(&root, false, &mut pending) {
                    failures.push(format!("{}: {error}", root.display()));
                }
            }
            if let Err(error) = indexer.resolve_deferred(pending) {
                failures.push(error.to_string());
            }
            if failures.is_empty() {
                Ok(())
            } else {
                Err(WatchError::EventError {
                    details: failures.join("; "),
                })
            }
        })
        .await
        .map_err(|e| WatchError::EventError {
            details: e.to_string(),
        })?
    }

    /// Serialized code mutations and exclusive legacy document mutations run
    /// off Tokio. Configured collections queue for one scan per dispatch.
    /// Notifications follow publication, never precede it.
    async fn execute_action(
        &self,
        action: WatchAction,
        handler_name: &str,
    ) -> Result<(), WatchError> {
        let result: Result<Option<FileChangeEvent>, WatchError> = match action {
            WatchAction::ReindexCode { path, created } => {
                let semantic_path = self.index_path.join("semantic");
                let source_path = if path.is_absolute() {
                    path.clone()
                } else {
                    self.workspace_root.join(&path)
                };
                crate::runtime::mutate(&self.facade, move |indexer| {
                    let result = indexer.index_file(&source_path)?;
                    indexer.save_semantic_search(&semantic_path)?;
                    Ok::<_, crate::IndexError>(match result {
                        crate::IndexingResult::Indexed(_) => Some(if created {
                            FileChangeEvent::FileCreated { path }
                        } else {
                            FileChangeEvent::FileReindexed { path }
                        }),
                        crate::IndexingResult::Cached(_) => None,
                    })
                })
                .await
                .map_err(|e| WatchError::EventError {
                    details: e.to_string(),
                })?
                .map_err(|e| WatchError::EventError {
                    details: e.to_string(),
                })
            }
            WatchAction::RemoveCode { path } => {
                let semantic_path = self.index_path.join("semantic");
                let source_path = if path.is_absolute() {
                    path.clone()
                } else {
                    self.workspace_root.join(&path)
                };
                crate::runtime::mutate(&self.facade, move |indexer| {
                    indexer.remove_file(&source_path)?;
                    indexer.save_semantic_search(&semantic_path)?;
                    Ok::<_, crate::IndexError>(Some(FileChangeEvent::FileDeleted { path }))
                })
                .await
                .map_err(|e| WatchError::EventError {
                    details: e.to_string(),
                })?
                .map_err(|e| WatchError::EventError {
                    details: e.to_string(),
                })
            }
            WatchAction::ReconcileDocuments {
                path,
                collections,
                defaults,
            } => {
                if self.document_store.is_some() {
                    self.document_reconciliations.write().await.record(
                        &path,
                        collections,
                        defaults,
                        Instant::now(),
                    );
                }
                Ok(None)
            }
            WatchAction::ReindexDocument { path } => {
                if let Some(store) = self.document_store.clone() {
                    let config = self.chunking_config.clone();
                    crate::runtime::blocking(move || {
                        store
                            .blocking_write()
                            .reindex_file(&path, &config)
                            .map(|result| result.map(|_| FileChangeEvent::FileReindexed { path }))
                            .map_err(|e| WatchError::EventError {
                                details: e.to_string(),
                            })
                    })
                    .await
                    .map_err(|e| WatchError::EventError {
                        details: e.to_string(),
                    })?
                } else {
                    Ok(None)
                }
            }
            WatchAction::RemoveDocument { path } => {
                if let Some(store) = self.document_store.clone() {
                    crate::runtime::blocking(move || {
                        store
                            .blocking_write()
                            .remove_file(&path)
                            .map(|removed| removed.then_some(FileChangeEvent::FileDeleted { path }))
                            .map_err(|e| WatchError::EventError {
                                details: e.to_string(),
                            })
                    })
                    .await
                    .map_err(|e| WatchError::EventError {
                        details: e.to_string(),
                    })?
                } else {
                    Ok(None)
                }
            }
            WatchAction::ReloadConfig {
                added,
                removed,
                current,
            } => {
                let changed = !added.is_empty() || !removed.is_empty();
                // The running facade and pipeline own an Arc snapshot of the
                // startup settings. Refresh it before indexing so subsequent
                // discovery and CodeFileHandler eligibility see new roots.
                crate::runtime::mutate(&self.facade, move |indexer| {
                    indexer.reload_indexed_paths(current);
                })
                .await
                .map_err(|e| WatchError::EventError {
                    details: e.to_string(),
                })?;

                if !added.is_empty() {
                    crate::log_event!("config", "adding directories", "{}", added.len());
                    for path in &added {
                        tracing::info!(
                            "  + {}",
                            crate::parsing::paths::render_absolute_path(path).display()
                        );
                    }
                    self.synchronize_roots(added).await?;
                }
                if !removed.is_empty() {
                    tracing::info!(
                        "Run 'codanna clean' to remove symbols from removed directories"
                    );
                }
                Ok(changed.then_some(FileChangeEvent::IndexReloaded))
            }
            WatchAction::ReloadSettings { settings } => {
                *self.pending_config.write().await = Some(PendingConfigReload::new(settings));
                Ok(None)
            }
            WatchAction::None => Ok(None),
        };
        match result {
            Ok(Some(event)) => {
                self.broadcaster.send(event);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(error) => {
                tracing::error!("[{handler_name}] mutation failed: {error}");
                Err(error)
            }
        }
    }

    /// Flush each due collection once. Native events arriving while this awaits
    /// remain in event_rx (or latch overflow); retries remain in the bounded map.
    async fn flush_document_reconciliations(&mut self, now: Instant) -> DocumentBatchStats {
        let mut total = DocumentBatchStats::default();
        let ready = self.document_reconciliations.write().await.take_ready(now);
        if ready.is_empty() {
            return total;
        }
        let started = Instant::now();
        let mut reinstall_roots: Vec<_> = ready
            .values()
            .filter(|pending| pending.refresh_watches)
            .flat_map(|pending| pending.config.paths.iter().cloned())
            .collect();
        reinstall_roots.sort();
        reinstall_roots.dedup();
        if !reinstall_roots.is_empty() {
            // Install watches first; the following truth scan catches files
            // that arrived before installation, and later writes queue events.
            if let Err(error) = self
                .register_handler_roots_with_reinstall(&reinstall_roots)
                .await
            {
                tracing::error!("[document] batch watch registration failed; retrying: {error}");
                let mut queue = self.document_reconciliations.write().await;
                for (name, pending) in ready {
                    queue.retry(name, pending, now.max(Instant::now()));
                }
                return total;
            }
        }
        for (name, pending) in ready {
            let Some(store) = self.document_store.clone() else {
                continue;
            };
            let config = pending.config.clone();
            let effective = config.effective_chunking(&pending.defaults);
            let collection_name = name.clone();
            total.collection_scans += 1;
            let result = crate::runtime::blocking(move || {
                store
                    .blocking_write()
                    .index_collection(&collection_name, &config, &effective)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(std::convert::identity);
            match result {
                Ok(stats) => {
                    total.files_checked += stats.files_processed + stats.files_skipped;
                    total.files_processed += stats.files_processed;
                    total.chunks_removed += stats.chunks_removed;
                }
                Err(error) => {
                    tracing::error!(
                        "[document] collection '{name}' reconciliation failed; retrying: {error}"
                    );
                    self.document_reconciliations.write().await.retry(
                        name,
                        pending,
                        now.max(Instant::now()),
                    );
                }
            }
        }
        tracing::debug!(
            target: "rag",
            collection_scans = total.collection_scans,
            files_checked = total.files_checked,
            files_processed = total.files_processed,
            elapsed_ms = started.elapsed().as_millis(),
            "document watcher dispatch completed"
        );
        if total.files_processed > 0 || total.chunks_removed > 0 {
            // A collection scan can change many sources, including sources
            // whose native events were lost. Request a complete client refresh
            // without retaining an unbounded list of individual source paths.
            self.broadcaster.send(FileChangeEvent::IndexReloaded);
        }
        total
    }

    /// Handle IndexReloaded notification - refresh all handlers.
    async fn handle_index_reloaded(&mut self) {
        crate::log_event!("watcher", "index reloaded, refreshing");

        for handler in &self.handlers {
            if let Err(e) = handler.refresh_paths().await {
                tracing::warn!(
                    "[watcher] failed to refresh {} handler: {e}",
                    handler.name()
                );
            }
        }

        // Rebuild path registry
        let mut all_paths = Vec::new();
        for handler in &self.handlers {
            all_paths.extend(handler.tracked_paths().await);
        }

        let old_dirs: HashSet<PathBuf> = self.registry.watch_dirs().clone();
        self.registry.rebuild(all_paths);

        // Collect new directories before mutably borrowing self
        let dirs_to_watch: Vec<PathBuf> = self
            .registry
            .watch_dirs()
            .difference(&old_dirs)
            .cloned()
            .collect();

        // Config reload can add or drop roots. Register roots first so macOS
        // can cover a large new tree with one recursive FSEvents path.
        let previous_roots = self.batch_sync_roots.clone();
        if let Err(error) = self.register_handler_roots().await {
            tracing::error!("[watcher] root registration incomplete: {error}");
        }
        let added_roots: Vec<_> = self
            .batch_sync_roots
            .iter()
            .filter(|root| !previous_roots.contains(root))
            .cloned()
            .collect();

        // Watch any new directories not already covered by a recursive root.
        if let Err(e) = self.watch_directories(&dirs_to_watch, false) {
            tracing::warn!("[watcher] failed to watch new directories: {e}");
        }

        if let Err(error) = self.refresh_code_policy_watches().await {
            tracing::error!("[watcher] ignore-policy watch registration incomplete: {error}");
        }

        // Close the index-then-register race for code roots only. Document
        // roots include policy-watch parents outside the configured code
        // inventory and must never expand code indexing as a side effect.
        // A file can land after config
        // indexing completes but before the new native root is committed.
        // With the watch active, one incremental truth scan catches that gap;
        // later writes are queued by the native watcher.
        if !added_roots.is_empty() {
            if let Err(e) = self.synchronize_roots(added_roots).await {
                tracing::error!("[watcher] new-root catch-up failed: {e}");
            }
            for handler in &self.handlers {
                if let Err(e) = handler.refresh_paths().await {
                    tracing::warn!(
                        "[watcher] failed to refresh {} handler after root catch-up: {e}",
                        handler.name()
                    );
                }
            }
        }
        crate::log_event!(
            "watcher",
            "watching",
            "{} files in {} directories",
            self.registry.path_count(),
            self.registry.dir_count()
        );
    }
}

/// Builder for constructing a UnifiedWatcher.
pub struct UnifiedWatcherBuilder {
    handlers: Vec<Box<dyn WatchHandler>>,
    broadcaster: Option<Arc<NotificationBroadcaster>>,
    facade: Option<Arc<RwLock<IndexFacade>>>,
    document_store: Option<Arc<RwLock<DocumentStore>>>,
    chunking_config: ChunkingConfig,
    index_path: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
    debounce_ms: u64,
}

impl UnifiedWatcherBuilder {
    /// Create a new builder with defaults.
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
            broadcaster: None,
            facade: None,
            document_store: None,
            chunking_config: ChunkingConfig::default(),
            index_path: None,
            workspace_root: None,
            debounce_ms: 500,
        }
    }

    /// Add a handler.
    pub fn handler(mut self, handler: impl WatchHandler + 'static) -> Self {
        self.handlers.push(Box::new(handler));
        self
    }

    /// Set the notification broadcaster.
    pub fn broadcaster(mut self, broadcaster: Arc<NotificationBroadcaster>) -> Self {
        self.broadcaster = Some(broadcaster);
        self
    }

    /// Set the facade (renamed from indexer).
    pub fn indexer(mut self, facade: Arc<RwLock<IndexFacade>>) -> Self {
        self.facade = Some(facade);
        self
    }

    /// Set the document store.
    pub fn document_store(mut self, store: Arc<RwLock<DocumentStore>>) -> Self {
        self.document_store = Some(store);
        self
    }

    /// Set the chunking config for documents.
    pub fn chunking_config(mut self, config: ChunkingConfig) -> Self {
        self.chunking_config = config;
        self
    }

    /// Set the index path for semantic search persistence.
    pub fn index_path(mut self, path: PathBuf) -> Self {
        self.index_path = Some(path);
        self
    }

    /// Set the workspace root.
    pub fn workspace_root(mut self, path: PathBuf) -> Self {
        self.workspace_root = Some(path);
        self
    }

    /// Set the debounce duration in milliseconds.
    pub fn debounce_ms(mut self, ms: u64) -> Self {
        self.debounce_ms = ms;
        self
    }

    /// Build the UnifiedWatcher.
    pub fn build(self) -> Result<UnifiedWatcher, WatchError> {
        let broadcaster = self.broadcaster.ok_or_else(|| WatchError::InitFailed {
            reason: "Broadcaster is required".to_string(),
        })?;

        let facade = self.facade.ok_or_else(|| WatchError::InitFailed {
            reason: "Facade is required".to_string(),
        })?;

        let workspace_root = self
            .workspace_root
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let index_path = self
            .index_path
            .unwrap_or_else(|| workspace_root.join(".codanna/index"));
        let index_path = if index_path.is_absolute() {
            index_path
        } else {
            workspace_root.join(index_path)
        };
        let index_path = crate::documents::store::normalize_source_path(&index_path);

        // Keep the callback queue bounded, but never block notify's native event
        // thread. Blocking here can deadlock with watch registration on Linux.
        let (tx, rx) = mpsc::channel(256);
        let event_overflowed = Arc::new(AtomicBool::new(false));
        let overflow_flag = Arc::clone(&event_overflowed);

        // Create the notify watcher. Queue overflow is a recoverable condition:
        // the async drain performs a full truth reconciliation on the next tick.
        let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            enqueue_watch_event(&tx, &overflow_flag, res);
        })?;

        Ok(UnifiedWatcher {
            handlers: self.handlers,
            registry: PathRegistry::new(),
            debouncer: Debouncer::new(self.debounce_ms),
            event_rx: rx,
            event_overflowed,
            _watcher: watcher,
            broadcaster,
            facade,
            document_store: self.document_store,
            document_reconciliations: RwLock::new(DocumentReconciliations::new(self.debounce_ms)),
            pending_config: RwLock::new(None),
            chunking_config: self.chunking_config,
            index_path,
            workspace_root,
            handler_roots: Vec::new(),
            batch_sync_roots: Vec::new(),
            prepared: false,
        })
    }
}

impl Default for UnifiedWatcherBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Native notify callbacks must never block on the async consumer. When the
/// bounded queue is full, latch a reconciliation request and return immediately.
fn enqueue_watch_event(
    tx: &mpsc::Sender<notify::Result<Event>>,
    overflow_flag: &AtomicBool,
    event: notify::Result<Event>,
) {
    match tx.try_send(event) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            overflow_flag.store(true, Ordering::Release);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            // The watcher is shutting down; there is no consumer to recover.
        }
    }
}

/// Another serve process holds the Tantivy index writer for this
/// workspace: its watcher indexes the change and this process
/// converges via hot-reload. Tantivy surfaces the contention as a
/// lockfile-acquire failure in the storage error chain; that text is
/// the only marker crossing the boxed layers.
#[cfg(test)]
fn is_writer_lock_contention(e: &crate::IndexError) -> bool {
    e.to_string().contains("Failed to acquire Lockfile")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::watcher::handlers::CodeFileHandler;
    use notify::event::{AccessKind, AccessMode, ModifyKind, RenameMode};
    use std::path::Path;

    async fn watcher_over(dir: &Path, root: &Path) -> UnifiedWatcher {
        let mut settings = Settings {
            index_path: dir.join("index"),
            workspace_root: None,
            ..Default::default()
        };
        settings
            .add_indexed_path(root.to_path_buf())
            .expect("register indexed path");
        let facade = Arc::new(RwLock::new(IndexFacade::new(Arc::new(settings)).unwrap()));
        let handler = CodeFileHandler::new(Arc::clone(&facade), dir.to_path_buf());
        handler.init_cache().await;
        UnifiedWatcher::builder()
            .handler(handler)
            .broadcaster(Arc::new(NotificationBroadcaster::new(16)))
            .indexer(facade)
            .workspace_root(dir.to_path_buf())
            .build()
            .unwrap()
    }

    #[test]
    fn hardening_watcher_full_queue_marks_reconciliation_without_blocking() {
        let (tx, mut rx) = mpsc::channel(1);
        let overflow = AtomicBool::new(false);
        let event = || {
            Ok(Event {
                kind: EventKind::Any,
                paths: Vec::new(),
                attrs: Default::default(),
            })
        };

        enqueue_watch_event(&tx, &overflow, event());
        assert!(!overflow.load(Ordering::Acquire));

        // Queue is now full. The second send must return synchronously and mark
        // filesystem reconciliation instead of blocking the notify thread.
        enqueue_watch_event(&tx, &overflow, event());
        assert!(overflow.load(Ordering::Acquire));
        assert!(rx.try_recv().is_ok());
    }

    // A dir rename's from-side arrives as Modify(Name) on a path that no
    // longer exists, and no per-file events follow. A vanished path that
    // prefixes watched directories is a directory removal observation:
    // it must enter the removal wave, not fall to the unmatched trace.
    #[tokio::test]
    async fn vanished_watched_dir_records_a_removal_observation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        std::fs::write(root.join("pkg/a.py"), "def a():\n    pass\n").unwrap();
        let canonical_root = root.canonicalize().unwrap();
        let pkg = canonical_root.join("pkg");

        let mut watcher = watcher_over(dir.path(), &root).await;
        watcher.registry.add_watch_dir(pkg.clone());

        std::fs::remove_dir_all(&pkg).unwrap();
        watcher
            .handle_event(Event {
                kind: EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
                paths: vec![pkg],
                attrs: Default::default(),
            })
            .await;

        assert!(
            watcher.debouncer.has_pending_removals(),
            "a vanished watched directory must record a removal observation"
        );
    }

    // A dir rename's to-side arrives as Modify(Name) on a path that IS a
    // directory -- never as Create. Disk truth decides the route: an
    // existing directory under a handler root runs created-directory
    // catch-up regardless of event kind.
    #[tokio::test]
    async fn existing_dir_routes_to_catchup_regardless_of_event_kind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("pkg_renamed")).unwrap();
        std::fs::write(root.join("pkg_renamed/a.py"), "def a():\n    pass\n").unwrap();
        let canonical_root = root.canonicalize().unwrap();

        let mut watcher = watcher_over(dir.path(), &root).await;
        watcher.handler_roots = vec![canonical_root.clone()];

        watcher
            .handle_event(Event {
                kind: EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
                paths: vec![canonical_root.join("pkg_renamed")],
                attrs: Default::default(),
            })
            .await;

        assert!(
            watcher.debouncer.has_pending(),
            "an existing directory's files must enter the catch-up debounce on any event kind"
        );
    }

    // inotify emits Access(Open) for every directory read, including
    // the catch-up walk's own opens; routing those into the directory
    // branch livelocks (walk emits Open, Open triggers walk), which
    // starves the debounce drain and silences every notification.
    // Access observes state and never changes it: dropped before any
    // routing. FSEvents emits no Access events, so only Linux
    // exercises this.
    #[tokio::test]
    async fn access_events_route_nowhere() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        std::fs::write(root.join("pkg/a.py"), "def a():\n    pass\n").unwrap();
        let canonical_root = root.canonicalize().unwrap();

        let mut watcher = watcher_over(dir.path(), &root).await;
        watcher.handler_roots = vec![canonical_root.clone()];

        watcher
            .handle_event(Event {
                kind: EventKind::Access(AccessKind::Open(AccessMode::Any)),
                paths: vec![canonical_root.join("pkg")],
                attrs: Default::default(),
            })
            .await;

        assert!(
            !watcher.debouncer.has_pending(),
            "an Access event on a directory must not enter catch-up"
        );
        assert!(
            !watcher.debouncer.has_pending_removals(),
            "an Access event must not record a removal observation"
        );
    }

    #[tokio::test]
    async fn nested_ignore_policy_reconciles_exclusion_and_reinclusion() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let nested = root.join("packages/payments");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("ledger.rs"), "pub fn ledger() {}\n").unwrap();
        let mut watcher = watcher_over(dir.path(), &root).await;
        watcher.synchronize_roots(vec![root.clone()]).await.unwrap();
        watcher.prepare().await.unwrap();
        watcher.debouncer = Debouncer::new(0);
        let canonical_nested = nested.canonicalize().unwrap();
        assert!(watcher.registry.watch_dirs().contains(&canonical_nested));
        for name in [".gitignore", ".codannaignore"] {
            let policy = nested.join(name);
            for excluded in [true, false] {
                std::fs::write(&policy, if excluded { "ledger.rs\n" } else { "" }).unwrap();
                watcher
                    .handle_event(Event {
                        kind: EventKind::Modify(ModifyKind::Any),
                        paths: vec![policy.clone()],
                        attrs: Default::default(),
                    })
                    .await;
                let (removed, modified) = watcher
                    .debouncer
                    .take_settled_burst()
                    .expect("policy events must enter the whole-root reconciliation lane");
                watcher.process_change_wave(removed, modified).await;
                watcher.handle_index_reloaded().await;
                let count = crate::runtime::read(&watcher.facade, |facade| {
                    facade.find_symbols_by_name("ledger", None).len()
                })
                .await
                .unwrap();
                assert_eq!(
                    count,
                    usize::from(!excluded),
                    "policy={name}, excluded={excluded}"
                );
                assert!(
                    watcher.registry.watch_dirs().contains(&canonical_nested),
                    "empty code directories must retain their policy watch"
                );
            }
        }
    }

    #[tokio::test]
    async fn preparation_watches_policy_in_an_initially_excluded_code_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let nested = root.join("packages/payments");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(".codannaignore"), "*.rs\n").unwrap();
        std::fs::write(nested.join("ledger.rs"), "pub fn ledger() {}\n").unwrap();
        let mut watcher = watcher_over(dir.path(), &root).await;
        watcher.prepare().await.unwrap();
        assert!(
            watcher
                .registry
                .watch_dirs()
                .contains(&nested.canonicalize().unwrap()),
            "a nested policy is observable even when no source in that directory is indexed"
        );
    }

    #[test]
    fn writer_lock_contention_is_classified_from_the_error_chain() {
        let contended = crate::IndexError::General(
            "Pipeline error: Storage error: Tantivy error: \
             Failed to acquire Lockfile: LockBusy. \
             Some(\"Failed to acquire index lock.\")"
                .to_string(),
        );
        assert!(is_writer_lock_contention(&contended));

        let unrelated = crate::IndexError::General("Pipeline error: parse failed".to_string());
        assert!(!is_writer_lock_contention(&unrelated));
    }
}

#[cfg(test)]
#[path = "collection_reload_tests.rs"]
mod collection_reload_tests;

#[cfg(test)]
#[path = "document_batching_tests.rs"]
mod document_batching_tests;

#[cfg(test)]
mod document_collection_tests {
    use super::*;
    use crate::documents::{CollectionConfig, DocumentsConfig};
    use crate::vector::VectorDimension;
    use crate::watcher::handlers::DocumentFileHandler;

    pub(super) fn fixture(
        root: &Path,
        documents: &Path,
    ) -> (UnifiedWatcher, Arc<RwLock<DocumentStore>>) {
        fixture_with_embeddings(root, documents, None)
    }

    pub(super) fn fixture_with_embeddings(
        root: &Path,
        documents: &Path,
        generator: Option<Box<dyn crate::vector::EmbeddingGenerator>>,
    ) -> (UnifiedWatcher, Arc<RwLock<DocumentStore>>) {
        let mut settings = crate::Settings {
            index_path: root.join("code-index"),
            workspace_root: Some(root.to_path_buf()),
            ..Default::default()
        };
        settings.semantic_search.enabled = false;
        let facade = Arc::new(RwLock::new(IndexFacade::new(Arc::new(settings)).unwrap()));
        let mut store = DocumentStore::new(
            root.join("documents-index"),
            VectorDimension::new(2).unwrap(),
        )
        .unwrap();
        if let Some(generator) = generator {
            store = store.with_embeddings(generator).unwrap();
        }
        let store = Arc::new(RwLock::new(store));
        let mut config = DocumentsConfig {
            enabled: true,
            defaults: ChunkingConfig {
                min_chunk_chars: 1,
                max_chunk_chars: 256,
                overlap_chars: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        config.collections.insert(
            "docs".into(),
            CollectionConfig {
                paths: vec![documents.to_path_buf()],
                max_chunk_chars: Some(50),
                ..Default::default()
            },
        );
        let handler =
            DocumentFileHandler::new(store.clone(), root.to_path_buf()).with_config(&config);
        let watcher = UnifiedWatcher::builder()
            .indexer(facade)
            .document_store(store.clone())
            .workspace_root(root.to_path_buf())
            .broadcaster(Arc::new(NotificationBroadcaster::new(8)))
            .handler(handler)
            .debounce_ms(0)
            .build()
            .unwrap();
        (watcher, store)
    }

    #[tokio::test]
    async fn document_watcher_indexes_new_and_recreated_files_with_collection_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let docs = root.join("docs");
        let empty = docs.join("initially-empty");
        std::fs::create_dir_all(&empty).unwrap();
        let (mut watcher, store) = fixture(&root, &docs);
        watcher.prepare().await.unwrap();
        assert!(watcher.registry.watch_dirs().contains(&empty));
        let file = empty.join("guide.md");
        std::fs::write(&file, "a".repeat(200)).unwrap();
        assert!(watcher.handlers[0].matches(&file));
        let action = watcher.handlers[0].on_modify(&file).await.unwrap();
        watcher.execute_action(action, "document").await.unwrap();
        watcher.flush_document_reconciliations(Instant::now()).await;
        assert_eq!(
            store
                .read()
                .await
                .collection_stats("docs")
                .unwrap()
                .chunk_count,
            4
        );
        std::fs::remove_file(&file).unwrap();
        let action = watcher.handlers[0].on_delete(&file).await.unwrap();
        watcher.execute_action(action, "document").await.unwrap();
        watcher.flush_document_reconciliations(Instant::now()).await;
        assert_eq!(
            store
                .read()
                .await
                .collection_stats("docs")
                .unwrap()
                .chunk_count,
            0
        );
        std::fs::write(&file, "recreated").unwrap();
        assert!(watcher.handlers[0].matches(&file));
        let action = watcher.handlers[0].on_modify(&file).await.unwrap();
        watcher.execute_action(action, "document").await.unwrap();
        watcher.flush_document_reconciliations(Instant::now()).await;
        assert_eq!(
            store
                .read()
                .await
                .collection_stats("docs")
                .unwrap()
                .chunk_count,
            1
        );
    }

    #[tokio::test]
    async fn document_watcher_catches_moved_subtrees_and_nested_ignore_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let docs = root.join("docs");
        std::fs::create_dir(&docs).unwrap();
        let (mut watcher, store) = fixture(&root, &docs);
        watcher.prepare().await.unwrap();
        let nested = docs.join("moved-in");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("guide.md"), "alpha").unwrap();
        watcher
            .handle_event(
                Event::new(EventKind::Create(notify::event::CreateKind::Folder))
                    .add_path(nested.clone()),
            )
            .await;
        watcher.flush_document_reconciliations(Instant::now()).await;
        assert_eq!(
            store
                .read()
                .await
                .collection_stats("docs")
                .unwrap()
                .chunk_count,
            1
        );
        let policy = nested.join(".codannaignore");
        std::fs::write(&policy, "*.md\n").unwrap();
        assert!(watcher.handlers[0].matches(&policy));
        let action = watcher.handlers[0].on_modify(&policy).await.unwrap();
        watcher.execute_action(action, "document").await.unwrap();
        watcher.flush_document_reconciliations(Instant::now()).await;
        assert_eq!(
            store
                .read()
                .await
                .collection_stats("docs")
                .unwrap()
                .chunk_count,
            0
        );
        std::fs::remove_file(&policy).unwrap();
        let action = watcher.handlers[0].on_delete(&policy).await.unwrap();
        watcher.execute_action(action, "document").await.unwrap();
        watcher.flush_document_reconciliations(Instant::now()).await;
        assert_eq!(
            store
                .read()
                .await
                .collection_stats("docs")
                .unwrap()
                .chunk_count,
            1
        );
        std::fs::remove_dir_all(&nested).unwrap();
        watcher
            .handle_event(
                Event::new(EventKind::Remove(notify::event::RemoveKind::Folder))
                    .add_path(nested.clone()),
            )
            .await;
        watcher.flush_document_reconciliations(Instant::now()).await;
        assert_eq!(
            store
                .read()
                .await
                .collection_stats("docs")
                .unwrap()
                .chunk_count,
            0
        );
        assert!(
            watcher.registry.watch_dirs().contains(&root),
            "the configured root's parent must be watched before the root vanishes"
        );
        std::fs::remove_dir(&docs).unwrap();
        watcher
            .handle_event(
                Event::new(EventKind::Remove(notify::event::RemoveKind::Folder))
                    .add_path(docs.clone()),
            )
            .await;
        watcher.flush_document_reconciliations(Instant::now()).await;
        std::fs::create_dir(&docs).unwrap();
        std::fs::write(docs.join("restored.md"), "restored root").unwrap();
        watcher
            .handle_event(
                Event::new(EventKind::Create(notify::event::CreateKind::Folder))
                    .add_path(docs.clone()),
            )
            .await;
        watcher.flush_document_reconciliations(Instant::now()).await;
        assert_eq!(
            store
                .read()
                .await
                .collection_stats("docs")
                .unwrap()
                .chunk_count,
            1
        );
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::Notify;

    struct StartupHandler {
        root: PathBuf,
        entered: Arc<Notify>,
        release: Arc<Notify>,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }
    #[async_trait::async_trait]
    impl WatchHandler for StartupHandler {
        fn name(&self) -> &str {
            "startup-fixture"
        }
        fn matches(&self, _path: &Path) -> bool {
            false
        }
        async fn tracked_paths(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        async fn watch_roots(&self) -> Vec<PathBuf> {
            vec![self.root.clone()]
        }
        async fn on_modify(&self, _path: &Path) -> Result<WatchAction, WatchError> {
            Ok(WatchAction::None)
        }
        async fn on_delete(&self, _path: &Path) -> Result<WatchAction, WatchError> {
            Ok(WatchAction::None)
        }
        async fn refresh_paths(&self) -> Result<(), WatchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            self.release.notified().await;
            if self.fail {
                return Err(WatchError::InitFailed {
                    reason: "fixture refused initialization".into(),
                });
            }
            Ok(())
        }
    }

    fn fixture(dir: &Path, handler: StartupHandler) -> UnifiedWatcher {
        let settings = Arc::new(crate::Settings {
            workspace_root: Some(dir.to_path_buf()),
            index_path: dir.join("index"),
            ..crate::Settings::default()
        });
        let facade = Arc::new(RwLock::new(IndexFacade::new(settings).unwrap()));
        UnifiedWatcher::builder()
            .indexer(facade)
            .workspace_root(dir.to_path_buf())
            .broadcaster(Arc::new(NotificationBroadcaster::new(8)))
            .handler(handler)
            .build()
            .unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hardening_final_watcher_preparation_waits_for_handlers_and_registers_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("src");
        std::fs::create_dir(&root).unwrap();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut watcher = fixture(
            dir.path(),
            StartupHandler {
                root: root.clone(),
                entered: entered.clone(),
                release: release.clone(),
                calls: calls.clone(),
                fail: false,
            },
        );
        let task = tokio::spawn(async move {
            watcher.prepare().await.unwrap();
            watcher
        });
        entered.notified().await;
        assert!(
            !task.is_finished(),
            "readiness cannot precede handler completion"
        );
        release.notify_one();
        let mut watcher = task.await.unwrap();
        assert!(watcher.prepared);
        assert!(watcher.registry.watch_dirs().contains(&root));
        assert_eq!(watcher.handler_roots, vec![root]);
        // A second prepare must not await or refresh again.
        watcher.prepare().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hardening_final_watcher_failed_handler_never_reports_ready() {
        let dir = tempfile::tempdir().unwrap();
        let release = Arc::new(Notify::new());
        release.notify_one();
        let mut watcher = fixture(
            dir.path(),
            StartupHandler {
                root: dir.path().to_path_buf(),
                entered: Arc::new(Notify::new()),
                release,
                calls: Arc::new(AtomicUsize::new(0)),
                fail: true,
            },
        );
        let error = watcher.prepare().await.unwrap_err();
        assert!(error.to_string().contains("fixture refused initialization"));
        assert!(!watcher.prepared);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hardening_final_watcher_failed_native_registration_never_reports_ready() {
        let dir = tempfile::tempdir().unwrap();
        let release = Arc::new(Notify::new());
        release.notify_one();
        let mut watcher = fixture(
            dir.path(),
            StartupHandler {
                root: dir.path().join("missing-root"),
                entered: Arc::new(Notify::new()),
                release,
                calls: Arc::new(AtomicUsize::new(0)),
                fail: false,
            },
        );
        assert!(watcher.prepare().await.is_err());
        assert!(!watcher.prepared);
    }
}
