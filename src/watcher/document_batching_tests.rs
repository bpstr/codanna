//! Local, clock-driven watcher controls. No model downloads or native event sleeps.

use super::document_collection_tests::{fixture, fixture_with_embeddings};
use super::*;
use crate::documents::{DocumentsConfig, SearchQuery};
use crate::vector::{EmbeddingGenerator, VectorDimension, VectorError};
use crate::watcher::handlers::DocumentFileHandler;
use notify::event::{CreateKind, ModifyKind, RemoveKind};
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

fn modified(path: &Path) -> Event {
    Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path.to_path_buf())
}

fn removed(path: &Path) -> Event {
    Event::new(EventKind::Remove(RemoveKind::Any)).add_path(path.to_path_buf())
}

fn created(path: &Path) -> Event {
    Event::new(EventKind::Create(CreateKind::Any)).add_path(path.to_path_buf())
}

#[tokio::test]
async fn document_burst_reduces_measured_collection_scans_and_file_checks() {
    const FILES: usize = 64;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    std::fs::create_dir(&docs).unwrap();
    let paths: Vec<_> = (0..FILES)
        .map(|index| {
            let path = docs.join(format!("guide-{index:04}.md"));
            std::fs::write(&path, format!("local fixture {index:04}")).unwrap();
            path
        })
        .collect();
    let config = CollectionConfig {
        paths: vec![docs.clone()],
        ..Default::default()
    };
    let chunks = ChunkingConfig {
        min_chunk_chars: 1,
        max_chunk_chars: 50,
        overlap_chars: 0,
        ..Default::default()
    };
    let mut before =
        DocumentStore::new(root.join("before-index"), VectorDimension::new(2).unwrap()).unwrap();
    let mut before_scans = 0;
    let mut before_checks = 0;
    let before_started = Instant::now();
    // Execute the prior one-collection-scan-per-event behavior against the same
    // settled fixture. Counts come from actual store calls and returned stats.
    for _ in &paths {
        let stats = before
            .index_collection_with_progress("docs", &config, &chunks, |progress| {
                if matches!(
                    progress,
                    crate::documents::IndexProgress::Phase {
                        name: "discovering files"
                    }
                ) {
                    before_scans += 1;
                }
            })
            .unwrap();
        before_checks += stats.files_processed + stats.files_skipped;
    }
    let before_elapsed = before_started.elapsed();

    let (mut watcher, store) = fixture(&root, &docs);
    let now = Instant::now();
    for path in &paths {
        watcher.handle_event_at(modified(path), now).await;
    }
    assert_eq!(
        watcher.document_reconciliations.read().await.pending.len(),
        1
    );
    assert!(!watcher.debouncer.has_pending());
    let after_started = Instant::now();
    let after = watcher.dispatch_ready_changes(now).await;
    let after_elapsed = after_started.elapsed();
    assert_eq!(before_scans, FILES);
    assert_eq!(before_checks, FILES * FILES);
    assert_eq!(after.collection_scans, 1);
    assert_eq!(after.files_checked, FILES);
    assert_eq!(after.files_processed, FILES);
    assert_eq!(
        store
            .read()
            .await
            .collection_stats("docs")
            .unwrap()
            .chunk_count,
        FILES
    );
    eprintln!(
        "document burst work: before scans={before_scans}, file_checks={before_checks}, elapsed_us={}; after scans={}, file_checks={}, elapsed_us={}",
        before_elapsed.as_micros(),
        after.collection_scans,
        after.files_checked,
        after_elapsed.as_micros(),
    );
}

#[tokio::test]
async fn high_cardinality_document_events_keep_only_configured_collections_pending() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    let more = root.join("more");
    std::fs::create_dir(&docs).unwrap();
    std::fs::create_dir(&more).unwrap();
    let (mut watcher, store) = fixture(&root, &docs);
    let mut config = DocumentsConfig {
        enabled: true,
        ..Default::default()
    };
    config.collections.insert(
        "more".into(),
        CollectionConfig {
            paths: vec![more.clone()],
            ..Default::default()
        },
    );
    watcher.handlers.push(Box::new(
        DocumentFileHandler::new(store, root).with_config(&config),
    ));
    let now = Instant::now();
    for index in 0..4_096 {
        let parent = if index % 2 == 0 { &docs } else { &more };
        watcher
            .handle_event_at(removed(&parent.join(format!("old-{index}.md"))), now)
            .await;
    }
    assert_eq!(
        watcher.document_reconciliations.read().await.pending.len(),
        2
    );
    assert!(
        !watcher.debouncer.has_pending(),
        "document-only paths must not grow the shared path queue"
    );
    let stats = watcher.dispatch_ready_changes(now).await;
    assert_eq!(stats.collection_scans, 2);
    assert_eq!(stats.files_checked, 0);
    assert!(
        watcher
            .document_reconciliations
            .read()
            .await
            .pending
            .is_empty()
    );
}

#[tokio::test]
async fn continuous_document_events_respect_quiet_window_and_maximum_delay() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    std::fs::create_dir(&docs).unwrap();
    let path = docs.join("guide.md");
    std::fs::write(&path, "event stream").unwrap();
    let (mut watcher, _) = fixture(&root, &docs);
    watcher.document_reconciliations = RwLock::new(DocumentReconciliations::new(500));
    let started = Instant::now();
    for tick in 0..20 {
        let now = started + Duration::from_millis(tick * 100);
        watcher.handle_event_at(modified(&path), now).await;
        assert_eq!(
            watcher.dispatch_ready_changes(now).await.collection_scans,
            0
        );
    }
    let deadline = started + MAX_DOCUMENT_BATCH_DELAY;
    watcher.handle_event_at(modified(&path), deadline).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(deadline)
            .await
            .collection_scans,
        1
    );
    assert!(
        watcher
            .document_reconciliations
            .read()
            .await
            .pending
            .is_empty()
    );
}

#[tokio::test]
async fn root_recreation_ignore_and_source_changes_share_one_truth_scan() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    std::fs::create_dir(&docs).unwrap();
    let stale = docs.join("stale.md");
    std::fs::write(&stale, "obsolete").unwrap();
    let (mut watcher, store) = fixture(&root, &docs);
    watcher.prepare().await.unwrap();
    watcher.handle_event(modified(&stale)).await;
    watcher.dispatch_ready_changes(Instant::now()).await;
    assert!(watcher.registry.watch_dirs().contains(&root));

    std::fs::remove_dir_all(&docs).unwrap();
    watcher.handle_event(removed(&docs)).await;
    let nested = docs.join("restored");
    std::fs::create_dir_all(&nested).unwrap();
    let keep = nested.join("keep.md");
    let excluded = nested.join("excluded.md");
    let policy = nested.join(".codannaignore");
    std::fs::write(&keep, "current evidence").unwrap();
    std::fs::write(&excluded, "temporarily hidden").unwrap();
    std::fs::write(&policy, "excluded.md\n").unwrap();
    for path in [&docs, &nested, &keep, &excluded, &policy] {
        watcher.handle_event(created(path)).await;
    }
    let stats = watcher.dispatch_ready_changes(Instant::now()).await;
    assert_eq!(stats.collection_scans, 1);
    assert_eq!(stats.files_checked, 1);
    assert!(watcher.registry.watch_dirs().contains(&root));
    assert!(watcher.registry.watch_dirs().contains(&nested));
    assert_eq!(store.read().await.get_indexed_paths(), vec![keep.clone()]);

    std::fs::remove_file(&policy).unwrap();
    std::fs::remove_file(&keep).unwrap();
    watcher.handle_event(removed(&policy)).await;
    watcher.handle_event(removed(&keep)).await;
    let stats = watcher.dispatch_ready_changes(Instant::now()).await;
    assert_eq!(stats.collection_scans, 1);
    assert_eq!(store.read().await.get_indexed_paths(), vec![excluded]);
}

#[tokio::test]
async fn nested_root_ancestor_ignore_policy_is_observed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("area/docs");
    std::fs::create_dir_all(&docs).unwrap();
    let source = docs.join("guide.md");
    std::fs::write(&source, "parent policy").unwrap();
    let (mut watcher, store) = fixture(&root, &docs);
    watcher.prepare().await.unwrap();
    assert!(watcher.registry.watch_dirs().contains(&root));
    watcher.handle_event(created(&source)).await;
    watcher.dispatch_ready_changes(Instant::now()).await;
    let policy = root.join(".codannaignore");
    std::fs::write(&policy, "*.md\n").unwrap();
    watcher.handle_event(modified(&policy)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert!(store.read().await.get_indexed_paths().is_empty());
    std::fs::remove_file(&policy).unwrap();
    watcher.handle_event(removed(&policy)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert_eq!(store.read().await.get_indexed_paths(), vec![source]);
}

struct FailingModel {
    calls: Arc<AtomicUsize>,
}

impl EmbeddingGenerator for FailingModel {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(VectorError::EmbeddingFailed("local fixture failure".into()));
        }
        Ok(vec![vec![1.0, 0.0]; texts.len()])
    }
    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }
}

#[tokio::test]
async fn failed_collection_retries_on_idle_dispatch_after_backoff() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    std::fs::create_dir(&docs).unwrap();
    let path = docs.join("guide.md");
    std::fs::write(&path, "retry evidence").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (mut watcher, store) = fixture_with_embeddings(
        &root,
        &docs,
        Some(Box::new(FailingModel {
            calls: calls.clone(),
        })),
    );
    watcher.handle_event(created(&path)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(store.read().await.get_indexed_paths().is_empty());
    let deadline = watcher.document_reconciliations.read().await.pending["docs"]
        .retry_at
        .unwrap();
    assert_eq!(
        watcher
            .dispatch_ready_changes(deadline - Duration::from_millis(1))
            .await
            .collection_scans,
        0
    );
    assert_eq!(
        watcher
            .dispatch_ready_changes(deadline)
            .await
            .collection_scans,
        1
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(store.read().await.get_indexed_paths(), vec![path]);
    assert!(
        watcher
            .document_reconciliations
            .read()
            .await
            .pending
            .is_empty()
    );
}

struct PausingModel {
    started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl EmbeddingGenerator for PausingModel {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        if let Some(started) = self.started.lock().unwrap().take() {
            started.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
        }
        Ok(vec![vec![1.0, 0.0]; texts.len()])
    }
    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }
}

#[tokio::test]
async fn events_and_overflow_during_reconciliation_survive_for_next_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    std::fs::create_dir(&docs).unwrap();
    let path = docs.join("guide.md");
    std::fs::write(&path, "before queued event").unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (mut watcher, store) = fixture_with_embeddings(
        &root,
        &docs,
        Some(Box::new(PausingModel {
            started: Mutex::new(Some(started_tx)),
            release: Mutex::new(release_rx),
        })),
    );
    // Replace native delivery with a controlled bounded channel. The watcher
    // itself has no installed native watches, so scheduling is deterministic.
    let (event_tx, event_rx) = mpsc::channel(1);
    watcher.event_rx = event_rx;
    let overflow = watcher.event_overflowed.clone();
    watcher.handle_event(created(&path)).await;
    let running = tokio::spawn(async move {
        let stats = watcher.dispatch_ready_changes(Instant::now()).await;
        (watcher, stats)
    });
    started_rx.await.unwrap();
    std::fs::write(&path, "after queued event").unwrap();
    let additional = docs.join("additional.md");
    std::fs::write(&additional, "overflow evidence").unwrap();
    enqueue_watch_event(&event_tx, &overflow, Ok(modified(&path)));
    enqueue_watch_event(&event_tx, &overflow, Ok(created(&additional)));
    release_tx.send(()).unwrap();
    let (mut watcher, first) = running.await.unwrap();
    assert_eq!(first.collection_scans, 1);
    assert!(overflow.load(Ordering::Acquire));
    assert_eq!(watcher.event_rx.len(), 1);
    let prior = store
        .write()
        .await
        .search(SearchQuery {
            text: "evidence".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(prior.len(), 1);
    assert!(prior[0].content_preview.contains("before queued event"));
    let queued = watcher.event_rx.try_recv().unwrap().unwrap();
    watcher.handle_event(queued).await;
    let second = watcher.dispatch_ready_changes(Instant::now()).await;
    assert_eq!(second.collection_scans, 1);
    assert_eq!(second.files_processed, 2);
    let paths = store.read().await.get_indexed_paths();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&path));
    assert!(paths.contains(&additional));
    assert!(!overflow.load(Ordering::Acquire));
}

struct OverlappingHandler {
    path: PathBuf,
    modifications: Arc<AtomicUsize>,
    deletions: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl WatchHandler for OverlappingHandler {
    fn name(&self) -> &str {
        "overlapping-source"
    }
    fn matches(&self, path: &Path) -> bool {
        path == self.path
    }
    async fn tracked_paths(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
    async fn on_modify(&self, _path: &Path) -> Result<WatchAction, WatchError> {
        self.modifications.fetch_add(1, Ordering::SeqCst);
        Ok(WatchAction::None)
    }
    async fn on_delete(&self, _path: &Path) -> Result<WatchAction, WatchError> {
        self.deletions.fetch_add(1, Ordering::SeqCst);
        Ok(WatchAction::None)
    }
}

#[tokio::test]
async fn overlapping_source_handler_retains_modification_and_deletion_events() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    std::fs::create_dir(&docs).unwrap();
    let path = docs.join("guide.md");
    std::fs::write(&path, "shared path").unwrap();
    let (mut watcher, store) = fixture(&root, &docs);
    let modifications = Arc::new(AtomicUsize::new(0));
    let deletions = Arc::new(AtomicUsize::new(0));
    watcher.handlers.push(Box::new(OverlappingHandler {
        path: path.clone(),
        modifications: modifications.clone(),
        deletions: deletions.clone(),
    }));
    watcher.handle_event(created(&path)).await;
    assert!(watcher.debouncer.has_pending());
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert_eq!(modifications.load(Ordering::SeqCst), 1);
    std::fs::remove_file(&path).unwrap();
    watcher.handle_event(removed(&path)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert_eq!(deletions.load(Ordering::SeqCst), 1);
    assert!(store.read().await.get_indexed_paths().is_empty());
}

#[tokio::test]
async fn pending_retry_does_not_delay_cooperative_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    std::fs::create_dir(&docs).unwrap();
    let path = docs.join("guide.md");
    std::fs::write(&path, "pending retry").unwrap();
    let (mut watcher, _) = fixture(&root, &docs);
    watcher.handle_event(created(&path)).await;
    let mut queue = watcher.document_reconciliations.write().await;
    queue.pending.get_mut("docs").unwrap().retry_at =
        Some(Instant::now() + Duration::from_secs(30));
    drop(queue);
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(2), watcher.watch_until(stop))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn code_and_document_collections_reconcile_the_same_source_independently() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("shared");
    std::fs::create_dir(&docs).unwrap();
    let path = docs.join("guide.rs");
    std::fs::write(&path, "pub fn shared_evidence() {}\n").unwrap();
    let sibling = root.join("shared-sibling");
    std::fs::create_dir(&sibling).unwrap();
    let unrelated = sibling.join("untouched.rs");
    std::fs::write(&unrelated, "pub fn unrelated_evidence() {}\n").unwrap();
    let (mut watcher, store) = fixture(&root, &docs);
    let code_roots = vec![docs.clone(), sibling];
    crate::runtime::mutate(&watcher.facade, move |facade| {
        facade.reload_indexed_paths(code_roots)
    })
    .await
    .unwrap();
    let mut config = DocumentsConfig {
        enabled: true,
        ..Default::default()
    };
    config.collections.insert(
        "docs".into(),
        CollectionConfig {
            paths: vec![docs.clone()],
            patterns: vec!["**/*.rs".into()],
            ..Default::default()
        },
    );
    watcher.handlers[0] =
        Box::new(DocumentFileHandler::new(store.clone(), root.clone()).with_config(&config));
    watcher
        .handlers
        .push(Box::new(crate::watcher::handlers::CodeFileHandler::new(
            watcher.facade.clone(),
            root,
        )));
    watcher.prepare().await.unwrap();
    watcher
        .execute_action(
            WatchAction::ReindexCode {
                path: unrelated,
                created: true,
            },
            "code",
        )
        .await
        .unwrap();
    watcher.handle_event(created(&path)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert_eq!(store.read().await.get_indexed_paths(), vec![path]);
    let symbols = crate::runtime::read(&watcher.facade, |facade| {
        facade.find_symbols_by_name("shared_evidence", None).len()
    })
    .await
    .unwrap();
    assert_eq!(symbols, 1);
    std::fs::remove_dir_all(&docs).unwrap();
    watcher.handle_event(removed(&docs)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert!(store.read().await.get_indexed_paths().is_empty());
    let symbols = crate::runtime::read(&watcher.facade, |facade| {
        facade.find_symbols_by_name("shared_evidence", None).len()
    })
    .await
    .unwrap();
    assert_eq!(symbols, 0);
    let unrelated_symbols = crate::runtime::read(&watcher.facade, |facade| {
        facade
            .find_symbols_by_name("unrelated_evidence", None)
            .len()
    })
    .await
    .unwrap();
    assert_eq!(
        unrelated_symbols, 1,
        "observed removal must not clean sibling roots"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn observed_root_removal_does_not_treat_a_dangling_symlink_as_absence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("shared");
    std::fs::create_dir(&docs).unwrap();
    let source = docs.join("guide.rs");
    std::fs::write(&source, "pub fn retained_evidence() {}\n").unwrap();
    let (watcher, _) = fixture(&root, &docs);
    crate::runtime::mutate(&watcher.facade, move |facade| facade.index_file(&source))
        .await
        .unwrap()
        .unwrap();
    std::fs::remove_dir_all(&docs).unwrap();
    std::os::unix::fs::symlink(root.join("missing-target"), &docs).unwrap();
    let observed = docs.clone();
    assert!(
        !crate::runtime::mutate(&watcher.facade, move |facade| {
            facade.remove_observed_directory(&observed)
        })
        .await
        .unwrap()
        .unwrap()
    );
    let retained = crate::runtime::read(&watcher.facade, |facade| {
        facade.find_symbols_by_name("retained_evidence", None).len()
    })
    .await
    .unwrap();
    assert_eq!(retained, 1);
    std::fs::remove_file(&docs).unwrap();
    assert!(
        crate::runtime::mutate(&watcher.facade, move |facade| {
            facade.remove_observed_directory(&docs)
        })
        .await
        .unwrap()
        .unwrap()
    );
    let retained = crate::runtime::read(&watcher.facade, |facade| {
        facade.find_symbols_by_name("retained_evidence", None).len()
    })
    .await
    .unwrap();
    assert_eq!(retained, 0);
}

#[tokio::test]
async fn document_watch_roots_do_not_expand_code_indexing_on_reload() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("docs");
    let code = root.join("src");
    std::fs::create_dir(&docs).unwrap();
    std::fs::create_dir(&code).unwrap();
    let (mut watcher, store) = fixture(&root, &docs);
    crate::runtime::mutate(&watcher.facade, move |facade| {
        facade.reload_indexed_paths(vec![code])
    })
    .await
    .unwrap();
    watcher
        .handlers
        .push(Box::new(crate::watcher::handlers::CodeFileHandler::new(
            watcher.facade.clone(),
            root,
        )));
    watcher.prepare().await.unwrap();
    let nested = docs.join("new-subtree");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(
        nested.join("stray.rs"),
        "pub fn outside_code_inventory() {}\n",
    )
    .unwrap();
    let source = nested.join("guide.md");
    std::fs::write(&source, "document evidence").unwrap();
    // A notification can arrive before the native created-directory event.
    // Its root refresh must not pass document roots to code indexing.
    watcher.handle_index_reloaded().await;
    watcher.handle_event(created(&nested)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert_eq!(store.read().await.get_indexed_paths(), vec![source]);
    let symbols = crate::runtime::read(&watcher.facade, |facade| {
        facade
            .find_symbols_by_name("outside_code_inventory", None)
            .len()
    })
    .await
    .unwrap();
    assert_eq!(symbols, 0);
}

#[tokio::test]
async fn relative_code_actions_use_workspace_paths_and_keep_notification_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let docs = root.join("shared");
    std::fs::create_dir(&docs).unwrap();
    let path = docs.join("guide.rs");
    let relative = PathBuf::from("shared/guide.rs");
    std::fs::write(&path, "pub fn relative_evidence() {}\n").unwrap();
    let (watcher, _) = fixture(&root, &docs);
    crate::runtime::mutate(&watcher.facade, move |facade| {
        facade.reload_indexed_paths(vec![docs])
    })
    .await
    .unwrap();
    let mut notifications = watcher.broadcaster.subscribe();
    watcher
        .execute_action(
            WatchAction::ReindexCode {
                path: relative.clone(),
                created: true,
            },
            "code",
        )
        .await
        .unwrap();
    let symbols = crate::runtime::read(&watcher.facade, |facade| {
        facade.find_symbols_by_name("relative_evidence", None).len()
    })
    .await
    .unwrap();
    assert_eq!(symbols, 1);
    assert!(
        matches!(notifications.try_recv().unwrap(), FileChangeEvent::FileCreated { path } if path == relative)
    );

    std::fs::remove_file(&path).unwrap();
    assert!(!path.exists());
    watcher
        .execute_action(
            WatchAction::RemoveCode {
                path: relative.clone(),
            },
            "code",
        )
        .await
        .unwrap();
    let symbols = crate::runtime::read(&watcher.facade, |facade| {
        facade.find_symbols_by_name("relative_evidence", None).len()
    })
    .await
    .unwrap();
    assert_eq!(symbols, 0);
    assert!(
        matches!(notifications.try_recv().unwrap(), FileChangeEvent::FileDeleted { path } if path == relative)
    );
}

#[tokio::test]
async fn ignored_index_artifacts_cannot_requeue_broad_document_collections() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let policy = root.join(".codannaignore");
    let rules = ".codanna/\ndocuments-index/\ncode-index/\n";
    std::fs::write(&policy, rules).unwrap();
    let source = root.join("guide.json");
    std::fs::write(&source, r#"{"evidence":"local"}"#).unwrap();
    let (mut watcher, store) = fixture(&root, &root);
    // Exercise the ignore matcher independently of the configured-index
    // containment guard; these artifacts are in a different ignored subtree.
    watcher.index_path = root.join("configured-elsewhere-index");
    let mut config = DocumentsConfig {
        enabled: true,
        ..Default::default()
    };
    config.collections.insert(
        "docs".into(),
        CollectionConfig {
            paths: vec![root.clone()],
            patterns: vec!["**/*.json".into()],
            ..Default::default()
        },
    );
    watcher.handlers[0] =
        Box::new(DocumentFileHandler::new(store.clone(), root.clone()).with_config(&config));
    watcher.handle_event(created(&source)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert_eq!(store.read().await.get_indexed_paths(), vec![source.clone()]);

    let artifacts = root.join(".codanna/index/documents/generations");
    std::fs::create_dir_all(&artifacts).unwrap();
    let generation = artifacts.join("gen-fixture.json");
    std::fs::write(&generation, "{}").unwrap();
    for _ in 0..10 {
        for event in [created(&artifacts), modified(&generation)] {
            watcher.handle_event(event).await;
        }
    }
    std::fs::remove_file(&generation).unwrap();
    watcher.handle_event(removed(&generation)).await;
    assert!(
        watcher
            .document_reconciliations
            .read()
            .await
            .pending
            .is_empty()
    );
    assert!(!watcher.debouncer.has_pending());
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        0
    );

    // Policy edits still reach reconciliation. Fresh matchers preserve valid
    // reinclusions without retaining paths from historical event bursts.
    std::fs::write(&policy, format!("{rules}*.json\n!guide.json\n")).unwrap();
    watcher.handle_event(modified(&policy)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    watcher.handle_event(modified(&source)).await;
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        1
    );
    assert_eq!(store.read().await.get_indexed_paths(), vec![source]);
}

#[tokio::test]
async fn custom_managed_index_events_do_not_enter_document_or_source_queues() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let (mut watcher, _) = fixture(&root, &root);
    let managed = root.join("custom-generated-index");
    std::fs::create_dir(&managed).unwrap();
    watcher.index_path = managed.clone();
    let artifact = managed.join("metadata.md");
    std::fs::write(&artifact, "managed metadata").unwrap();
    watcher.handle_event(created(&managed)).await;
    watcher.handle_event(modified(&artifact)).await;
    std::fs::remove_file(&artifact).unwrap();
    watcher.handle_event(removed(&artifact)).await;
    assert!(
        watcher
            .document_reconciliations
            .read()
            .await
            .pending
            .is_empty()
    );
    assert!(!watcher.debouncer.has_pending());
    assert_eq!(
        watcher
            .dispatch_ready_changes(Instant::now())
            .await
            .collection_scans,
        0
    );

    let policy = root.join(".codannaignore");
    std::fs::write(&policy, "custom-generated-index/\n").unwrap();
    watcher.handle_event(modified(&policy)).await;
    assert_eq!(
        watcher.document_reconciliations.read().await.pending.len(),
        1
    );
}

#[test]
fn retry_backoff_is_capped_and_preserves_newer_pending_work() {
    let now = Instant::now();
    let path = Path::new("docs/guide.md");
    let collection = CollectionConfig::default();
    let defaults = ChunkingConfig::default();
    let mut queue = DocumentReconciliations::default();
    queue.record(
        path,
        vec![("docs".into(), collection.clone())],
        defaults.clone(),
        now,
    );
    let mut failed = queue.take_ready(now).remove("docs").unwrap();
    for _ in 0..32 {
        failed.retry(now);
    }
    assert_eq!(failed.retry_at, Some(now + Duration::from_secs(30)));
    let mut newer = collection;
    newer.max_chunk_chars = Some(100);
    queue.record(path, vec![("docs".into(), newer)], defaults, now);
    queue.retry("docs".into(), failed, now);
    assert_eq!(queue.pending.len(), 1);
    assert_eq!(queue.pending["docs"].config.max_chunk_chars, Some(100));
    assert!(queue.take_ready(now + Duration::from_secs(29)).is_empty());
    assert_eq!(queue.take_ready(now + Duration::from_secs(30)).len(), 1);
}
