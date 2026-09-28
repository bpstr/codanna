//! Candidate retrieval must not certify support for an absent requirement.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore, SearchQuery};
use codanna::vector::{EmbeddingGenerator, VectorDimension, VectorError};

struct Prepared;
impl EmbeddingGenerator for Prepared {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }
    fn cache_identity(&self) -> String {
        "support-fixture@1".into()
    }
}

#[tokio::test]
async fn nearest_neighbors_can_return_candidates_without_literal_support() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("policy.md");
    std::fs::write(&path, "Account preferences control calendar display.").unwrap();
    let mut store = DocumentStore::new(temp.path().join("index"), VectorDimension::new(2).unwrap())
        .unwrap()
        .with_embeddings(Box::new(Prepared))
        .unwrap();
    store
        .index_collection(
            "fixture",
            &CollectionConfig {
                paths: vec![path],
                ..Default::default()
            },
            &ChunkingConfig {
                min_chunk_chars: 1,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(store.retrieval_mode(), "semantic_nearest_neighbors");
    let results = store
        .search(SearchQuery {
            text: "Every violet narwhal must dance seventeen polkas".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(
        !results.is_empty(),
        "prepared vectors deliberately return unrelated candidates"
    );
    assert!(
        results
            .iter()
            .all(|result| !result.content_preview.contains("narwhal"))
    );
    let mut settings = codanna::Settings {
        index_path: temp.path().join("code"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let facade =
        codanna::indexing::facade::IndexFacade::new(std::sync::Arc::new(settings)).unwrap();
    let server = codanna::mcp::CodeIntelligenceServer::new(facade).with_document_store(store);
    for (collection, expected) in [(None, "returned"), (Some("absent".to_string()), "none")] {
        let response = server
            .search_documents(rmcp::handler::server::wrapper::Parameters(
                codanna::mcp::SearchDocumentsRequest {
                    query: "Every violet narwhal must dance seventeen polkas".into(),
                    collection,
                    limit: 5,
                    literal: false,
                    score_floor: None,
                },
            ))
            .await
            .unwrap();
        let metadata = response.structured_content.unwrap();
        assert_eq!(metadata["retrieval"]["support_status"], "not_assessed");
        assert_eq!(metadata["retrieval"]["candidate_status"], expected);
        assert_eq!(metadata["retrieval"]["scores_are_probabilities"], false);
    }
}

use codanna::documents::DocumentSearchOptions;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct RecordingPrepared(Arc<AtomicUsize>);
impl EmbeddingGenerator for RecordingPrepared {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }
    fn cache_identity(&self) -> String {
        "literal-support-fixture@1".into()
    }
}

fn support_fixture() -> (
    tempfile::TempDir,
    DocumentStore,
    codanna::Settings,
    Arc<AtomicUsize>,
) {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.md");
    let second = temp.path().join("second.md");
    std::fs::write(
        &first,
        "Calendar display. Exact phrase: Account preferences. 雪だるま ☃ ::[]",
    )
    .unwrap();
    std::fs::write(&second, "Preferences account display calendar.").unwrap();
    let mut settings = codanna::Settings {
        index_path: temp.path().join("index"),
        workspace_root: Some(temp.path().to_path_buf()),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.documents.enabled = true;
    let collection = CollectionConfig {
        paths: vec![first, second],
        ..Default::default()
    };
    settings
        .documents
        .collections
        .insert("docs".into(), collection.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut store = DocumentStore::new(
        settings.index_path.join("documents"),
        VectorDimension::new(2).unwrap(),
    )
    .unwrap()
    .with_embeddings(Box::new(RecordingPrepared(calls.clone())))
    .unwrap();
    store
        .index_collection(
            "docs",
            &collection,
            &ChunkingConfig {
                min_chunk_chars: 1,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    calls.store(0, Ordering::SeqCst);
    (temp, store, settings, calls)
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        collection: Some("docs".into()),
        ..Default::default()
    }
}

#[test]
fn literal_search_matches_indexed_phrases_punctuation_and_unicode_without_embeddings() {
    let (temp, mut store, _settings, calls) = support_fixture();
    let options = DocumentSearchOptions {
        literal: true,
        score_floor: None,
    };
    for text in ["Account preferences", "::[]", "雪だるま", "☃"] {
        let results = store.search_with_options(query(text), &options).unwrap();
        assert_eq!(results.len(), 1, "{text}");
        assert_eq!(results[0].similarity, 1.0);
    }
    for text in [
        "preferences Account",
        "account preferences",
        "Account, preferences",
        "violet narwhal",
        "",
        "   ",
    ] {
        assert!(
            store
                .search_with_options(query(text), &options)
                .unwrap()
                .is_empty(),
            "{text}"
        );
    }
    let mut absent = query("Account preferences");
    absent.collection = Some("absent".into());
    assert!(
        store
            .search_with_options(absent, &options)
            .unwrap()
            .is_empty()
    );
    std::fs::write(
        temp.path().join("first.md"),
        "Live source changed without indexing",
    )
    .unwrap();
    assert_eq!(
        store
            .search_with_options(query("Account preferences"), &options)
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .search_with_options(query("Live source changed"), &options)
            .unwrap()
            .is_empty()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn score_floors_include_equal_scores_reject_nonfinite_and_preserve_omitted_behavior() {
    let (_temp, mut store, settings, calls) = support_fixture();
    let baseline = store.search(query("violet narwhal")).unwrap();
    assert!(
        !baseline.is_empty(),
        "prepared vectors deliberately return unrelated candidates"
    );
    for literal in [false, true] {
        let text = if literal {
            "Account preferences"
        } else {
            "violet narwhal"
        };
        let at_floor = DocumentSearchOptions {
            literal,
            score_floor: Some(1.0),
        };
        assert!(
            !store
                .search_with_options(query(text), &at_floor)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .search_with_options(
                    query(text),
                    &DocumentSearchOptions {
                        score_floor: Some(1.1),
                        ..at_floor
                    }
                )
                .unwrap()
                .is_empty()
        );
        for floor in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                store
                    .search_with_options(
                        query(text),
                        &DocumentSearchOptions {
                            literal,
                            score_floor: Some(floor)
                        }
                    )
                    .is_err()
            );
        }
    }
    calls.store(0, Ordering::SeqCst);
    drop(store);
    let mut lexical = DocumentStore::new(
        settings.index_path.join("documents"),
        VectorDimension::new(2).unwrap(),
    )
    .unwrap();
    let results = lexical.search(query("Account")).unwrap();
    assert!(!results.is_empty());
    let highest = results
        .iter()
        .map(|r| r.similarity)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        !lexical
            .search_with_options(
                query("Account"),
                &DocumentSearchOptions {
                    literal: false,
                    score_floor: Some(highest)
                }
            )
            .unwrap()
            .is_empty()
    );
    assert!(
        lexical
            .search_with_options(
                query("Account"),
                &DocumentSearchOptions {
                    literal: false,
                    score_floor: Some(highest + 1.0)
                }
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn literal_mcp_initializes_no_embedding_backend_and_never_certifies_support() {
    let (_temp, store, mut settings, calls) = support_fixture();
    drop(store);
    settings.semantic_search.enabled = true;
    settings.semantic_search.model = "__fixture_unavailable_model__".into();
    let facade = codanna::indexing::facade::IndexFacade::new(Arc::new(settings)).unwrap();
    let server = codanna::mcp::CodeIntelligenceServer::new(facade);
    for text in ["Account preferences", "violet narwhal"] {
        let result = server
            .search_documents(rmcp::handler::server::wrapper::Parameters(
                codanna::mcp::SearchDocumentsRequest {
                    query: text.into(),
                    collection: Some("docs".into()),
                    limit: 5,
                    literal: true,
                    score_floor: Some(1.0),
                },
            ))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        let data = result.structured_content.unwrap();
        assert_eq!(data["retrieval"]["mode"], "literal");
        assert_eq!(data["retrieval"]["score_units"], "exact_match");
        assert_eq!(data["retrieval"]["support_status"], "not_assessed");
        assert_eq!(data["retrieval"]["effective_score_floor"], 1.0);
        assert_eq!(data["retrieval"]["scores_are_probabilities"], false);
        assert_eq!(
            data["retrieval"]["returned_chunks"],
            if text == "violet narwhal" { 0 } else { 1 }
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn literal_cli_and_direct_mcp_json_report_the_same_controls_without_a_model() {
    let (temp, store, mut settings, _calls) = support_fixture();
    drop(store);
    let facade = codanna::indexing::facade::IndexFacade::new(Arc::new(settings.clone())).unwrap();
    drop(facade);
    settings.semantic_search.enabled = true;
    settings.semantic_search.model = "__fixture_unavailable_model__".into();
    let config = temp.path().join("settings.toml");
    std::fs::write(&config, toml::to_string(&settings).unwrap()).unwrap();
    for args in [
        vec![
            "documents",
            "search",
            "query:::[]",
            "--collection",
            "docs",
            "--literal",
            "--score-floor",
            "1",
            "--json",
        ],
        vec![
            "mcp",
            "search_documents",
            "query:::[]",
            "collection:docs",
            "literal:true",
            "score_floor:1",
            "--json",
        ],
        vec![
            "documents",
            "search",
            "Account preferences",
            "--collection",
            "docs",
            "--literal",
            "--score-floor",
            "1",
            "--json",
        ],
        vec![
            "mcp",
            "search_documents",
            "query:Account preferences",
            "collection:docs",
            "literal:true",
            "score_floor:1",
            "--json",
        ],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_codanna"))
            .arg("--config")
            .arg(&config)
            .args(args)
            .current_dir(temp.path())
            .env_clear()
            .env("HOME", temp.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(data["data"].as_array().unwrap().len(), 1);
        assert_eq!(data["meta"]["retrieval"]["mode"], "literal");
        assert_eq!(data["meta"]["retrieval"]["effective_score_floor"], 1.0);
        assert_eq!(data["meta"]["retrieval"]["support_status"], "not_assessed");
    }
}

#[test]
fn document_cli_rejects_invalid_controls_before_opening_documents() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = codanna::Settings {
        index_path: temp.path().join("missing"),
        workspace_root: Some(temp.path().to_path_buf()),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let config = temp.path().join("settings.toml");
    std::fs::write(&config, toml::to_string(&settings).unwrap()).unwrap();
    for args in [
        vec![
            "documents",
            "search",
            "needle",
            "--literal",
            "--score-floor",
            "NaN",
            "--json",
        ],
        vec!["documents", "search", "needle", "score_floor:inf", "--json"],
        vec!["documents", "search", "needle", "literal:perhaps", "--json"],
        vec![
            "mcp",
            "search_documents",
            "query:needle",
            "score_floor:NaN",
            "--json",
        ],
        vec![
            "mcp",
            "search_documents",
            "query:needle",
            "score_floor:inf",
            "--json",
        ],
        vec![
            "mcp",
            "search_documents",
            "query:needle",
            "score_floor:1e100",
            "--json",
        ],
        vec![
            "mcp",
            "search_documents",
            "query:needle",
            "literal:perhaps",
            "--json",
        ],
        vec![
            "mcp",
            "search_documents",
            "query:needle",
            "limit:1001",
            "--json",
        ],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_codanna"))
            .arg("--config")
            .arg(&config)
            .args(&args)
            .current_dir(temp.path())
            .env_clear()
            .env("HOME", temp.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["code"], "INVALID_QUERY", "{args:?}: {envelope}");
        assert!(!settings.index_path.join("documents").exists());
    }
}

#[test]
fn literal_open_refuses_missing_and_foreign_indexes_without_creation() {
    let temp = tempfile::tempdir().unwrap();
    let settings = codanna::Settings {
        index_path: temp.path().join("missing"),
        workspace_root: Some(temp.path().to_path_buf()),
        ..Default::default()
    };
    assert!(codanna::documents::open_literal_from_settings(&settings).is_err());
    assert!(!settings.index_path.exists());
    #[cfg(unix)]
    {
        let (_other, store, other_settings, _calls) = support_fixture();
        drop(store);
        std::fs::create_dir(&settings.index_path).unwrap();
        std::os::unix::fs::symlink(
            other_settings.index_path.join("documents"),
            settings.index_path.join("documents"),
        )
        .unwrap();
        assert!(codanna::documents::open_literal_from_settings(&settings).is_err());
    }
}

#[test]
fn literal_query_preserves_persistence_and_does_not_wait_for_publication() {
    fn snapshot(base: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        walkdir::WalkDir::new(base)
            .into_iter()
            .map(Result::unwrap)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                (
                    entry.path().strip_prefix(base).unwrap().to_path_buf(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }
    let (temp, store, settings, _calls) = support_fixture();
    drop(store);
    let base = settings.index_path.join("documents");
    std::fs::remove_file(base.join("state.json")).unwrap();
    std::fs::write(
        base.join("generations/unused-fixture.json"),
        "preserve obsolete generation evidence",
    )
    .unwrap();
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .open(base.join("publication.lock"))
        .unwrap();
    fs4::fs_std::FileExt::lock_exclusive(&lock).unwrap();
    let before = snapshot(&base);
    let config = temp.path().join("settings.toml");
    std::fs::write(&config, toml::to_string(&settings).unwrap()).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_codanna"))
        .arg("--config")
        .arg(&config)
        .args([
            "documents",
            "search",
            "Account preferences",
            "--literal",
            "--json",
        ])
        .current_dir(temp.path())
        .env_clear()
        .env("HOME", temp.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("literal query waited behind publication lock");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot(&base), before);
    assert!(!base.join("state.json").exists());
}
