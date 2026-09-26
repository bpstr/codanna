//! Fixed-vector lifecycle contracts. No model, provider, or production source.
use super::simple::*;
use crate::SymbolId;
use crate::symbol_representation::{CodeEmbeddingPolicy, SymbolInput};
use std::sync::Arc;

fn fixture() -> SimpleSemanticSearch {
    let mut search = SimpleSemanticSearch::new_empty(2, "fixture");
    search
        .set_embedding_identity(CodeEmbeddingPolicy::SymbolBodyV1.bind_identity("{}".into()))
        .unwrap();
    search
}
fn put(search: &mut SimpleSemanticSearch, raw: u32, vectors: &[[f32; 2]], language: &str) {
    let inputs: Vec<_> = vectors
        .iter()
        .enumerate()
        .map(|(index, _)| SymbolInput {
            text: format!("parent {raw} segment {index}"),
            source_range: Some(index * 10..index * 10 + 10),
        })
        .collect();
    let parts = vectors
        .iter()
        .zip(&inputs)
        .map(|(vector, input)| {
            SymbolSegment::new(input.source_range.clone(), Arc::from(vector.as_slice()))
        })
        .collect();
    search
        .store_symbol_segments(SymbolId::new(raw).unwrap(), parts, &inputs, language)
        .unwrap();
}
#[test]
fn symbol_representation_tail_segment_retrieves_parent_once_before_limit() {
    let mut search = fixture();
    put(
        &mut search,
        1,
        &[[1.0, 0.0], [0.0, 1.0], [0.1, 0.9]],
        "rust",
    );
    put(&mut search, 2, &[[0.2, 0.8]], "typescript");
    assert_eq!(search.embedding_count(), 2);
    assert_eq!(search.vector_count(), 4);
    let hits = search.search_with_embedding(&[0.0, 1.0], 2, 0.0).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0], (SymbolId::new(1).unwrap(), 1.0));
    assert_eq!(hits[1].0, SymbolId::new(2).unwrap());
    let filtered = search
        .search_with_embedding_and_language(&[0.0, 1.0], 1, Some("typescript"))
        .unwrap();
    assert_eq!(filtered[0].0, SymbolId::new(2).unwrap());
}
#[test]
fn symbol_representation_snapshot_replacement_and_delete_keep_consistent_parents() {
    let mut search = fixture();
    put(&mut search, 1, &[[1.0, 0.0], [0.0, 1.0]], "rust");
    let pinned = search.query_snapshot();
    put(&mut search, 1, &[[1.0, 0.0]], "rust");
    assert!(
        search
            .search_with_embedding(&[0.0, 1.0], 1, 0.9)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        pinned
            .search_with_embedding_and_language(&[0.0, 1.0], 1, None)
            .unwrap()[0]
            .1,
        1.0
    );
    assert_eq!(search.vector_count(), 1);
    search.remove_embeddings(&[SymbolId::new(1).unwrap()]);
    assert_eq!(search.vector_count(), 0);
}
#[test]
fn symbol_representation_checkpoint_delta_reopen_and_tombstone_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut search = fixture();
    put(&mut search, 1, &[[1.0, 0.0], [0.0, 1.0]], "rust");
    search.save(dir.path()).unwrap();
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("metadata.json")).unwrap()).unwrap();
    assert_eq!(metadata["version"], 4);
    assert_eq!(metadata["segment_embedding_count"], 1);
    let mut loaded = SimpleSemanticSearch::load_without_model(dir.path()).unwrap();
    assert_eq!(loaded.vector_count(), 2);
    assert_eq!(
        loaded.search_with_embedding(&[0.0, 1.0], 1, 0.9).unwrap()[0].0,
        SymbolId::new(1).unwrap()
    );
    put(&mut loaded, 1, &[[1.0, 0.0]], "typescript");
    loaded.save(dir.path()).unwrap();
    let loaded = SimpleSemanticSearch::load_without_model(dir.path()).unwrap();
    assert_eq!(loaded.vector_count(), 1);
    assert!(
        loaded
            .search_with_embedding(&[0.0, 1.0], 1, 0.9)
            .unwrap()
            .is_empty()
    );
    let mut loaded = loaded;
    loaded.remove_embeddings(&[SymbolId::new(1).unwrap()]);
    loaded.save(dir.path()).unwrap();
    assert_eq!(
        SimpleSemanticSearch::load_without_model(dir.path())
            .unwrap()
            .vector_count(),
        0
    );
}
#[test]
fn symbol_representation_legacy_replacement_clears_old_children() {
    let mut search = fixture();
    put(&mut search, 1, &[[1.0, 0.0], [0.0, 1.0]], "rust");
    search.store_embeddings(vec![(
        SymbolId::new(1).unwrap(),
        vec![1.0, 0.0],
        "rust".into(),
    )]);
    assert_eq!(search.vector_count(), 1);
    assert!(
        search
            .search_with_embedding(&[0.0, 1.0], 1, 0.9)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn symbol_representation_corrupt_or_missing_checkpoint_is_not_empty_success() {
    for delete in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut search = fixture();
        put(&mut search, 1, &[[1.0, 0.0], [0.0, 1.0]], "rust");
        search.save(dir.path()).unwrap();
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("metadata.json")).unwrap())
                .unwrap();
        let file = dir
            .path()
            .join(metadata["journal"]["base"].as_str().unwrap())
            .join("symbol-segments.json");
        if delete {
            std::fs::remove_file(file).unwrap();
        } else {
            std::fs::write(file, b"{}").unwrap();
        }
        assert!(SimpleSemanticSearch::load_without_model(dir.path()).is_err());
    }
}
#[test]
fn symbol_representation_invalid_vector_group_is_atomic() {
    let mut search = fixture();
    put(&mut search, 1, &[[1.0, 0.0], [0.0, 1.0]], "rust");
    let inputs = vec![SymbolInput {
        text: "bad".into(),
        source_range: None,
    }];
    let result = search.store_symbol_segments(
        SymbolId::new(1).unwrap(),
        vec![SymbolSegment::new(None, Arc::from([f32::NAN, 0.0]))],
        &inputs,
        "typescript",
    );
    assert!(result.is_err());
    assert_eq!(search.vector_count(), 2);
    assert_eq!(
        search
            .search_with_embedding_and_language(&[0.0, 1.0], 1, Some("rust"))
            .unwrap()[0]
            .1,
        1.0
    );
}
#[test]
fn symbol_representation_identity_changes_reject_legacy_and_changed_budgets() {
    let mut search = fixture();
    put(&mut search, 1, &[[1.0, 0.0]], "rust");
    assert!(search.validate_embedding_identity("{}").is_err());
    let changed =
        CodeEmbeddingPolicy::SymbolBodyV1.bind_identity("{\"input_policy\":\"changed\"}".into());
    assert!(search.validate_embedding_identity(&changed).is_err());
    assert!(!CodeEmbeddingPolicy::SymbolBodyV1.accepts_recorded_identity(None));
    assert!(
        !CodeEmbeddingPolicy::DocComment
            .accepts_recorded_identity(search.metadata().unwrap().embedding_identity.as_deref())
    );
}
