//! Repeated-update measurements use prepared vectors, never live providers.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore};
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
        "segment-measurement-fixture@1".into()
    }
}

#[test]
fn repeated_updates_report_physical_and_live_vectors_separately() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.md");
    let mut store = DocumentStore::new(temp.path().join("index"), VectorDimension::new(2).unwrap())
        .unwrap()
        .with_embeddings(Box::new(Prepared))
        .unwrap();
    let collection = CollectionConfig {
        paths: vec![source.clone()],
        ..Default::default()
    };
    let chunking = ChunkingConfig {
        min_chunk_chars: 1,
        overlap_chars: 0,
        ..Default::default()
    };
    for revision in 0..4 {
        std::fs::write(
            &source,
            format!("Document revision {revision}: account calendar preference."),
        )
        .unwrap();
        let stats = store
            .index_collection("fixture", &collection, &chunking)
            .unwrap();
        assert_eq!(stats.files_processed, 1);
        assert_eq!(stats.files_skipped, 0);
        assert!(stats.chunks_created > 0);
        let diagnostics = store.embedding_diagnostics();
        let live = store.collection_stats("fixture").unwrap().chunk_count;
        assert!(live > 0);
        assert_eq!(diagnostics.live_vectors, live);
        assert!(diagnostics.physical_vectors >= diagnostics.live_vectors);
        assert_eq!(diagnostics.unembedded_chunks, 0);
        // Record existing behavior without prescribing compaction or assuming
        // every physical vector remains live.
        println!(
            "{}",
            serde_json::json!({
                "fixture_revision": revision, "diagnostics": diagnostics,
                "provider_requests": 0, "qualification": "prepared_storage_only"
            })
        );

        let unchanged = store
            .index_collection("fixture", &collection, &chunking)
            .unwrap();
        assert_eq!(unchanged.files_processed, 0);
        assert_eq!(unchanged.files_skipped, 1);
        assert_eq!(unchanged.chunks_created, 0);
        assert_eq!(unchanged.chunks_removed, 0);
        let after = store.embedding_diagnostics();
        assert_eq!(after.physical_vectors, diagnostics.physical_vectors);
        assert_eq!(after.live_vectors, diagnostics.live_vectors);
        assert_eq!(after.vector_segments, diagnostics.vector_segments);
    }
}
