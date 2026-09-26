//! Failed reloads cannot silently keep the previous semantic generation usable.
//! Public persistence/facade boundaries, fixed vectors, no backend or provider.
use codanna::Settings;
use codanna::indexing::facade::IndexFacade;
use codanna::semantic::SimpleSemanticSearch;
use codanna::symbol_representation::CodeEmbeddingPolicy;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

fn fixture(policy: CodeEmbeddingPolicy) -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "/// Conversation timeout dispatch.\npub fn dispatch_ticket() {}\n",
    )
    .unwrap();
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: root.join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.semantic_search.code_representation = policy;
    settings.add_indexed_path(root.join("src")).unwrap();
    let mut facade = IndexFacade::new(Arc::new(settings)).unwrap();
    facade.index_directory(&root.join("src"), true).unwrap();
    let found = facade.find_symbols_by_name("dispatch_ticket", None);
    assert_eq!(found.len(), 1);
    let path = root.join("semantic");
    let mut vectors = SimpleSemanticSearch::new_empty(2, "fixed-fixture");
    vectors.store_embeddings(vec![(found[0].id, vec![1.0, 0.0], "rust".into())]);
    vectors.save(&path).unwrap();
    drop(vectors);
    // Fixed test metadata exercises both source-policy admission paths without
    // creating a query backend or claiming these vectors encode source meaning.
    let manifest = path.join("metadata.json");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    metadata["embedding_identity"] = json!(
        json!({
            "source_input_policy": policy.source_policy(),
            "backend": "fixed-test-vectors"
        })
        .to_string()
    );
    std::fs::write(&manifest, metadata.to_string()).unwrap();
    assert!(facade.load_semantic_search(&path).unwrap());
    assert!(!facade.is_semantic_incompatible());
    (temp, facade)
}

fn assert_invalidated(facade: &IndexFacade, rejected_save: &Path) {
    assert!(
        facade.is_semantic_incompatible(),
        "failed reload left the previous vectors usable"
    );
    let error = facade
        .semantic_search_docs("conversation timeout", 1)
        .unwrap_err();
    assert!(error.to_string().contains("incompatible"), "{error}");
    assert!(facade.save_semantic_search(rejected_save).is_err());
    assert!(
        !rejected_save.exists(),
        "invalid state must not publish a checkpoint"
    );
    let lexical = facade
        .search("conversation timeout", 1, None, None, None)
        .unwrap();
    assert_eq!(lexical[0].name, "dispatch_ticket");
}

#[test]
fn malformed_reload_invalidates_previous_vectors_and_valid_reload_recovers() {
    for policy in [
        CodeEmbeddingPolicy::DocComment,
        CodeEmbeddingPolicy::SymbolBodyV1,
    ] {
        let (temp, mut facade) = fixture(policy);
        let path = temp.path().join("semantic");
        let manifest = path.join("metadata.json");
        let original = std::fs::read(&manifest).unwrap();
        std::fs::write(&manifest, "malformed metadata").unwrap();
        assert!(!facade.load_semantic_search(&path).unwrap());
        assert_invalidated(&facade, &temp.path().join("must-not-publish"));
        assert_eq!(std::fs::read(&manifest).unwrap(), b"malformed metadata");
        std::fs::write(&manifest, &original).unwrap();
        assert!(facade.load_semantic_search(&path).unwrap());
        assert!(!facade.is_semantic_incompatible());
        assert_eq!(std::fs::read(&manifest).unwrap(), original);
    }
}

#[test]
fn removed_manifest_invalidates_previous_vectors_without_recreating_it() {
    for policy in [
        CodeEmbeddingPolicy::DocComment,
        CodeEmbeddingPolicy::SymbolBodyV1,
    ] {
        let (temp, mut facade) = fixture(policy);
        let path = temp.path().join("semantic");
        let manifest = path.join("metadata.json");
        std::fs::remove_file(&manifest).unwrap();
        assert!(!facade.load_semantic_search(&path).unwrap());
        assert_invalidated(&facade, &temp.path().join("must-not-publish"));
        assert!(!manifest.exists());
    }
}

#[test]
fn absent_semantic_store_on_first_load_stays_optional() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let mut facade = IndexFacade::new(Arc::new(settings)).unwrap();
    let absent = temp.path().join("semantic");
    assert!(!facade.load_semantic_search(&absent).unwrap());
    assert!(!facade.is_semantic_incompatible());
    assert!(!absent.exists());
}
