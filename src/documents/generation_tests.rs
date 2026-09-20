//! Publication tests use only local mock vectors. The subprocess hook exits
//! without destructors, so rollback code cannot make a broken commit look safe.
use super::*;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

struct FixedModel {
    calls: Arc<AtomicUsize>,
    fail_on_call: usize,
}

impl EmbeddingGenerator for FixedModel {
    fn generate_embeddings(
        &self,
        texts: &[&str],
    ) -> Result<Vec<Vec<f32>>, crate::vector::VectorError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.fail_on_call {
            return Err(crate::vector::VectorError::EmbeddingFailed(
                "fixed batch failure".into(),
            ));
        }
        Ok(texts
            .iter()
            .map(|text| {
                if text.contains("alpha") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }
    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }
    fn cache_identity(&self) -> String {
        "generation-fixture@1".into()
    }
}

fn open(base: &Path) -> DocumentStore {
    DocumentStore::new(base, VectorDimension::new(2).unwrap())
        .unwrap()
        .with_embeddings(Box::new(FixedModel {
            calls: Arc::new(AtomicUsize::new(0)),
            fail_on_call: usize::MAX,
        }))
        .unwrap()
}

fn config(source: &Path) -> CollectionConfig {
    CollectionConfig {
        paths: vec![source.to_path_buf()],
        ..Default::default()
    }
}

fn chunks() -> ChunkingConfig {
    ChunkingConfig {
        min_chunk_chars: 1,
        max_chunk_chars: 1024,
        overlap_chars: 0,
        ..Default::default()
    }
}

fn query(source: &Path) -> SearchQuery {
    SearchQuery {
        text: "alpha".into(),
        collection: Some("docs".into()),
        document: Some(source.to_path_buf()),
        limit: 1000,
        preview_config: Some(super::super::config::SearchConfig {
            preview_mode: super::super::config::PreviewMode::Full,
            highlight: false,
            ..Default::default()
        }),
    }
}

fn seed(temp: &TempDir) -> (PathBuf, PathBuf, DocumentStore) {
    let source = temp.path().join("guide.md");
    let other = temp.path().join("untouched.md");
    fs::write(&source, "alpha original policy").unwrap();
    fs::write(&other, "alpha untouched source").unwrap();
    let mut store = open(&temp.path().join("index"));
    store
        .index_collection("docs", &config(&source), &chunks())
        .unwrap();
    store
        .index_collection("other", &config(&other), &chunks())
        .unwrap();
    (source, other, store)
}

fn assert_snapshot(store: &mut DocumentStore, source: &Path, expected: Option<&str>) {
    let candidates = store.get_filtered_candidates(&query(source)).unwrap();
    let ids: HashSet<_> = candidates.iter().copied().collect();
    assert_eq!(
        ids.len(),
        candidates.len(),
        "one live chunk generation with unique IDs"
    );
    let hits = store
        .build_search_results(
            candidates.iter().map(|id| (*id, 0.0)).collect(),
            &query(source),
        )
        .unwrap();
    match expected {
        None => {
            assert!(hits.is_empty());
            assert!(store.get_file_collection(source).is_none());
        }
        Some(text) => {
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].content_preview, text);
            let state = store.file_states.get(source).unwrap();
            assert_eq!(state.content_hash, calculate_hash(text));
            assert_eq!(state.chunk_ids, candidates);
            let scores = store.score_by_similarity(&candidates, &[1.0, 0.0]).unwrap();
            assert_eq!(scores.len(), 1);
            assert_eq!(scores[0].1, if text.contains("alpha") { 1.0 } else { 0.0 });
        }
    }
    assert_eq!(store.collection_stats("other").unwrap().chunk_count, 1);
}

#[test]
fn crash_child() {
    let Ok(base) = std::env::var("CODANNA_TEST_DOCUMENT_BASE") else {
        return;
    };
    let base = PathBuf::from(base);
    let source = base.join("guide.md");
    let mut store = open(&base.join("index"));
    match std::env::var("CODANNA_TEST_DOCUMENT_ACTION")
        .unwrap()
        .as_str()
    {
        "update" => {
            store
                .index_collection("docs", &config(&source), &chunks())
                .unwrap();
        }
        "remove" => {
            store.remove_file(&source).unwrap();
        }
        "delete_collection" => {
            store.delete_collection("docs").unwrap();
        }
        action => panic!("unexpected child action {action}"),
    }
    panic!("requested publication boundary was not reached");
}

#[test]
fn process_termination_reopens_one_matching_generation_at_every_publication_boundary() {
    let boundaries = [
        ("before_vector_publication", false),
        ("after_vector_publication", false),
        ("before_metadata_commit", false),
        ("after_metadata_commit", true),
        ("before_state_publication", true),
        ("after_state_publication", true),
    ];
    for action in ["update", "remove", "delete_collection"] {
        let batch_boundary = (action == "update").then_some(("after_embedding_batch", false));
        for (boundary, committed) in boundaries.into_iter().chain(batch_boundary) {
            let temp = TempDir::new().unwrap();
            let (source, _, store) = seed(&temp);
            let old_id = store.file_states[&source].chunk_ids[0];
            drop(store);
            fs::write(&source, "beta replacement policy").unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "documents::store::generation_tests::crash_child",
                    "--nocapture",
                ])
                .env("CODANNA_TEST_DOCUMENT_BASE", temp.path())
                .env("CODANNA_TEST_DOCUMENT_ACTION", action)
                .env("CODANNA_TEST_DOCUMENT_CRASH", boundary)
                .output()
                .unwrap();
            assert_eq!(
                status.status.code(),
                Some(86),
                "{action} {boundary}: {}",
                String::from_utf8_lossy(&status.stderr)
            );
            let index = temp.path().join("index");
            let mut reopened = open(&index);
            let expected = if !committed {
                Some("alpha original policy")
            } else if action == "update" {
                Some("beta replacement policy")
            } else {
                None
            };
            assert_snapshot(&mut reopened, &source, expected);
            // Retry must be safe regardless of which side of publication died.
            reopened
                .index_collection("docs", &config(&source), &chunks())
                .unwrap();
            assert_snapshot(&mut reopened, &source, Some("beta replacement policy"));
            assert!(reopened.file_states[&source].chunk_ids[0].get() > old_id.get());
            let stats = reopened.embedding_diagnostics();
            assert!(stats.physical_vectors <= stats.live_vectors * 2);
            assert_eq!(fs::read_dir(index.join("generations")).unwrap().count(), 1);
            assert!(!fs::read_dir(&index).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".document-spool-")
            }));
        }
    }
}

#[test]
fn failed_second_embedding_batch_keeps_generation_and_reserves_ids_across_reopen() {
    let temp = TempDir::new().unwrap();
    let (source, _, mut store) = seed(&temp);
    store.embedding_generator = Some(Arc::new(FixedModel {
        calls: Arc::new(AtomicUsize::new(0)),
        fail_on_call: 2,
    }));
    let original = store.current_generation.clone();
    let replacement: String = (0..140)
        .map(|n| format!("beta unique paragraph {n:03} with evidence.\n\n"))
        .collect();
    fs::write(&source, replacement).unwrap();
    let small = ChunkingConfig {
        max_chunk_chars: 44,
        ..chunks()
    };
    assert!(
        store
            .index_collection("docs", &config(&source), &small)
            .is_err()
    );
    assert_eq!(store.current_generation, original);
    assert_snapshot(&mut store, &source, Some("alpha original policy"));
    let next_after_failure = store.next_chunk_id;
    drop(store);
    let mut reopened = open(&temp.path().join("index"));
    assert_snapshot(&mut reopened, &source, Some("alpha original policy"));
    assert!(reopened.next_chunk_id >= next_after_failure);
    reopened
        .index_collection("docs", &config(&source), &small)
        .unwrap();
    let ids = &reopened.file_states[&source].chunk_ids;
    assert!(ids.len() > 64);
    assert!(ids.iter().all(|id| id.get() as u64 >= next_after_failure));
    assert_eq!(reopened.embedding_diagnostics().unembedded_chunks, 0);
}

#[test]
fn compaction_bounds_one_hundred_updates_and_preserves_pinned_queries() {
    let temp = TempDir::new().unwrap();
    let (source, _, mut store) = seed(&temp);
    let mut pinned = store.query_snapshot();
    let mut copied_bytes = 0;
    let mut added_bytes = 0;
    let mut no_copy_updates = 0;
    for cycle in 0..100 {
        fs::write(&source, format!("beta revision {cycle}")).unwrap();
        store
            .index_collection("docs", &config(&source), &chunks())
            .unwrap();
        let stats = store.embedding_diagnostics();
        assert_eq!(stats.live_vectors, 2);
        assert!(stats.physical_vectors <= 4);
        assert!(stats.vector_segments <= 8);
        assert_eq!(stats.new_vector_bytes, 12);
        copied_bytes += stats.compacted_vector_bytes;
        added_bytes += stats.new_vector_bytes;
        no_copy_updates += usize::from(stats.compacted_vector_bytes == 0);
        assert_snapshot(&mut store, &source, Some(&format!("beta revision {cycle}")));
        let previous = pinned.search(query(&source)).unwrap();
        assert_eq!(previous.len(), 1);
        assert_eq!(previous[0].content_preview, "alpha original policy");
        assert_eq!(previous[0].similarity, 1.0);
    }
    assert!(
        no_copy_updates >= 60,
        "ordinary edits append only their new vectors"
    );
    assert!(copied_bytes <= 1200);
    assert_eq!(added_bytes, 1200);
    eprintln!(
        "document vector churn: cycles=100 live=2 physical={} new_bytes={added_bytes} copied_bytes={copied_bytes} updates_without_copy={no_copy_updates}",
        store.embedding_diagnostics().physical_vectors
    );
    drop(store);
    let mut reopened = open(&temp.path().join("index"));
    assert_snapshot(&mut reopened, &source, Some("beta revision 99"));
    // The old query can outlive both the store and its compacted on-disk files.
    assert_eq!(
        pinned.search(query(&source)).unwrap()[0].content_preview,
        "alpha original policy"
    );
}

#[test]
fn committed_generation_repairs_missing_and_stale_state_mirrors() {
    let temp = TempDir::new().unwrap();
    let (source, _, mut store) = seed(&temp);
    let index = temp.path().join("index");
    let old = fs::read(index.join("state.json")).unwrap();
    fs::write(&source, "beta replacement policy").unwrap();
    store
        .index_collection("docs", &config(&source), &chunks())
        .unwrap();
    drop(store);
    for state in [Some(old), None] {
        if let Some(old) = state {
            fs::write(index.join("state.json"), old).unwrap();
        } else {
            fs::remove_file(index.join("state.json")).unwrap();
        }
        let mut reopened = open(&index);
        assert_snapshot(&mut reopened, &source, Some("beta replacement policy"));
        let mirror: PersistedState = DocumentStore::load_state(&index.join("state.json")).unwrap();
        assert_eq!(
            mirror.file_states[&source.to_string_lossy().to_string()].content_hash,
            calculate_hash("beta replacement policy")
        );
    }
}

#[test]
fn failed_state_mirror_publication_preserves_committed_results_and_safe_retry() {
    let temp = TempDir::new().unwrap();
    let (source, _, mut store) = seed(&temp);
    let index = temp.path().join("index");
    let mirror = index.join("state.json");
    fs::remove_file(&mirror).unwrap();
    fs::create_dir(&mirror).unwrap();
    fs::write(&source, "beta replacement policy").unwrap();
    assert!(
        store
            .index_collection("docs", &config(&source), &chunks())
            .is_err()
    );
    assert_snapshot(&mut store, &source, Some("beta replacement policy"));
    drop(store);
    // The optional mirror must not make an otherwise complete committed
    // generation unavailable for queries.
    let mut reopened = open(&index);
    assert_snapshot(&mut reopened, &source, Some("beta replacement policy"));
    fs::remove_dir(mirror).unwrap();
    let retry = reopened
        .index_collection("docs", &config(&source), &chunks())
        .unwrap();
    assert_eq!(retry.files_skipped, 1);
    assert_snapshot(&mut reopened, &source, Some("beta replacement policy"));
}

#[test]
fn opening_committed_queries_does_not_wait_for_embedding_work_or_collect_its_staging() {
    let temp = TempDir::new().unwrap();
    let (source, _, store) = seed(&temp);
    let index = temp.path().join("index");
    let _build_lock = generation::lock(&index).unwrap();
    let staging = StagedVectors::new(&index, VectorDimension::new(2).unwrap()).unwrap();
    // The constructor takes only the brief publication lock and leaves another
    // writer's unpublished segment alone while its build lock is held.
    let mut concurrent = open(&index);
    assert_snapshot(&mut concurrent, &source, Some("alpha original policy"));
    drop(staging);
    drop(store);
}

#[test]
fn rejects_generation_traversal_duplicates_and_oversized_reservations() {
    let temp = TempDir::new().unwrap();
    assert!(
        generation::load(temp.path(), Some("codanna-documents-v1:../../outside.json")).is_err()
    );
    let (_, _, store) = seed(&temp);
    let index = temp.path().join("index");
    let name = store.current_generation.clone().unwrap();
    drop(store);
    let path = index.join("generations").join(name);
    let original = fs::read(&path).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let duplicate = value["vectors"][0].clone();
    value["vectors"].as_array_mut().unwrap().push(duplicate);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(DocumentStore::new(&index, VectorDimension::new(2).unwrap()).is_err());
    fs::write(path, original).unwrap();
    fs::write(index.join("next-chunk-id.json"), "1".repeat(100)).unwrap();
    assert!(DocumentStore::new(&index, VectorDimension::new(2).unwrap()).is_err());
}

#[test]
fn missing_vector_ids_are_reported_even_when_obsolete_records_hide_the_count_gap() {
    let temp = TempDir::new().unwrap();
    let (source, other, store) = seed(&temp);
    let index = temp.path().join("index");
    let first_segment = store.vector_storage.as_ref().unwrap().names()[0].clone();
    let other_id = store.file_states[&other].chunk_ids[0].get();
    drop(store);
    let path = index
        .join("vectors")
        .join(first_segment)
        .join("segment_0.vec");
    let mut bytes = fs::read(&path).unwrap();
    // Keep the physical count and valid format intact, but replace the original
    // source's vector ID with an already-present ID from the other source.
    bytes[16..20].copy_from_slice(&other_id.to_le_bytes());
    fs::write(path, bytes).unwrap();
    let mut reopened = open(&index);
    assert_eq!(reopened.embedding_diagnostics().physical_vectors, 2);
    assert_eq!(reopened.embedding_diagnostics().unembedded_chunks, 2);
    assert_eq!(
        reopened
            .index_collection("docs", &config(&source), &chunks())
            .unwrap()
            .files_processed,
        1
    );
    assert_snapshot(&mut reopened, &source, Some("alpha original policy"));
}

#[test]
fn corrupt_nonfinite_vector_is_rejected_instead_of_ranked() {
    let temp = TempDir::new().unwrap();
    let (source, _, store) = seed(&temp);
    let index = temp.path().join("index");
    let first_segment = store.vector_storage.as_ref().unwrap().names()[0].clone();
    drop(store);
    let path = index
        .join("vectors")
        .join(first_segment)
        .join("segment_0.vec");
    let mut bytes = fs::read(&path).unwrap();
    bytes[20..24].copy_from_slice(&f32::NAN.to_le_bytes());
    fs::write(path, bytes).unwrap();
    let mut reopened = open(&index);
    let error = reopened.search(query(&source)).unwrap_err();
    assert!(error.to_string().contains("Non-finite"));
}

#[test]
fn document_store_excludes_its_own_files_from_broad_and_explicit_sources() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("intended.json");
    fs::write(&source, r#"{"topic":"alpha intended source"}"#).unwrap();
    let index = temp.path().join("document-store");
    let mut store = open(&index);
    let managed = index.join("managed-metadata.json");
    fs::write(&managed, r#"{"internal":"managed metadata"}"#).unwrap();
    let collection = CollectionConfig {
        paths: vec![temp.path().to_path_buf(), managed.clone()],
        patterns: vec!["**/*.json".into()],
        ..Default::default()
    };
    for scan in 0..3 {
        fs::write(&managed, format!(r#"{{"internal":"revision {scan}"}}"#)).unwrap();
        let stats = store
            .index_collection("docs", &collection, &chunks())
            .unwrap();
        assert_eq!(stats.files_processed, usize::from(scan == 0));
        assert_eq!(stats.files_skipped, usize::from(scan != 0));
        assert_eq!(
            store.get_indexed_paths(),
            vec![source.canonicalize().unwrap()]
        );
        assert_eq!(store.collection_stats("docs").unwrap().chunk_count, 1);
    }
}

#[test]
fn configured_custom_index_metadata_is_never_a_document_source() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("intended.json");
    fs::write(&source, r#"{"topic":"alpha intended source"}"#).unwrap();
    let index = temp.path().join("custom-index-without-ignore-rule");
    fs::create_dir_all(index.join("code/tantivy")).unwrap();
    let code_metadata = index.join("code/tantivy/meta.json");
    fs::write(&code_metadata, r#"{"internal":"code index metadata"}"#).unwrap();
    let mut settings = crate::config::Settings {
        index_path: index.clone(),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let mut store = super::super::open_from_settings(&settings).unwrap();
    let collection = CollectionConfig {
        paths: vec![
            temp.path().to_path_buf(),
            code_metadata.clone(),
            store.base_path.clone(),
        ],
        patterns: vec!["**/*.json".into()],
        ..Default::default()
    };
    for scan in 0..3 {
        fs::write(
            &code_metadata,
            format!(r#"{{"internal":"revision {scan}"}}"#),
        )
        .unwrap();
        let stats = store
            .index_collection("docs", &collection, &chunks())
            .unwrap();
        assert_eq!(stats.files_processed, usize::from(scan == 0));
        assert_eq!(stats.files_skipped, usize::from(scan != 0));
        assert_eq!(
            store.get_indexed_paths(),
            vec![source.canonicalize().unwrap()]
        );
        assert_eq!(store.collection_stats("docs").unwrap().chunk_count, 1);
    }
    let snapshot = store.query_snapshot();
    assert_eq!(
        snapshot.0.source_exclusion.as_deref(),
        Some(index.canonicalize().unwrap().as_path())
    );
    drop(store);
    let mut reopened = super::super::open_from_settings(&settings).unwrap();
    let unchanged = reopened
        .index_collection("docs", &collection, &chunks())
        .unwrap();
    assert_eq!(unchanged.files_processed, 0);
    assert_eq!(unchanged.files_skipped, 1);
    assert_eq!(
        reopened.get_indexed_paths(),
        vec![source.canonicalize().unwrap()]
    );
}
