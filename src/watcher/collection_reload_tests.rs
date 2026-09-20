//! Deterministic settings-event fixtures; no provider transport, credentials or model downloads.

use super::*;
use crate::documents::{DocumentsConfig, SearchQuery};
use crate::vector::{EmbeddingGenerator, VectorDimension, VectorError};
use crate::watcher::handlers::{CodeFileHandler, ConfigFileHandler, DocumentFileHandler};
use notify::event::ModifyKind;
use std::fs;
use std::sync::Mutex;

struct ReloadFixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    settings_path: PathBuf,
    settings: crate::Settings,
    watcher: UnifiedWatcher,
    store: Arc<RwLock<DocumentStore>>,
}

impl ReloadFixture {
    async fn new(generator: Option<Box<dyn EmbeddingGenerator>>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("docs")).unwrap();
        fs::create_dir(root.join(crate::init::local_dir_name())).unwrap();
        fs::write(
            root.join("docs/alpha.md"),
            "alpha original evidence ".repeat(8),
        )
        .unwrap();
        let mut settings = crate::Settings {
            index_path: root.join(crate::init::local_dir_name()).join("index"),
            workspace_root: Some(root.clone()),
            ..Default::default()
        };
        settings.semantic_search.enabled = false;
        settings.documents = DocumentsConfig {
            enabled: true,
            defaults: ChunkingConfig {
                min_chunk_chars: 1,
                max_chunk_chars: 256,
                overlap_chars: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        settings.documents.collections.insert(
            "alpha".into(),
            CollectionConfig {
                paths: vec![root.join("docs")],
                ..Default::default()
            },
        );
        let mut store = DocumentStore::new(
            settings.index_path.join("documents"),
            VectorDimension::new(2).unwrap(),
        )
        .unwrap()
        .with_source_exclusion(&settings.index_path);
        if let Some(generator) = generator {
            store = store.with_embeddings(generator).unwrap();
        }
        store
            .index_collection(
                "alpha",
                &settings.documents.collections["alpha"],
                &settings.documents.defaults,
            )
            .unwrap();
        let store = Arc::new(RwLock::new(store));
        let facade = Arc::new(RwLock::new(
            IndexFacade::new(Arc::new(settings.clone())).unwrap(),
        ));
        let settings_path = root
            .join(crate::init::local_dir_name())
            .join("settings.toml");
        fs::write(&settings_path, toml::to_string(&settings).unwrap()).unwrap();
        let mut watcher = UnifiedWatcher::builder()
            .indexer(facade)
            .document_store(Arc::clone(&store))
            .workspace_root(root.clone())
            .index_path(settings.index_path.clone())
            .broadcaster(Arc::new(NotificationBroadcaster::new(32)))
            .handler(
                DocumentFileHandler::new(Arc::clone(&store), root.clone())
                    .with_config(&settings.documents),
            )
            .handler(ConfigFileHandler::new(settings_path.clone()).unwrap())
            .debounce_ms(0)
            .build()
            .unwrap();
        watcher.prepare().await.unwrap();
        Self {
            _temp: temp,
            root,
            settings_path,
            settings,
            watcher,
            store,
        }
    }

    async fn reload(&mut self, settings: &crate::Settings) {
        fs::write(&self.settings_path, toml::to_string(settings).unwrap()).unwrap();
        self.watcher
            .handle_event(
                Event::new(EventKind::Modify(ModifyKind::Any)).add_path(self.settings_path.clone()),
            )
            .await;
        self.watcher.dispatch_ready_changes(Instant::now()).await;
    }

    fn lexical_hits(&self, text: &str) -> Vec<crate::documents::SearchResult> {
        // Reopen only local metadata to inspect durable publication without making
        // a query-embedding call in mock embedding-count assertions.
        DocumentStore::new(
            self.settings.index_path.join("documents"),
            VectorDimension::new(2).unwrap(),
        )
        .unwrap()
        .search(SearchQuery {
            text: text.into(),
            limit: 100,
            ..Default::default()
        })
        .unwrap()
    }
}

#[tokio::test]
async fn collection_reload_add_remove_disable_and_reenable_publish_query_settings() {
    let mut fixture = ReloadFixture::new(None).await;
    let beta = fixture.root.join("more/beta.txt");
    fs::create_dir(beta.parent().unwrap()).unwrap();
    fs::write(&beta, "beta reload evidence").unwrap();
    let mut next = fixture.settings.clone();
    next.documents.collections.insert(
        "beta".into(),
        CollectionConfig {
            paths: vec![PathBuf::from("more")],
            patterns: vec!["**/*.txt".into()],
            ..Default::default()
        },
    );
    next.documents.search.highlight = false;
    next.index_path = PathBuf::from(crate::init::local_dir_name()).join("index");
    fixture.reload(&next).await;
    assert_eq!(fixture.lexical_hits("beta")[0].source_path, beta);
    assert!(
        fixture
            .watcher
            .registry
            .watch_dirs()
            .contains(beta.parent().unwrap())
    );
    let live = fixture
        .watcher
        .facade
        .read()
        .await
        .settings()
        .documents
        .clone();
    assert_eq!(
        live.collections["beta"].paths,
        vec![fixture.root.join("more")]
    );
    assert!(
        !live.search.highlight,
        "query preview settings read the accepted facade snapshot"
    );

    next.documents.collections.remove("alpha");
    fixture.reload(&next).await;
    assert!(fixture.lexical_hits("alpha").is_empty());
    assert_eq!(fixture.store.read().await.list_collections(), vec!["beta"]);
    assert!(fixture.root.join("docs/alpha.md").is_file());
    assert!(
        !fixture
            .watcher
            .registry
            .watch_dirs()
            .contains(&fixture.root.join("docs"))
    );

    next.documents.enabled = false;
    fixture.reload(&next).await;
    assert!(fixture.lexical_hits("beta").is_empty());
    assert!(fixture.store.read().await.list_collections().is_empty());
    assert!(
        !fixture
            .watcher
            .facade
            .read()
            .await
            .settings()
            .documents
            .enabled
    );
    assert!(
        fixture
            .watcher
            .document_reconciliations
            .read()
            .await
            .pending
            .is_empty()
    );
    assert!(beta.is_file());

    next.documents.enabled = true;
    fixture.reload(&next).await;
    assert_eq!(fixture.lexical_hits("beta").len(), 1);
    assert!(
        fixture
            .watcher
            .facade
            .read()
            .await
            .settings()
            .documents
            .enabled
    );
}

struct ReloadModel {
    inputs: Arc<Mutex<Vec<String>>>,
    fail: Arc<AtomicBool>,
}

impl EmbeddingGenerator for ReloadModel {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        self.inputs
            .lock()
            .unwrap()
            .extend(texts.iter().map(|text| (*text).to_owned()));
        if self.fail.load(Ordering::SeqCst) {
            return Err(VectorError::EmbeddingFailed(
                "deterministic reload fixture failure".into(),
            ));
        }
        Ok(vec![vec![1.0, 0.0]; texts.len()])
    }
    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }
    fn cache_identity(&self) -> String {
        "collection-reload-fixture@1".into()
    }
}

#[tokio::test]
async fn collection_reload_globs_and_roots_preserve_retained_embeddings() {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let mut fixture = ReloadFixture::new(Some(Box::new(ReloadModel {
        inputs: Arc::clone(&inputs),
        fail: Arc::new(AtomicBool::new(false)),
    })))
    .await;
    let original_id = fixture.lexical_hits("alpha")[0].chunk_id;
    inputs.lock().unwrap().clear();
    fs::write(
        fixture.root.join("docs/beta.txt"),
        "beta newly admitted text",
    )
    .unwrap();
    let mut next = fixture.settings.clone();
    next.documents
        .collections
        .get_mut("alpha")
        .unwrap()
        .patterns
        .push("**/*.txt".into());
    fixture.reload(&next).await;
    assert_eq!(fixture.lexical_hits("alpha")[0].chunk_id, original_id);
    assert_eq!(
        inputs.lock().unwrap().as_slice(),
        ["beta newly admitted text"]
    );

    inputs.lock().unwrap().clear();
    fs::create_dir(fixture.root.join("more")).unwrap();
    fs::write(fixture.root.join("more/gamma.md"), "gamma added root").unwrap();
    next.documents
        .collections
        .get_mut("alpha")
        .unwrap()
        .paths
        .push(PathBuf::from("more"));
    fixture.reload(&next).await;
    let gamma_id = fixture.lexical_hits("gamma")[0].chunk_id;
    assert_eq!(inputs.lock().unwrap().as_slice(), ["gamma added root"]);
    inputs.lock().unwrap().clear();
    next.documents.collections.get_mut("alpha").unwrap().paths = vec![PathBuf::from("more")];
    fixture.reload(&next).await;
    assert!(fixture.lexical_hits("alpha beta").is_empty());
    assert_eq!(fixture.lexical_hits("gamma")[0].chunk_id, gamma_id);
    assert!(inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn collection_reload_default_and_collection_chunking_replace_only_affected_chunks() {
    let mut fixture = ReloadFixture::new(None).await;
    fs::create_dir(fixture.root.join("more")).unwrap();
    fs::write(
        fixture.root.join("more/beta.md"),
        "beta override evidence ".repeat(8),
    )
    .unwrap();
    let mut next = fixture.settings.clone();
    next.documents.collections.insert(
        "beta".into(),
        CollectionConfig {
            paths: vec![PathBuf::from("more")],
            max_chunk_chars: Some(256),
            ..Default::default()
        },
    );
    fixture.reload(&next).await;
    let alpha_before = fixture.lexical_hits("alpha")[0].chunk_id;
    let beta_before = fixture.lexical_hits("beta")[0].chunk_id;
    next.documents.defaults.max_chunk_chars = 64;
    fixture.reload(&next).await;
    assert_eq!(
        fixture
            .store
            .read()
            .await
            .collection_stats("alpha")
            .unwrap()
            .chunk_count,
        3
    );
    assert!(
        fixture
            .lexical_hits("alpha")
            .iter()
            .all(|hit| hit.chunk_id != alpha_before)
    );
    assert_eq!(fixture.lexical_hits("beta")[0].chunk_id, beta_before);
    next.documents
        .collections
        .get_mut("beta")
        .unwrap()
        .max_chunk_chars = Some(48);
    fixture.reload(&next).await;
    assert_eq!(
        fixture
            .store
            .read()
            .await
            .collection_stats("beta")
            .unwrap()
            .chunk_count,
        4
    );
    assert!(
        fixture
            .lexical_hits("beta")
            .iter()
            .all(|hit| hit.chunk_id != beta_before)
    );
}

#[tokio::test]
async fn collection_reload_invalid_settings_and_identity_changes_keep_working_policy() {
    let mut fixture = ReloadFixture::new(None).await;
    let before = fixture.lexical_hits("alpha")[0].chunk_id;
    let original = fixture.settings.documents.clone();
    let mut candidates = Vec::new();
    let mut bad_glob = fixture.settings.clone();
    bad_glob
        .documents
        .collections
        .get_mut("alpha")
        .unwrap()
        .patterns = vec!["[".into()];
    candidates.push(bad_glob);
    let mut bad_defaults = fixture.settings.clone();
    bad_defaults.documents.defaults.min_chunk_chars = 256;
    candidates.push(bad_defaults);
    let mut bad_override = fixture.settings.clone();
    bad_override
        .documents
        .collections
        .get_mut("alpha")
        .unwrap()
        .max_chunk_chars = Some(1);
    candidates.push(bad_override);
    let mut index = fixture.settings.clone();
    index.index_path = fixture.root.join("another-index");
    candidates.push(index);
    let mut workspace = fixture.settings.clone();
    workspace.workspace_root = Some(fixture.root.join("another-workspace"));
    candidates.push(workspace);
    for field in [
        "model",
        "model_revision",
        "max_input_tokens",
        "remote_dim",
        "enabled",
        "tokenizer_path",
    ] {
        let mut candidate = fixture.settings.clone();
        match field {
            "model" => candidate.semantic_search.model = "unloaded-model".into(),
            "model_revision" => {
                candidate.semantic_search.model_revision = Some("revision-2".into())
            }
            "max_input_tokens" => candidate.semantic_search.max_input_tokens = Some(77),
            "remote_dim" => candidate.semantic_search.remote_dim = Some(8),
            "enabled" => candidate.semantic_search.enabled = true,
            "tokenizer_path" => {
                candidate.semantic_search.tokenizer_path = Some(PathBuf::from("missing.json"))
            }
            _ => unreachable!(),
        }
        candidate.documents.collections.clear();
        candidates.push(candidate);
    }
    for mut candidate in candidates {
        candidate
            .indexing
            .indexed_paths
            .push(fixture.root.join("new-code"));
        fixture.reload(&candidate).await;
        assert_eq!(
            fixture.watcher.facade.read().await.settings().documents,
            original
        );
        assert!(
            fixture
                .watcher
                .facade
                .read()
                .await
                .settings()
                .indexed_paths_cache
                .is_empty()
        );
        assert_eq!(fixture.lexical_hits("alpha")[0].chunk_id, before);
        assert!(fixture.watcher.pending_config.read().await.is_none());
    }
    fs::write(&fixture.settings_path, "[documents\nnot toml").unwrap();
    fixture
        .watcher
        .process_modification(&fixture.settings_path)
        .await;
    fixture.watcher.dispatch_ready_changes(Instant::now()).await;
    assert_eq!(fixture.lexical_hits("alpha")[0].chunk_id, before);
    assert_eq!(
        fixture.watcher.facade.read().await.settings().documents,
        original
    );
}

#[tokio::test]
async fn collection_reload_failed_batch_preserves_all_collections_then_retries_without_event() {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(AtomicBool::new(false));
    let mut fixture = ReloadFixture::new(Some(Box::new(ReloadModel {
        inputs: Arc::clone(&inputs),
        fail: Arc::clone(&fail),
    })))
    .await;
    let old = fixture.lexical_hits("alpha")[0].chunk_id;
    fs::create_dir(fixture.root.join("more")).unwrap();
    fs::write(
        fixture.root.join("more/beta.md"),
        "beta replacement evidence",
    )
    .unwrap();
    let mut next = fixture.settings.clone();
    next.documents.collections.clear();
    next.documents.collections.insert(
        "beta".into(),
        CollectionConfig {
            paths: vec![PathBuf::from("more")],
            ..Default::default()
        },
    );
    fail.store(true, Ordering::SeqCst);
    fixture.reload(&next).await;
    assert_eq!(fixture.lexical_hits("alpha")[0].chunk_id, old);
    assert!(fixture.lexical_hits("beta").is_empty());
    assert_eq!(
        fixture.watcher.facade.read().await.settings().documents,
        fixture.settings.documents
    );
    assert!(
        !fixture
            .watcher
            .registry
            .watch_dirs()
            .contains(&fixture.root.join("more"))
    );
    let calls = inputs.lock().unwrap().len();
    // This clock jump is beyond the bounded retry deadline and requires no edit.
    fail.store(false, Ordering::SeqCst);
    fixture
        .watcher
        .dispatch_ready_changes(Instant::now() + Duration::from_secs(31))
        .await;
    assert!(fixture.lexical_hits("alpha").is_empty());
    assert_eq!(fixture.lexical_hits("beta").len(), 1);
    assert_eq!(inputs.lock().unwrap().len(), calls + 1);
    assert!(fixture.watcher.pending_config.read().await.is_none());
}

#[tokio::test]
async fn collection_reload_removal_prunes_pending_retry_and_rejects_captured_old_actions() {
    let mut fixture = ReloadFixture::new(None).await;
    let source = fixture.root.join("docs/alpha.md");
    let stale_action = fixture
        .watcher
        .handlers
        .iter()
        .find(|handler| handler.name() == "document")
        .unwrap()
        .on_modify(&source)
        .await
        .unwrap();
    fixture
        .watcher
        .execute_action(stale_action.clone(), "document")
        .await
        .unwrap();
    {
        let mut queue = fixture.watcher.document_reconciliations.write().await;
        let mut failed = queue.take_ready(Instant::now()).remove("alpha").unwrap();
        failed.retry(Instant::now());
        queue.pending.insert("alpha".into(), failed);
    }
    let mut next = fixture.settings.clone();
    next.documents.collections.clear();
    fixture.reload(&next).await;
    fixture
        .watcher
        .execute_action(stale_action, "document")
        .await
        .unwrap();
    fixture
        .watcher
        .handle_event(Event::new(EventKind::Modify(ModifyKind::Any)).add_path(source.clone()))
        .await;
    fixture
        .watcher
        .dispatch_ready_changes(Instant::now() + Duration::from_secs(60))
        .await;
    assert!(
        fixture
            .watcher
            .document_reconciliations
            .read()
            .await
            .pending
            .is_empty()
    );
    assert!(fixture.lexical_hits("alpha").is_empty());
    assert!(fixture.store.read().await.list_collections().is_empty());
    assert!(source.is_file());
}

#[tokio::test]
async fn collection_reload_transfers_source_ownership_atomically_and_preserves_pinned_queries() {
    let mut fixture = ReloadFixture::new(None).await;
    let mut pinned = fixture.store.read().await.query_snapshot();
    let mut next = fixture.settings.clone();
    next.documents.collections.insert(
        "earlier".into(),
        CollectionConfig {
            paths: vec![fixture.root.join("docs/alpha.md")],
            ..Default::default()
        },
    );
    next.documents.collections.get_mut("alpha").unwrap().paths = vec![fixture.root.join("empty")];
    fixture.reload(&next).await;
    assert_eq!(fixture.lexical_hits("alpha")[0].collection, "earlier");
    assert_eq!(
        pinned
            .search(SearchQuery {
                text: "alpha".into(),
                ..Default::default()
            })
            .unwrap()[0]
            .collection,
        "alpha"
    );
    let mut overlapping = next.clone();
    overlapping
        .documents
        .collections
        .get_mut("alpha")
        .unwrap()
        .paths = vec![fixture.root.join("docs")];
    fixture.reload(&overlapping).await;
    assert_eq!(fixture.lexical_hits("alpha")[0].collection, "earlier");
    assert_eq!(
        fixture
            .watcher
            .facade
            .read()
            .await
            .settings()
            .documents
            .collections["alpha"]
            .paths,
        vec![fixture.root.join("empty")]
    );
}

#[tokio::test]
async fn collection_reload_committed_generation_wins_over_failed_state_mirror() {
    let mut fixture = ReloadFixture::new(None).await;
    let mirror = fixture.settings.index_path.join("documents/state.json");
    fs::remove_file(&mirror).unwrap();
    fs::create_dir(&mirror).unwrap();
    let mut next = fixture.settings.clone();
    next.documents.collections.clear();
    fixture.reload(&next).await;
    assert!(fixture.lexical_hits("alpha").is_empty());
    assert!(
        fixture
            .watcher
            .facade
            .read()
            .await
            .settings()
            .documents
            .collections
            .is_empty()
    );
    assert!(fixture.watcher.pending_config.read().await.is_none());
    fs::remove_dir(mirror).unwrap();
}

#[tokio::test]
async fn collection_reload_managed_roots_and_code_document_overlap_keep_separate_admission() {
    let mut fixture = ReloadFixture::new(None).await;
    let code = fixture.root.join("docs/shared.rs");
    fs::write(&code, "pub fn shared_reload_fixture() {}\n").unwrap();
    fixture.settings.indexing.indexed_paths = vec![fixture.root.join("docs")];
    let code_roots = fixture.settings.indexing.indexed_paths.clone();
    crate::runtime::mutate(&fixture.watcher.facade, move |facade| {
        facade.reload_indexed_paths(code_roots)
    })
    .await
    .unwrap();
    // Existing code is indexed through its absolute source lane. Directory
    // catch-up's older CWD coupling is separate from document policy reload.
    fixture
        .watcher
        .execute_action(
            WatchAction::ReindexCode {
                path: code.clone(),
                created: true,
            },
            "code",
        )
        .await
        .unwrap();
    fixture.watcher.handlers.push(Box::new(CodeFileHandler::new(
        Arc::clone(&fixture.watcher.facade),
        fixture.root.clone(),
    )));
    fixture
        .watcher
        .handlers
        .last()
        .unwrap()
        .refresh_paths()
        .await
        .unwrap();
    fixture.watcher.register_handler_roots().await.unwrap();
    let artifact = fixture.settings.index_path.join("generated.md");
    fs::write(&artifact, "managed artifact evidence").unwrap();
    let mut next = fixture.settings.clone();
    next.indexing.indexed_paths = vec![fixture.root.join("docs")];
    next.documents
        .collections
        .get_mut("alpha")
        .unwrap()
        .patterns
        .push("**/*.rs".into());
    next.documents.collections.insert(
        "managed".into(),
        CollectionConfig {
            paths: vec![fixture.settings.index_path.clone()],
            ..Default::default()
        },
    );
    fixture.reload(&next).await;
    assert_eq!(
        fixture.store.read().await.get_file_collection(&code),
        Some("alpha")
    );
    assert_eq!(
        crate::runtime::read(&fixture.watcher.facade, |facade| facade
            .find_symbols_by_name("shared_reload_fixture", None)
            .len())
        .await
        .unwrap(),
        1
    );
    assert!(fixture.lexical_hits("managed").is_empty());
    assert!(
        !fixture
            .watcher
            .registry
            .watch_dirs()
            .contains(&fixture.settings.index_path)
    );
    next.documents.collections.clear();
    fixture.reload(&next).await;
    fs::write(&code, "pub fn changed_code_after_document_removal() {}\n").unwrap();
    fixture
        .watcher
        .handle_event(Event::new(EventKind::Modify(ModifyKind::Any)).add_path(code))
        .await;
    fixture.watcher.dispatch_ready_changes(Instant::now()).await;
    assert_eq!(
        crate::runtime::read(&fixture.watcher.facade, |facade| facade
            .find_symbols_by_name("changed_code_after_document_removal", None)
            .len())
        .await
        .unwrap(),
        1
    );
    assert!(fixture.store.read().await.get_indexed_paths().is_empty());
}

#[tokio::test]
async fn collection_reload_without_attached_store_requires_restart_without_loading_a_model() {
    let mut fixture = ReloadFixture::new(None).await;
    fixture.watcher.document_store = None;
    fixture
        .watcher
        .handlers
        .retain(|handler| handler.name() != "document");
    let old = fixture.lexical_hits("alpha")[0].chunk_id;
    let mut next = fixture.settings.clone();
    next.documents.collections.clear();
    fixture.reload(&next).await;
    assert!(fixture.watcher.pending_config.read().await.is_none());
    assert_eq!(
        fixture.watcher.facade.read().await.settings().documents,
        fixture.settings.documents
    );
    assert_eq!(fixture.lexical_hits("alpha")[0].chunk_id, old);
}

#[cfg(unix)]
#[tokio::test]
async fn collection_reload_alias_missing_roots_and_dangling_links_respect_workspace_boundary() {
    use std::os::unix::fs::symlink;
    let mut fixture = ReloadFixture::new(None).await;
    symlink(fixture.root.join("docs"), fixture.root.join("alias")).unwrap();
    let old = fixture.lexical_hits("alpha")[0].chunk_id;
    let mut next = fixture.settings.clone();
    next.documents.collections.get_mut("alpha").unwrap().paths = vec![PathBuf::from("alias")];
    fixture.reload(&next).await;
    assert_eq!(fixture.lexical_hits("alpha")[0].chunk_id, old);
    next.documents.collections.insert(
        "future".into(),
        CollectionConfig {
            paths: vec![PathBuf::from("new/nested")],
            ..Default::default()
        },
    );
    fixture.reload(&next).await;
    assert!(
        fixture
            .watcher
            .facade
            .read()
            .await
            .settings()
            .documents
            .collections
            .contains_key("future")
    );
    fs::create_dir_all(fixture.root.join("new/nested")).unwrap();
    fs::write(
        fixture.root.join("new/nested/future.md"),
        "future root evidence",
    )
    .unwrap();
    fixture
        .watcher
        .handle_event(
            Event::new(EventKind::Create(notify::event::CreateKind::Folder))
                .add_path(fixture.root.join("new")),
        )
        .await;
    fixture.watcher.dispatch_ready_changes(Instant::now()).await;
    assert_eq!(fixture.lexical_hits("future").len(), 1);

    symlink(
        fixture.root.join("unavailable"),
        fixture.root.join("dangling"),
    )
    .unwrap();
    let accepted = fixture
        .watcher
        .facade
        .read()
        .await
        .settings()
        .documents
        .clone();
    next.documents.collections.get_mut("alpha").unwrap().paths =
        vec![fixture.root.join("dangling")];
    fixture.reload(&next).await;
    assert_eq!(
        fixture.watcher.facade.read().await.settings().documents,
        accepted
    );
    fixture
        .watcher
        .facade
        .write()
        .await
        .restrict_workspace(fixture.root.clone())
        .unwrap();
    let foreign = tempfile::tempdir().unwrap();
    fs::write(foreign.path().join("foreign.md"), "foreign secret evidence").unwrap();
    next = fixture.settings.clone();
    next.documents.collections.insert(
        "outside".into(),
        CollectionConfig {
            paths: vec![foreign.path().to_path_buf()],
            ..Default::default()
        },
    );
    fixture.reload(&next).await;
    assert_eq!(
        fixture.watcher.facade.read().await.settings().documents,
        accepted
    );
    assert!(fixture.lexical_hits("foreign").is_empty());
    symlink(
        foreign.path().join("foreign.md"),
        fixture.root.join("docs/escape.md"),
    )
    .unwrap();
    next = fixture.settings.clone();
    next.documents.defaults.max_chunk_chars = 128;
    fixture.reload(&next).await;
    assert_eq!(
        fixture.watcher.facade.read().await.settings().documents,
        accepted
    );
    assert!(fixture.lexical_hits("foreign").is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn collection_reload_retargeted_startup_alias_compares_accepted_canonical_roots() {
    use std::os::unix::fs::symlink;
    let mut fixture = ReloadFixture::new(None).await;
    let alias = fixture.root.join("startup-alias");
    symlink(fixture.root.join("docs"), &alias).unwrap();
    let mut startup = fixture.settings.clone();
    startup
        .documents
        .collections
        .get_mut("alpha")
        .unwrap()
        .paths = vec![alias.clone()];
    fixture
        .watcher
        .facade
        .write()
        .await
        .reload_watched_settings(Vec::new(), startup.documents.clone());
    fixture
        .watcher
        .handlers
        .retain(|handler| handler.name() != "document");
    fixture.watcher.handlers.push(Box::new(
        DocumentFileHandler::new(Arc::clone(&fixture.store), fixture.root.clone())
            .with_config(&startup.documents),
    ));
    fs::create_dir(fixture.root.join("replacement")).unwrap();
    fs::write(
        fixture.root.join("replacement/beta.md"),
        "beta retargeted alias evidence",
    )
    .unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(fixture.root.join("replacement"), &alias).unwrap();
    fixture.reload(&startup).await;
    assert!(fixture.lexical_hits("alpha").is_empty());
    assert_eq!(fixture.lexical_hits("beta").len(), 1);
    assert_eq!(
        fixture
            .watcher
            .facade
            .read()
            .await
            .settings()
            .documents
            .collections["alpha"]
            .paths,
        vec![fixture.root.join("replacement")]
    );
}

#[tokio::test]
async fn collection_reload_overflow_cancels_pending_proposals_after_invalid_or_deleted_settings() {
    for deleted in [false, true] {
        let fail = Arc::new(AtomicBool::new(false));
        let mut fixture = ReloadFixture::new(Some(Box::new(ReloadModel {
            inputs: Arc::new(Mutex::new(Vec::new())),
            fail: Arc::clone(&fail),
        })))
        .await;
        let old = fixture.lexical_hits("alpha")[0].chunk_id;
        let mut next = fixture.settings.clone();
        next.documents.defaults.max_chunk_chars = 32;
        fail.store(true, Ordering::SeqCst);
        fixture.reload(&next).await;
        assert!(fixture.watcher.pending_config.read().await.is_some());
        if deleted {
            fs::remove_file(&fixture.settings_path).unwrap();
        } else {
            fs::write(&fixture.settings_path, "[invalid toml").unwrap();
        }
        fail.store(false, Ordering::SeqCst);
        fixture
            .watcher
            .event_overflowed
            .store(true, Ordering::Release);
        fixture
            .watcher
            .dispatch_ready_changes(Instant::now() + Duration::from_secs(60))
            .await;
        assert!(fixture.watcher.pending_config.read().await.is_none());
        assert_eq!(
            fixture.watcher.facade.read().await.settings().documents,
            fixture.settings.documents
        );
        assert_eq!(fixture.lexical_hits("alpha")[0].chunk_id, old);
    }
}

#[tokio::test]
async fn collection_reload_observed_settings_edits_cancel_retries_before_debounce() {
    for change in 0..3 {
        let fail = Arc::new(AtomicBool::new(false));
        let mut fixture = ReloadFixture::new(Some(Box::new(ReloadModel {
            inputs: Arc::new(Mutex::new(Vec::new())),
            fail: Arc::clone(&fail),
        })))
        .await;
        let old = fixture.lexical_hits("alpha")[0].chunk_id;
        let mut next = fixture.settings.clone();
        next.documents.defaults.max_chunk_chars = 32;
        fail.store(true, Ordering::SeqCst);
        fixture.reload(&next).await;
        assert!(fixture.watcher.pending_config.read().await.is_some());
        fixture.watcher.debouncer = Debouncer::new(60_000);
        fail.store(false, Ordering::SeqCst);
        if change != 0 {
            fs::remove_file(&fixture.settings_path).unwrap();
        } else {
            fs::write(&fixture.settings_path, "[invalid toml").unwrap();
        }
        let event_path = if change == 2 {
            fixture.settings_path.parent().unwrap().to_path_buf()
        } else {
            fixture.settings_path.clone()
        };
        fixture
            .watcher
            .handle_event(Event::new(EventKind::Modify(ModifyKind::Any)).add_path(event_path))
            .await;
        assert!(
            fixture.watcher.pending_config.read().await.is_none(),
            "observed newer settings supersede the old proposal immediately"
        );
        fixture
            .watcher
            .dispatch_ready_changes(Instant::now() + Duration::from_secs(60))
            .await;
        assert_eq!(
            fixture.watcher.facade.read().await.settings().documents,
            fixture.settings.documents
        );
        assert_eq!(fixture.lexical_hits("alpha")[0].chunk_id, old);
    }
}

#[tokio::test]
async fn collection_reload_recovers_a_committed_generation_before_a_reverted_proposal() {
    let mut fixture = ReloadFixture::new(None).await;
    let mut pinned = fixture.store.read().await.query_snapshot();
    let base = fixture.settings.index_path.join("documents");
    let staged = fixture.root.join("staged.md");
    fs::write(&staged, "staged unpublished configuration evidence").unwrap();
    let mut external = DocumentStore::new(&base, VectorDimension::new(2).unwrap()).unwrap();
    external.delete_collection("alpha").unwrap();
    external
        .index_collection(
            "staged",
            &CollectionConfig {
                paths: vec![staged],
                ..Default::default()
            },
            &fixture.settings.documents.defaults,
        )
        .unwrap();
    drop(external);
    let reserved: u64 =
        serde_json::from_slice(&fs::read(base.join("next-chunk-id.json")).unwrap()).unwrap();
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(base.join("tantivy/meta.json")).unwrap()).unwrap();
    let name = metadata["payload"]
        .as_str()
        .unwrap()
        .strip_prefix("codanna-documents-v1:")
        .unwrap();
    let generation = base.join("generations").join(name);
    let bytes = fs::read(&generation).unwrap();
    fs::remove_file(&generation).unwrap();
    // The latest settings are the original policy: a no-op against live settings,
    // but different from the durable publication the reader has not recovered.
    let latest = fixture.settings.clone();
    fixture.reload(&latest).await;
    assert!(fixture.watcher.pending_config.read().await.is_some());
    assert_eq!(fixture.store.read().await.list_collections(), vec!["alpha"]);
    assert_eq!(
        fixture.watcher.facade.read().await.settings().documents,
        latest.documents
    );
    assert_eq!(
        pinned
            .search(SearchQuery {
                text: "alpha".into(),
                ..Default::default()
            })
            .unwrap()
            .len(),
        1
    );
    fs::write(generation, bytes).unwrap();
    let original_source = fixture.root.join("docs/alpha.md");
    fs::write(&original_source, [0xff, 0xfe]).unwrap();
    fixture
        .watcher
        .dispatch_ready_changes(Instant::now() + Duration::from_secs(60))
        .await;
    assert!(fixture.watcher.pending_config.read().await.is_some());
    assert_eq!(
        fixture.store.read().await.list_collections(),
        vec!["alpha"],
        "failed latest-policy apply restores the prior live generation"
    );
    let live = fixture
        .store
        .read()
        .await
        .query_snapshot()
        .search(SearchQuery {
            text: "alpha".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        live.len(),
        1,
        "a shared reader reload cannot change the restored live snapshot"
    );
    // A new no-op proposal must not lose the need for full reconciliation.
    fixture.reload(&latest).await;
    assert!(fixture.watcher.pending_config.read().await.is_some());
    fs::write(original_source, "alpha restored original evidence").unwrap();
    fixture
        .watcher
        .dispatch_ready_changes(Instant::now() + Duration::from_secs(60))
        .await;
    assert!(fixture.watcher.pending_config.read().await.is_none());
    assert!(
        fixture.lexical_hits("alpha")[0].chunk_id.get() as u64 >= reserved,
        "recovery must honor another writer's reserved chunk IDs"
    );
    assert_eq!(fixture.store.read().await.list_collections(), vec!["alpha"]);
    assert!(fixture.lexical_hits("staged").is_empty());
    assert_eq!(fixture.lexical_hits("alpha").len(), 1);
}

#[tokio::test]
async fn collection_reload_recovery_checks_workspace_before_exposing_foreign_sources() {
    let mut fixture = ReloadFixture::new(None).await;
    fixture
        .watcher
        .facade
        .write()
        .await
        .restrict_workspace(fixture.root.clone())
        .unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let source = foreign.path().join("foreign.md");
    fs::write(&source, "foreign recovery evidence").unwrap();
    let mut external = DocumentStore::new(
        fixture.settings.index_path.join("documents"),
        VectorDimension::new(2).unwrap(),
    )
    .unwrap();
    external
        .index_collection(
            "foreign",
            &CollectionConfig {
                paths: vec![source.clone()],
                ..Default::default()
            },
            &fixture.settings.documents.defaults,
        )
        .unwrap();
    drop(external);
    let latest = fixture.settings.clone();
    fixture.reload(&latest).await;
    assert!(fixture.watcher.pending_config.read().await.is_some());
    assert_eq!(
        fixture.store.read().await.get_file_collection(&source),
        None
    );
    assert_eq!(fixture.store.read().await.list_collections(), vec!["alpha"]);
    assert_eq!(
        fixture.watcher.facade.read().await.settings().documents,
        latest.documents
    );
    let hits = fixture
        .store
        .read()
        .await
        .query_snapshot()
        .search(SearchQuery {
            text: "foreign".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(hits.is_empty());
}
