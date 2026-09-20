//! Validated, transactional document configuration publication.

use super::*;
use crate::documents::DocumentsConfig;
use crate::watcher::handlers::DocumentFileHandler;

pub(super) struct PendingConfigReload {
    settings: Box<crate::Settings>,
    failures: u32,
    retry_at: Option<Instant>,
}

impl PendingConfigReload {
    pub(super) fn new(settings: Box<crate::Settings>) -> Self {
        Self {
            settings,
            failures: 0,
            retry_at: None,
        }
    }

    fn retry(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let delay_ms = (250_u64 << self.failures.saturating_sub(1).min(7)).min(30_000);
        self.retry_at = Some(now + Duration::from_millis(delay_ms));
    }
}

struct AcceptedReload {
    settings: crate::Settings,
    previous: DocumentsConfig,
    added_code: Vec<PathBuf>,
    removed_code: bool,
    workspace_boundary: Option<PathBuf>,
}

fn config_error(reason: impl Into<String>) -> WatchError {
    WatchError::ConfigError {
        reason: reason.into(),
    }
}

/// Resolve missing children through a real ancestor, while refusing dangling
/// symlinks whose eventual target would change the admitted ownership boundary.
fn resolve_reload_path(path: &Path, workspace: &Path) -> Result<PathBuf, WatchError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    for ancestor in absolute.ancestors() {
        if let Ok(canonical) = ancestor.canonicalize() {
            let suffix = absolute
                .strip_prefix(ancestor)
                .map_err(|error| config_error(error.to_string()))?;
            let mut resolved = canonical;
            for component in suffix.components() {
                match component {
                    std::path::Component::ParentDir => {
                        resolved.pop();
                    }
                    std::path::Component::CurDir => {}
                    component => resolved.push(component.as_os_str()),
                }
            }
            return Ok(resolved);
        }
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(config_error(format!(
                    "Cannot reload unresolved symlink {}",
                    ancestor.display()
                )));
            }
            Ok(_) => {
                return Err(config_error(format!(
                    "Cannot resolve configured path {}",
                    ancestor.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(config_error(format!(
                    "Cannot inspect configured path {}: {error}",
                    ancestor.display()
                )));
            }
        }
    }
    Err(config_error(format!(
        "No existing ancestor for configured path {}",
        absolute.display()
    )))
}

fn normalize_documents(
    config: &mut DocumentsConfig,
    workspace: &Path,
    boundary: Option<&Path>,
) -> Result<(), WatchError> {
    config.defaults.validate().map_err(config_error)?;
    for (name, collection) in &mut config.collections {
        collection
            .effective_chunking(&config.defaults)
            .validate()
            .map_err(|error| {
                config_error(format!("Invalid chunking for collection '{name}': {error}"))
            })?;
        for pattern in collection.effective_patterns() {
            glob::Pattern::new(&pattern).map_err(|error| {
                config_error(format!("Invalid glob in collection '{name}': {error}"))
            })?;
        }
        for path in &mut collection.paths {
            if let Some(root) = boundary {
                IndexFacade::contained_source(root, path)
                    .map_err(|error| config_error(error.to_string()))?;
            }
            *path = resolve_reload_path(path, workspace)?;
            if let Some(root) = boundary {
                IndexFacade::contained_source(root, path)
                    .map_err(|error| config_error(error.to_string()))?;
            }
        }
        collection.paths.sort();
        collection.paths.dedup();
    }
    Ok(())
}

impl UnifiedWatcher {
    async fn validate_config_reload(
        &self,
        settings: &crate::Settings,
    ) -> Result<AcceptedReload, WatchError> {
        let current = self.facade.read().await;
        let previous_settings = current.settings().clone();
        let workspace_boundary = current.network_workspace.clone();
        drop(current);
        let mut settings = settings.clone();
        let root = settings
            .workspace_root
            .as_deref()
            .unwrap_or(&self.workspace_root);
        if resolve_reload_path(root, &self.workspace_root)?
            != resolve_reload_path(&self.workspace_root, &self.workspace_root)?
        {
            return Err(config_error(
                "workspace_root cannot change during live reload; restart with the intended workspace",
            ));
        }
        let index_path = resolve_reload_path(&settings.index_path, &self.workspace_root)?;
        if index_path != self.index_path {
            return Err(config_error(
                "index_path cannot change during live reload; restart with the intended index directory",
            ));
        }
        if serde_json::to_value(&settings.semantic_search)
            .map_err(|error| config_error(error.to_string()))?
            != serde_json::to_value(&previous_settings.semantic_search)
                .map_err(|error| config_error(error.to_string()))?
        {
            return Err(config_error(
                "semantic_search settings cannot change during live reload; restart and follow the embedding model migration instructions",
            ));
        }
        settings.index_path = index_path;
        normalize_documents(
            &mut settings.documents,
            &self.workspace_root,
            workspace_boundary.as_deref(),
        )?;
        if settings.documents.enabled && self.document_store.is_none() {
            return Err(config_error(
                "Cannot enable documents in a server started without a document store; run 'codanna documents index' and restart the server",
            ));
        }
        let previous = self
            .handlers
            .iter()
            .find_map(|handler| handler.document_config())
            .unwrap_or_else(|| previous_settings.documents.clone());
        for path in &mut settings.indexed_paths_cache {
            if let Some(root) = &workspace_boundary {
                IndexFacade::contained_source(root, path)
                    .map_err(|error| config_error(error.to_string()))?;
            }
            *path = resolve_reload_path(path, &self.workspace_root)?;
        }
        let old: HashSet<_> = previous_settings
            .indexed_paths_cache
            .iter()
            .cloned()
            .collect();
        let new: HashSet<_> = settings.indexed_paths_cache.iter().cloned().collect();
        let added_code = new.difference(&old).cloned().collect();
        let removed_code = old.difference(&new).next().is_some();
        Ok(AcceptedReload {
            settings,
            previous,
            added_code,
            removed_code,
            workspace_boundary,
        })
    }

    pub(super) async fn flush_config_reload(&mut self, now: Instant) {
        let mut pending = {
            let mut slot = self.pending_config.write().await;
            if slot
                .as_ref()
                .is_none_or(|proposal| proposal.retry_at.is_some_and(|deadline| deadline > now))
            {
                return;
            }
            slot.take().expect("ready configuration exists")
        };
        let accepted = match self.validate_config_reload(&pending.settings).await {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::error!(
                    "[config] reload rejected; retaining running settings and documents: {error}"
                );
                return;
            }
        };
        match self.apply_config_reload(accepted).await {
            Ok(()) => {}
            Err(error) => {
                tracing::error!(
                    "[config] reload failed; running settings unchanged, publication recovery or retry pending: {error}"
                );
                pending.retry(now.max(Instant::now()));
                *self.pending_config.write().await = Some(pending);
            }
        }
    }

    async fn apply_config_reload(&mut self, accepted: AcceptedReload) -> Result<(), WatchError> {
        let next = &accepted.settings.documents;
        let mut changed: Vec<_> = if next.enabled {
            let mut changed: Vec<_> = next
                .collections
                .iter()
                .filter(|(name, config)| {
                    !accepted.previous.enabled
                        || accepted.previous.collections.get(*name).is_none_or(|old| {
                            old != *config
                                || old.effective_chunking(&accepted.previous.defaults)
                                    != config.effective_chunking(&next.defaults)
                        })
                })
                .map(|(name, config)| (name.clone(), config.clone()))
                .collect();
            changed.sort_by(|a, b| a.0.cmp(&b.0));
            changed
        } else {
            Vec::new()
        };
        let mut removed: Vec<_> = accepted
            .previous
            .collections
            .keys()
            .filter(|name| !next.enabled || !next.collections.contains_key(*name))
            .cloned()
            .collect();
        if !next.enabled {
            if let Some(store) = &self.document_store {
                // A disabled attached store remains shared with query snapshots;
                // its new generation must contain no searchable documents.
                removed.extend(store.read().await.list_collections());
            }
        }
        removed.sort();
        removed.dedup();
        let documents_changed = next != &accepted.previous;
        let candidate = self.document_store.as_ref().map(|store| {
            DocumentFileHandler::new(Arc::clone(store), self.workspace_root.clone())
                .with_config(next)
        });
        let mut proposed_watches = if documents_changed {
            match &candidate {
                Some(handler) => handler.watch_roots().await,
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        proposed_watches.retain(|path| {
            !path.starts_with(&self.index_path) && !self.registry.watch_dirs().contains(path)
        });
        if let Err(error) = self.watch_directories(&proposed_watches, true) {
            self.remove_native_watches(&proposed_watches);
            return Err(error);
        }
        let store = self.document_store.clone();
        let settings = accepted.settings.clone();
        let boundary = accepted.workspace_boundary;
        let changed_names: HashSet<_> = changed.iter().map(|(name, _)| name.clone()).collect();
        let result = crate::runtime::mutate(&self.facade, move |facade| {
            let mut recovered = false;
            if let Some(store) = store {
                let mut store = store.blocking_write();
                recovered = store
                    .with_committed_generation(boundary.as_deref(), |store, recovered| {
                        if recovered {
                            // A recovered publication may belong to an earlier proposal
                            // or another writer. Reconcile the complete latest policy,
                            // even when it is unchanged from the old live settings.
                            changed = if settings.documents.enabled {
                                settings
                                    .documents
                                    .collections
                                    .iter()
                                    .map(|(name, config)| (name.clone(), config.clone()))
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            changed.sort_by(|a, b| a.0.cmp(&b.0));
                            removed = store
                                .list_collections()
                                .into_iter()
                                .filter(|name| {
                                    !settings.documents.enabled
                                        || !settings.documents.collections.contains_key(name)
                                })
                                .collect();
                        }
                        if !changed.is_empty() || !removed.is_empty() {
                            if let Some(warning) = store.reconfigure_collections(
                                &removed,
                                &changed,
                                &settings.documents.defaults,
                            )? {
                                tracing::warn!("[config] {warning}");
                            }
                        }
                        Ok(recovered)
                    })
                    .map_err(|error| config_error(error.to_string()))?;
            }
            facade.reload_watched_settings(settings.indexed_paths_cache, settings.documents);
            Ok::<_, WatchError>(recovered)
        })
        .await
        .map_err(|error| config_error(error.to_string()))
        .and_then(std::convert::identity);
        let recovered = match result {
            Ok(recovered) => recovered,
            Err(error) => {
                self.remove_native_watches(&proposed_watches);
                return Err(error);
            }
        };
        {
            let mut queue = self.document_reconciliations.write().await;
            queue.pending.retain(|name, pending| {
                if recovered || !next.enabled || changed_names.contains(name) {
                    return false;
                }
                let Some(config) = next.collections.get(name) else {
                    return false;
                };
                pending.config = config.clone();
                pending.defaults = next.defaults.clone();
                true
            });
            queue.active = Some(next.clone());
        }
        self.chunking_config = next.defaults.clone();
        if documents_changed {
            if let Some(handler) = candidate {
                self.handlers.retain(|handler| handler.name() != "document");
                self.handlers.push(Box::new(handler));
            }
        }
        let old_dirs = self.registry.watch_dirs().clone();
        self.handle_index_reloaded().await;
        let obsolete: Vec<_> = old_dirs
            .difference(self.registry.watch_dirs())
            .cloned()
            .collect();
        self.remove_native_watches(&obsolete);
        if !accepted.added_code.is_empty() {
            // Code directory catch-up keeps its established independent commit
            // semantics. Document publication has already succeeded here.
            if let Err(error) = self.synchronize_roots(accepted.added_code).await {
                tracing::error!("[config] accepted code roots need catch-up: {error}");
                self.event_overflowed.store(true, Ordering::Release);
            }
        }
        if accepted.removed_code {
            tracing::info!("Run 'codanna clean' to remove symbols from removed directories");
        }
        self.broadcaster.send(FileChangeEvent::IndexReloaded);
        Ok(())
    }

    fn remove_native_watches(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        // FSEvents stops/restarts its stream for each ordinary unwatch. A single
        // batch keeps large collection removals and failed applies bounded.
        let mut changes = self._watcher.paths_mut();
        for path in paths {
            if let Err(error) = changes.remove(path) {
                tracing::debug!("[config] watch cleanup for {}: {error}", path.display());
            }
        }
        if let Err(error) = changes.commit() {
            tracing::error!("[config] native watch cleanup incomplete: {error}");
            self.event_overflowed.store(true, Ordering::Release);
        }
    }
}
