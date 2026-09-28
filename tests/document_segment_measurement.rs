//! Repeated-update measurements use prepared vectors, never live providers.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore};
use codanna::vector::{EmbeddingGenerator, VectorDimension, VectorError};

struct Prepared;
impl EmbeddingGenerator for Prepared {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
    fn dimension(&self) -> VectorDimension { VectorDimension::new(2).unwrap() }
    fn cache_identity(&self) -> String { "segment-measurement-fixture@1".into() }
}

#[test]
fn repeated_updates_report_physical_and_live_vectors_separately() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.md");
    let mut store = DocumentStore::new(temp.path().join("index"), VectorDimension::new(2).unwrap())
        .unwrap().with_embeddings(Box::new(Prepared)).unwrap();
    let collection = CollectionConfig { paths: vec![source.clone()], ..Default::default() };
    for revision in 0..4 {
        std::fs::write(&source, format!("Document revision {revision}: account calendar preference.")).unwrap();
        store.index_collection("fixture", &collection,
            &ChunkingConfig { min_chunk_chars: 1, ..Default::default() }).unwrap();
        let diagnostics = store.embedding_diagnostics();
        let live = store.collection_stats("fixture").unwrap().chunk_count;
        assert_eq!(diagnostics.live_vectors, live);
        assert!(diagnostics.physical_vectors >= diagnostics.live_vectors);
        assert_eq!(diagnostics.unembedded_chunks, 0);
        // Future execution records existing compaction behavior rather than
        // prescribing deletion or assuming every stored vector remains live.
        println!("{}", serde_json::json!({
            "fixture_revision": revision, "diagnostics": diagnostics,
            "provider_requests": 0, "qualification": "prepared_storage_only"
        }));
    }
}
