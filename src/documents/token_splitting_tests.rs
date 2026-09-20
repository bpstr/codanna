//! Token-budget document regressions use deterministic local vectors and inputs.
use super::*;
use crate::embedding_input::InputBudget;
use crate::vector::VectorError;
use std::fs;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

#[derive(Clone)]
struct BudgetModel {
    budget: InputBudget,
    requests: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicUsize>,
    fail_on_call: Arc<AtomicUsize>,
    invalid_range_mode: Arc<AtomicUsize>,
}

impl BudgetModel {
    fn new(limit: usize) -> Self {
        Self {
            budget: InputBudget::remote(Some(limit), None).unwrap(),
            requests: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(AtomicUsize::new(0)),
            fail_on_call: Arc::new(AtomicUsize::new(usize::MAX)),
            invalid_range_mode: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl EmbeddingGenerator for BudgetModel {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        self.budget
            .validate(texts.iter().copied())
            .map_err(VectorError::EmbeddingFailed)?;
        self.requests
            .lock()
            .unwrap()
            .extend(texts.iter().map(|text| (*text).to_string()));
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.fail_on_call.load(Ordering::SeqCst) {
            return Err(VectorError::EmbeddingFailed(
                "deterministic failure after an earlier embedding batch".into(),
            ));
        }
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }

    fn document_input_ranges(
        &self,
        prefix: &str,
        body: &str,
    ) -> Result<Vec<Range<usize>>, VectorError> {
        match self.invalid_range_mode.load(Ordering::SeqCst) {
            1 => return Ok(vec![0..1, 1..body.len()]),
            2 => return Ok(vec![0..4, 5..body.len()]),
            3 => return Ok(vec![0..5, 4..body.len()]),
            4 => return Ok(std::iter::once(0..body.len() - 1).collect()),
            5 => {
                return Ok(body
                    .char_indices()
                    .map(|(start, ch)| start..start + ch.len_utf8())
                    .collect());
            }
            _ => {}
        }
        self.budget
            .document_ranges(prefix, body)
            .map_err(VectorError::EmbeddingFailed)
    }

    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }

    fn cache_identity(&self) -> String {
        format!("document-budget-fixture@1:{}", self.budget.identity())
    }
}

fn open_budget_store(base: &Path, model: &BudgetModel) -> DocumentStore {
    DocumentStore::new(base, model.dimension())
        .unwrap()
        .with_embeddings(Box::new(model.clone()))
        .unwrap()
}

fn collection_for(path: &Path) -> CollectionConfig {
    CollectionConfig {
        paths: vec![path.to_path_buf()],
        ..Default::default()
    }
}

fn source_chunks() -> ChunkingConfig {
    ChunkingConfig {
        min_chunk_chars: 24,
        max_chunk_chars: 96,
        overlap_chars: 11,
        ..Default::default()
    }
}

fn stored_chunks(store: &DocumentStore, path: &Path, collection: &str) -> Vec<(u32, RawChunk)> {
    let query = SearchQuery {
        text: "fixture".into(),
        collection: Some(collection.into()),
        document: Some(path.to_path_buf()),
        limit: 10_000,
        preview_config: Some(super::super::config::SearchConfig {
            preview_mode: super::super::config::PreviewMode::Full,
            highlight: false,
            ..Default::default()
        }),
    };
    let candidates = store.get_filtered_candidates(&query).unwrap();
    let mut chunks: Vec<_> = store
        .build_search_results(candidates.into_iter().map(|id| (id, 1.0)).collect(), &query)
        .unwrap()
        .into_iter()
        .map(|hit| {
            (
                hit.chunk_id.get(),
                RawChunk::new(hit.byte_range, hit.content_preview, hit.heading_context),
            )
        })
        .collect();
    chunks.sort_unstable_by_key(|(id, _)| *id);
    chunks
}

fn coverage(source: &str, chunks: impl IntoIterator<Item = RawChunk>) -> Vec<usize> {
    let mut counts = vec![0; source.len()];
    for chunk in chunks {
        let (start, end) = chunk.byte_range;
        assert!(start < end, "empty evidence chunk");
        assert_eq!(source.get(start..end), Some(chunk.content.as_str()));
        for count in &mut counts[start..end] {
            *count += 1;
        }
    }
    counts
}

fn corpus() -> String {
    format!(
        "  # Root\r\n\r\n## Unicode\r\n\r\n{}\r\n\r\n## Code\r\n\r\n```rust\r\n// # a code comment\r\nlet values = [{}];\r\n```\r\n\r\n## Table\r\n\r\n| key | value |\r\n| --- | --- |\r\n{}\r\n## LongLine\r\n\r\n{}\t  ",
        "汉字かな🙂é e\u{301} ".repeat(35),
        "\"漢🙂\", ".repeat(30),
        "| 多言語 | 🙂保留 |\r\n".repeat(20),
        "0123456789abcdefghijklmnopqrstuvwxyz".repeat(30),
    )
}

#[test]
fn document_budget_splitting_preserves_source_bytes_headings_and_existing_overlap() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("corpus.md");
    let source = corpus();
    fs::write(&path, &source).unwrap();
    let config = source_chunks();
    let baseline = HybridChunker::new().chunk(&source, &config.clone().try_into().unwrap());
    let model = BudgetModel::new(64);
    let mut store = open_budget_store(&temp.path().join("index"), &model);

    let stats = store
        .index_collection("docs", &collection_for(&path), &config)
        .unwrap();

    let stored = stored_chunks(&store, &path, "docs");
    assert_eq!(stats.chunks_created, stored.len());
    assert!(
        stored.len() > baseline.len(),
        "the fixture must require splitting"
    );
    assert_eq!(
        coverage(&source, baseline.clone()),
        coverage(&source, stored.iter().map(|(_, chunk)| chunk.clone())),
        "refinement must preserve every source byte, including the original overlap multiplicity"
    );
    let requests: HashSet<_> = model.requests.lock().unwrap().iter().cloned().collect();
    assert!(!requests.is_empty());
    assert!(requests.iter().all(|input| input.len() <= 64));
    for (_, chunk) in &stored {
        assert!(baseline.iter().any(|original| {
            original.byte_range.0 <= chunk.byte_range.0
                && chunk.byte_range.1 <= original.byte_range.1
                && original.heading_context == chunk.heading_context
        }));
        let input = embedding_input(chunk);
        assert!(
            input.len() <= 64,
            "breadcrumbs must consume the same budget as the body"
        );
        assert!(
            requests.contains(&input),
            "stored evidence must be embedded in full"
        );
    }
}

#[test]
fn document_budget_splitting_matches_collection_and_watcher_and_skips_unchanged_files() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("guide.md");
    fs::write(&path, "# Original\n\nalpha original policy").unwrap();
    let collection_model = BudgetModel::new(64);
    let watcher_model = BudgetModel::new(64);
    let collection_base = temp.path().join("collection-index");
    let watcher_base = temp.path().join("watcher-index");
    let mut collection_store = open_budget_store(&collection_base, &collection_model);
    let mut watcher_store = open_budget_store(&watcher_base, &watcher_model);
    let config = source_chunks();
    for store in [&mut collection_store, &mut watcher_store] {
        store
            .index_collection("docs", &collection_for(&path), &config)
            .unwrap();
    }
    fs::write(&path, corpus()).unwrap();

    let collection_stats = collection_store
        .index_collection("docs", &collection_for(&path), &config)
        .unwrap();
    let watcher_count = watcher_store.reindex_file(&path, &config).unwrap().unwrap();

    assert_eq!(collection_stats.chunks_created, watcher_count);
    let collection_chunks = stored_chunks(&collection_store, &path, "docs");
    let watcher_chunks = stored_chunks(&watcher_store, &path, "docs");
    assert_eq!(
        collection_chunks
            .iter()
            .map(|(_, chunk)| chunk)
            .collect::<Vec<_>>(),
        watcher_chunks
            .iter()
            .map(|(_, chunk)| chunk)
            .collect::<Vec<_>>()
    );
    for (store, model) in [
        (&mut collection_store, &collection_model),
        (&mut watcher_store, &watcher_model),
    ] {
        let normalized_path = normalize_source_path(&path);
        let prior_ids = store.file_states[&normalized_path].chunk_ids.clone();
        let prior_calls = model.calls.load(Ordering::SeqCst);
        let skipped = store
            .index_collection("docs", &collection_for(&path), &config)
            .unwrap();
        assert_eq!(skipped.files_skipped, 1);
        assert_eq!(skipped.chunks_created, 0);
        assert_eq!(store.file_states[&normalized_path].chunk_ids, prior_ids);
        assert_eq!(model.calls.load(Ordering::SeqCst), prior_calls);
    }
    drop(collection_store);
    drop(watcher_store);
    assert_eq!(
        stored_chunks(
            &open_budget_store(&collection_base, &collection_model),
            &path,
            "docs"
        ),
        collection_chunks
    );
    assert_eq!(
        stored_chunks(
            &open_budget_store(&watcher_base, &watcher_model),
            &path,
            "docs"
        ),
        watcher_chunks
    );
}

#[derive(Debug, PartialEq)]
struct SourceSnapshot {
    chunks: Vec<(u32, RawChunk)>,
    chunk_ids: Vec<ChunkId>,
    content_hash: String,
    embedding_identity: Option<String>,
    generation: Option<String>,
    scores: Vec<(ChunkId, f32)>,
}

fn snapshot(store: &mut DocumentStore, path: &Path, collection: &str) -> SourceSnapshot {
    let path = normalize_source_path(path);
    let state = store.file_states[&path].clone();
    let mut scores = store
        .score_by_similarity(&state.chunk_ids, &[1.0, 0.0])
        .unwrap();
    scores.sort_unstable_by_key(|(id, _)| id.get());
    SourceSnapshot {
        chunks: stored_chunks(store, &path, collection),
        chunk_ids: state.chunk_ids.clone(),
        content_hash: state.content_hash.clone(),
        embedding_identity: store.embedded_files.get(&path).cloned(),
        generation: store.current_generation.clone(),
        scores,
    }
}

fn seed_collections(temp: &TempDir, model: &BudgetModel) -> (DocumentStore, PathBuf, PathBuf) {
    let path = temp.path().join("guide.md");
    let other = temp.path().join("untouched.md");
    fs::write(&path, "# Original\n\nalpha original policy").unwrap();
    fs::write(&other, "# Other\n\nalpha untouched source").unwrap();
    let mut store = open_budget_store(&temp.path().join("index"), model);
    for (name, source) in [("docs", &path), ("other", &other)] {
        store
            .index_collection(name, &collection_for(source), &source_chunks())
            .unwrap();
    }
    (store, path, other)
}

fn update_source(store: &mut DocumentStore, path: &Path, watcher: bool) -> StoreResult<()> {
    if watcher {
        store.reindex_file(path, &source_chunks()).map(|_| ())
    } else {
        store
            .index_collection("docs", &collection_for(path), &source_chunks())
            .map(|_| ())
    }
}

#[test]
fn document_budget_late_heading_failure_preserves_both_collections_and_reopen() {
    for watcher in [false, true] {
        let temp = TempDir::new().unwrap();
        let model = BudgetModel::new(64);
        let (mut store, path, other) = seed_collections(&temp, &model);
        let prior = snapshot(&mut store, &path, "docs");
        let untouched = snapshot(&mut store, &other, "other");
        let prior_calls = model.calls.load(Ordering::SeqCst);
        let replacement = format!(
            "{}\n\n# {}\n\nfinal evidence must remain intact",
            "valid replacement 漢字🙂 body\n\n".repeat(80),
            "impossible heading ancestry ".repeat(8),
        );
        fs::write(&path, replacement).unwrap();

        let error = update_source(&mut store, &path, watcher).unwrap_err();

        assert!(error.to_string().contains("budget"), "{error}");
        assert_eq!(model.calls.load(Ordering::SeqCst), prior_calls);
        assert_eq!(snapshot(&mut store, &path, "docs"), prior);
        assert_eq!(snapshot(&mut store, &other, "other"), untouched);
        drop(store);
        let mut reopened = open_budget_store(&temp.path().join("index"), &model);
        assert_eq!(snapshot(&mut reopened, &path, "docs"), prior);
        assert_eq!(snapshot(&mut reopened, &other, "other"), untouched);
    }
}

#[test]
fn document_budget_later_embedding_failure_rolls_back_splits_and_can_retry() {
    for watcher in [false, true] {
        let temp = TempDir::new().unwrap();
        let model = BudgetModel::new(64);
        let (mut store, path, other) = seed_collections(&temp, &model);
        let prior = snapshot(&mut store, &path, "docs");
        let untouched = snapshot(&mut store, &other, "other");
        let prior_calls = model.calls.load(Ordering::SeqCst);
        model.fail_on_call.store(prior_calls + 2, Ordering::SeqCst);
        let replacement = (0..180)
            .map(|index| {
                format!("replacement-{index:04} 漢字🙂 complete body with preserved evidence\n\n")
            })
            .collect::<String>();
        fs::write(&path, &replacement).unwrap();

        let error = update_source(&mut store, &path, watcher).unwrap_err();

        assert!(
            error.to_string().contains("deterministic failure"),
            "{error}"
        );
        assert_eq!(model.calls.load(Ordering::SeqCst), prior_calls + 2);
        assert_eq!(snapshot(&mut store, &path, "docs"), prior);
        assert_eq!(snapshot(&mut store, &other, "other"), untouched);
        drop(store);
        let mut reopened = open_budget_store(&temp.path().join("index"), &model);
        assert_eq!(snapshot(&mut reopened, &path, "docs"), prior);
        assert_eq!(snapshot(&mut reopened, &other, "other"), untouched);

        update_source(&mut reopened, &path, watcher).unwrap();

        let updated = stored_chunks(&reopened, &path, "docs");
        let baseline =
            HybridChunker::new().chunk(&replacement, &source_chunks().try_into().unwrap());
        assert_eq!(
            coverage(&replacement, baseline),
            coverage(&replacement, updated.into_iter().map(|(_, chunk)| chunk))
        );
        assert_eq!(stored_chunks(&reopened, &other, "other"), untouched.chunks);
        assert_eq!(
            reopened.file_states[&normalize_source_path(&path)].content_hash,
            calculate_hash(&replacement)
        );
        assert_eq!(reopened.embedding_diagnostics().unembedded_chunks, 0);
    }
}

#[test]
fn document_budget_invalid_generator_ranges_reject_without_publishing_partial_evidence() {
    for (mode, label) in [
        (1, "UTF-8 boundary"),
        (2, "gap"),
        (3, "overlap"),
        (4, "missing tail"),
    ] {
        for watcher in [false, true] {
            let temp = TempDir::new().unwrap();
            let model = BudgetModel::new(64);
            let (mut store, path, other) = seed_collections(&temp, &model);
            let prior = snapshot(&mut store, &path, "docs");
            let untouched = snapshot(&mut store, &other, "other");
            let prior_calls = model.calls.load(Ordering::SeqCst);
            fs::write(&path, "🙂complete replacement evidence").unwrap();
            model.invalid_range_mode.store(mode, Ordering::SeqCst);

            let error = update_source(&mut store, &path, watcher).unwrap_err();

            assert!(error.to_string().contains("ranges"), "{label}: {error}");
            assert_eq!(model.calls.load(Ordering::SeqCst), prior_calls, "{label}");
            assert_eq!(snapshot(&mut store, &path, "docs"), prior, "{label}");
            assert_eq!(snapshot(&mut store, &other, "other"), untouched, "{label}");
            drop(store);
            model.invalid_range_mode.store(0, Ordering::SeqCst);
            let mut reopened = open_budget_store(&temp.path().join("index"), &model);
            assert_eq!(snapshot(&mut reopened, &path, "docs"), prior, "{label}");
            assert_eq!(
                snapshot(&mut reopened, &other, "other"),
                untouched,
                "{label}"
            );
        }
    }
}

#[test]
fn document_budget_policy_migration_reprocesses_unchanged_source_once() {
    let temp = TempDir::new().unwrap();
    let model = BudgetModel::new(64);
    let (mut store, path, other) = seed_collections(&temp, &model);
    let config = source_chunks();
    let validated = config.clone().try_into().unwrap();
    let old_fingerprint = chunking_fingerprint(&validated, None).unwrap();
    let new_fingerprint = chunking_fingerprint(&validated, Some(&model)).unwrap();
    assert_ne!(old_fingerprint, new_fingerprint);
    let prior_chunks = stored_chunks(&store, &path, "docs");
    let untouched = stored_chunks(&store, &other, "other");
    let normalized_path = normalize_source_path(&path);
    let prior_hash = store.file_states[&normalized_path].content_hash.clone();
    let prior_calls = model.calls.load(Ordering::SeqCst);
    store
        .chunking_fingerprints
        .insert(normalized_path.clone(), old_fingerprint);

    let migrated = store
        .index_collection("docs", &collection_for(&path), &config)
        .unwrap();

    assert_eq!(migrated.files_processed, 1);
    assert_eq!(migrated.files_skipped, 0);
    assert_eq!(migrated.chunks_removed, prior_chunks.len());
    assert_eq!(migrated.chunks_created, prior_chunks.len());
    assert_eq!(store.file_states[&normalized_path].content_hash, prior_hash);
    assert_eq!(
        store.chunking_fingerprints[&normalized_path],
        new_fingerprint
    );
    assert_eq!(
        model.calls.load(Ordering::SeqCst),
        prior_calls,
        "identical inputs may reuse embeddings"
    );
    let migrated_chunks = stored_chunks(&store, &path, "docs");
    assert_ne!(migrated_chunks[0].0, prior_chunks[0].0);
    assert_eq!(
        migrated_chunks
            .iter()
            .map(|(_, chunk)| chunk)
            .collect::<Vec<_>>(),
        prior_chunks
            .iter()
            .map(|(_, chunk)| chunk)
            .collect::<Vec<_>>()
    );
    drop(store);
    let mut reopened = open_budget_store(&temp.path().join("index"), &model);

    let skipped = reopened
        .index_collection("docs", &collection_for(&path), &config)
        .unwrap();

    assert_eq!(skipped.files_processed, 0);
    assert_eq!(skipped.files_skipped, 1);
    assert_eq!(skipped.chunks_created, 0);
    assert_eq!(stored_chunks(&reopened, &path, "docs"), migrated_chunks);
    assert_eq!(stored_chunks(&reopened, &other, "other"), untouched);
    assert_eq!(model.calls.load(Ordering::SeqCst), prior_calls);
}

#[test]
fn document_budget_bounds_custom_generator_heading_amplification_before_cloning() {
    let temp = TempDir::new().unwrap();
    let model = BudgetModel::new(64);
    let (mut store, path, other) = seed_collections(&temp, &model);
    let prior = snapshot(&mut store, &path, "docs");
    let untouched = snapshot(&mut store, &other, "other");
    let prior_calls = model.calls.load(Ordering::SeqCst);
    model.invalid_range_mode.store(5, Ordering::SeqCst);
    fs::write(&path, format!("# {}\n\ncomplete body", "h".repeat(16_383))).unwrap();

    let error = update_source(&mut store, &path, false).unwrap_err();

    assert!(
        error.to_string().contains("output-size allowance"),
        "{error}"
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), prior_calls);
    assert_eq!(snapshot(&mut store, &path, "docs"), prior);
    assert_eq!(snapshot(&mut store, &other, "other"), untouched);
}

#[test]
fn document_budget_bounds_total_file_refinement_before_publication() {
    for (limit, source, expected) in [
        (262_144, format!("# {}", "h".repeat(262_118)), "64 MiB"),
        (1, "a".repeat(65_536), "65536-chunk"),
    ] {
        let temp = TempDir::new().unwrap();
        let model = BudgetModel::new(limit);
        let path = temp.path().join("guide.md");
        fs::write(&path, "a").unwrap();
        let mut store = open_budget_store(&temp.path().join("index"), &model);
        let config = ChunkingConfig::default();
        store
            .index_collection("docs", &collection_for(&path), &config)
            .unwrap();
        let prior = snapshot(&mut store, &path, "docs");
        let calls = model.calls.load(Ordering::SeqCst);
        fs::write(&path, source).unwrap();

        let error = store
            .index_collection("docs", &collection_for(&path), &config)
            .unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(model.calls.load(Ordering::SeqCst), calls);
        assert_eq!(snapshot(&mut store, &path, "docs"), prior);
        drop(store);
        let mut reopened = open_budget_store(&temp.path().join("index"), &model);
        assert_eq!(snapshot(&mut reopened, &path, "docs"), prior);
    }
}
