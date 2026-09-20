//! Document indexing and retrieval regressions, using deterministic local vectors.
use codanna::documents::{
    Chunker, ChunkingConfig, CollectionConfig, DocumentStore, HybridChunker, PreviewMode,
    SearchConfig, SearchQuery, ValidatedChunkingConfig,
};
use codanna::vector::{EmbeddingGenerator, VectorDimension, VectorError};
use std::fs::{self, File, FileTimes};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};
use tempfile::TempDir;

fn dimension() -> VectorDimension {
    VectorDimension::new(2).unwrap()
}

fn chunks(max: usize) -> ChunkingConfig {
    ChunkingConfig {
        min_chunk_chars: 1,
        max_chunk_chars: max,
        overlap_chars: 0,
        ..Default::default()
    }
}

fn collection(path: &Path) -> CollectionConfig {
    CollectionConfig {
        paths: vec![path.to_path_buf()],
        ..Default::default()
    }
}

fn query(text: &str, collection: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        collection: Some(collection.into()),
        document: None,
        limit: 20,
        preview_config: Some(SearchConfig {
            preview_mode: PreviewMode::Full,
            preview_chars: 600,
            highlight: false,
        }),
    }
}

fn write_at_fixed_time(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_000)))
        .unwrap();
}

struct MockModel {
    revision: &'static str,
    flipped: bool,
    fail_calls: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

impl MockModel {
    fn new(revision: &'static str, flipped: bool) -> Self {
        Self {
            revision,
            flipped,
            fail_calls: Arc::new(AtomicUsize::new(0)),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl EmbeddingGenerator for MockModel {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        self.calls.fetch_add(texts.len(), Ordering::SeqCst);
        if self
            .fail_calls
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(VectorError::EmbeddingFailed(
                "deterministic fixture failure".into(),
            ));
        }
        Ok(texts
            .iter()
            .map(|text| {
                let alpha = text.contains("alpha");
                if alpha ^ self.flipped {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }
    fn dimension(&self) -> VectorDimension {
        dimension()
    }
    fn cache_identity(&self) -> String {
        self.revision.into()
    }
}

#[test]
fn force_reindex_preserves_one_live_chunk_per_source() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "alpha policy").unwrap();
    let mut store = DocumentStore::new(temp.path().join("index"), dimension()).unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    store.clear_file_states(); // Exact operation used by documents index --force.
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    assert_eq!(
        store.collection_stats("docs").unwrap().chunk_count,
        1,
        "force reindex must replace the prior generation"
    );
}

#[test]
fn force_selected_collection_preserves_other_collection_tracking() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a.md");
    let b = temp.path().join("b.md");
    fs::write(&a, "alpha").unwrap();
    fs::write(&b, "beta").unwrap();
    let mut store = DocumentStore::new(temp.path().join("index"), dimension()).unwrap();
    store
        .index_collection("a", &collection(&a), &chunks(256))
        .unwrap();
    store
        .index_collection("b", &collection(&b), &chunks(256))
        .unwrap();
    store.clear_file_states();
    store
        .index_collection("a", &collection(&a), &chunks(256))
        .unwrap();
    assert_eq!(
        store.get_file_collection(&b),
        Some("b"),
        "forcing A must preserve B's freshness and watcher state"
    );
}

#[test]
fn overlapping_collections_are_indexed_or_rejected_explicitly() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("shared.md");
    fs::write(&path, "alpha").unwrap();
    let mut store = DocumentStore::new(temp.path().join("index"), dimension()).unwrap();
    store
        .index_collection("a", &collection(&path), &chunks(256))
        .unwrap();
    if store
        .index_collection("b", &collection(&path), &chunks(256))
        .is_err()
    {
        return;
    }
    assert_eq!(
        store.collection_stats("b").unwrap().chunk_count,
        1,
        "successful indexing of B cannot silently omit its shared source"
    );
}

#[test]
fn overlapping_roots_deduplicate_discovered_files() {
    let temp = TempDir::new().unwrap();
    let docs = temp.path().join("docs");
    fs::create_dir(&docs).unwrap();
    let path = docs.join("guide.md");
    fs::write(&path, "alpha").unwrap();
    let mut config = collection(&docs);
    config.paths.push(path);
    let mut store = DocumentStore::new(temp.path().join("index"), dimension()).unwrap();
    store
        .index_collection("docs", &config, &chunks(256))
        .unwrap();
    assert_eq!(
        store.collection_stats("docs").unwrap().chunk_count,
        1,
        "a directory plus explicit child file must index the source once"
    );
}

#[test]
fn equal_second_mtime_does_not_hide_content_edits() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    write_at_fixed_time(&path, "alpha old");
    let mut store = DocumentStore::new(temp.path().join("index"), dimension()).unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    write_at_fixed_time(&path, "alpha new");
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    let hits = store.search(query("alpha", "docs")).unwrap();
    assert_eq!(
        hits[0].content_preview, "alpha new",
        "preserved/coarse timestamps cannot be proof of unchanged content"
    );
}

#[test]
fn chunking_config_changes_invalidate_unchanged_file_state() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "a".repeat(200)).unwrap();
    let mut store = DocumentStore::new(temp.path().join("index"), dimension()).unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(50))
        .unwrap();
    assert_eq!(
        store.collection_stats("docs").unwrap().chunk_count,
        4,
        "chunking fingerprint changes require rechunking even if file bytes are unchanged"
    );
}

#[test]
fn enabling_embeddings_populates_existing_metadata_only_chunks() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "alpha policy").unwrap();
    let index = temp.path().join("index");
    {
        let mut store = DocumentStore::new(&index, dimension()).unwrap();
        store
            .index_collection("docs", &collection(&path), &chunks(256))
            .unwrap();
    }
    let mut store = DocumentStore::new(&index, dimension())
        .unwrap()
        .with_embeddings(Box::new(MockModel::new("a@1", false)))
        .unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    assert!(
        !store.search(query("alpha", "docs")).unwrap().is_empty(),
        "metadata already indexed must not suppress missing-vector backfill"
    );
}

#[test]
fn equal_dimension_model_change_rebuilds_or_errors() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "alpha policy\n\nbeta policy").unwrap();
    let index = temp.path().join("index");
    {
        let mut store = DocumentStore::new(&index, dimension())
            .unwrap()
            .with_embeddings(Box::new(MockModel::new("a@1", false)))
            .unwrap();
        store
            .index_collection("docs", &collection(&path), &chunks(256))
            .unwrap();
    }
    let opened = DocumentStore::new(&index, dimension())
        .unwrap()
        .with_embeddings(Box::new(MockModel::new("b@2", true)));
    let Ok(mut store) = opened else {
        return;
    };
    if store
        .index_collection("docs", &collection(&path), &chunks(256))
        .is_err()
    {
        return;
    }
    let hits = store.search(query("alpha", "docs")).unwrap();
    assert!(
        hits[0].content_preview.contains("alpha"),
        "query and corpus vectors must share model identity, not merely dimensions"
    );
}

#[test]
fn embedding_failure_can_resume_in_same_store() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "alpha policy").unwrap();
    let model = MockModel::new("a@1", false);
    model.fail_calls.store(1, Ordering::SeqCst);
    let mut store = DocumentStore::new(temp.path().join("index"), dimension())
        .unwrap()
        .with_embeddings(Box::new(model))
        .unwrap();
    assert!(
        store
            .index_collection("docs", &collection(&path), &chunks(256))
            .is_err()
    );
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    assert!(
        !store.search(query("alpha", "docs")).unwrap().is_empty(),
        "retry cannot mark the run complete while the committed chunk has no vector"
    );
}

#[test]
fn embedding_failure_then_reopen_does_not_duplicate_metadata() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "alpha policy").unwrap();
    let index = temp.path().join("index");
    {
        let model = MockModel::new("a@1", false);
        model.fail_calls.store(1, Ordering::SeqCst);
        let mut store = DocumentStore::new(&index, dimension())
            .unwrap()
            .with_embeddings(Box::new(model))
            .unwrap();
        assert!(
            store
                .index_collection("docs", &collection(&path), &chunks(256))
                .is_err()
        );
    }
    let mut store = DocumentStore::new(&index, dimension())
        .unwrap()
        .with_embeddings(Box::new(MockModel::new("a@1", false)))
        .unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    assert_eq!(
        store.collection_stats("docs").unwrap().chunk_count,
        1,
        "metadata committed before failed embeddings must be reconciled on reopen"
    );
}

#[test]
fn absent_lexical_terms_do_not_return_arbitrary_chunks() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "alpha policy").unwrap();
    let mut store = DocumentStore::new(temp.path().join("index"), dimension()).unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    assert!(
        store
            .search(query("missingliteralxyz", "docs"))
            .unwrap()
            .is_empty(),
        "embedding-free search must match query text"
    );
}

#[test]
fn heading_hierarchy_respects_source_order() {
    let source = "# Alpha\n\n## Beta\n\npayload";
    let result = HybridChunker::new().chunk(
        source,
        &ValidatedChunkingConfig::try_from(chunks(256)).unwrap(),
    );
    let payload = result
        .iter()
        .find(|chunk| chunk.content == "payload")
        .unwrap();
    assert_eq!(payload.heading_context, vec!["Alpha", "Beta"]);
}

#[test]
fn fenced_code_comments_do_not_become_document_headings() {
    let source = "preamble\n\n# Real\n\n```sh\n# bogus\n```\n\npayload";
    let result = HybridChunker::new().chunk(
        source,
        &ValidatedChunkingConfig::try_from(chunks(256)).unwrap(),
    );
    let payload = result
        .iter()
        .find(|chunk| chunk.content == "payload")
        .unwrap();
    assert_eq!(payload.heading_context, vec!["Real"]);
}

#[test]
fn chunk_ranges_reproduce_source_after_whitespace_merging() {
    let source = "first   \n\n\n  second";
    let config = ChunkingConfig {
        min_chunk_chars: 30,
        max_chunk_chars: 100,
        overlap_chars: 0,
        ..Default::default()
    };
    let result =
        HybridChunker::new().chunk(source, &ValidatedChunkingConfig::try_from(config).unwrap());
    for chunk in result {
        assert_eq!(
            source.get(chunk.byte_range.0..chunk.byte_range.1),
            Some(chunk.content.as_str()),
            "embedding content and cited source span must correspond exactly"
        );
    }
}

#[test]
fn unicode_case_expansion_does_not_panic_during_preview() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "İé needle").unwrap();
    let mut store = DocumentStore::new(temp.path().join("index"), dimension()).unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    let mut q = query("é", "docs");
    q.preview_config = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.search(q)));
    assert!(
        result.is_ok(),
        "Unicode lowercase byte offsets cannot index into the original string"
    );
}

#[test]
fn control_unchanged_index_reuses_embeddings_and_deletion_hides_hits() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "alpha policy").unwrap();
    let model = MockModel::new("a@1", false);
    let calls = model.calls.clone();
    let mut store = DocumentStore::new(temp.path().join("index"), dimension())
        .unwrap()
        .with_embeddings(Box::new(model))
        .unwrap();
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    store
        .index_collection("docs", &collection(&path), &chunks(256))
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.search(query("alpha", "docs")).unwrap().len(), 1);
    store.remove_file(&path).unwrap();
    assert!(store.search(query("alpha", "docs")).unwrap().is_empty());
}
