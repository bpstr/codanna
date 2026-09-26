//! Deterministic cache-policy experiments, not predictions of real provider cost.
//! Exercises the production cache with fixed vectors and explicit scan ordering.
use crate::embedding_cache::EmbeddingCache;
use std::sync::Arc;

fn corpus(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("synthetic input {index}"))
        .collect()
}

fn populated(inputs: &[String], dimension: usize) -> EmbeddingCache {
    let mut cache = EmbeddingCache::empty("pressure-fixture", dimension);
    let vector: Arc<[f32]> = Arc::from(vec![0.0; dimension]);
    for input in inputs {
        assert!(cache.insert(input, Arc::clone(&vector)));
    }
    cache
}

fn scan(mut cache: EmbeddingCache, inputs: &[String], admit_misses: bool) -> usize {
    let vector: Arc<[f32]> = Arc::from(vec![1.0, 0.0]);
    let mut hits = 0;
    // Look up a complete 64-item batch before inserting its newly generated
    // vectors, matching the relevant cache-lookup/admission ordering.
    for batch in inputs.chunks(64) {
        let missing: Vec<_> = batch
            .iter()
            .filter(|input| cache.get(input).is_none())
            .collect();
        hits += batch.len() - missing.len();
        if admit_misses {
            for input in missing {
                assert!(cache.insert(input, Arc::clone(&vector)));
            }
        }
    }
    hits
}

#[test]
fn snapshot_matches_can_be_lost_to_sequential_admission() {
    let inputs = corpus(4500);
    let cache = populated(&inputs, 2);
    let snapshot_hits = inputs
        .iter()
        .filter(|input| cache.get(input).is_some())
        .count();
    let fifo_hits = scan(cache.clone(), &inputs, true);
    let pinned_experiment_hits = scan(cache.clone(), &inputs, false);
    let reversed: Vec<_> = inputs.iter().rev().cloned().collect();
    let reverse_hits = scan(cache, &reversed, true);
    println!(
        "inputs=4500 snapshot_hits={snapshot_hits} fifo_batch64_hits={fifo_hits} no_admission_experiment_hits={pinned_experiment_hits} reverse_scan_hits={reverse_hits}"
    );
    assert_eq!(snapshot_hits, 4096);
    assert_eq!(
        fifo_hits, 0,
        "sequential misses evict not-yet-consumed resident inputs"
    );
    assert_eq!(pinned_experiment_hits, 4096);
    assert_eq!(reverse_hits, 4096);
    // No-admission is an experiment only: it fails to learn new inputs for a
    // later rebuild. This test does not silently select it as runtime policy.
}

#[test]
fn cache_byte_budget_can_bind_before_the_entry_count_limit() {
    let inputs = corpus(1400);
    let cache = populated(&inputs, 3072);
    let resident = inputs
        .iter()
        .filter(|input| cache.get(input).is_some())
        .count();
    println!("inputs=1400 dimension=3072 resident={resident}");
    // Existing 16 MiB accounting includes 256 bytes of per-entry overhead.
    // This is a logical budget assertion, not a measured process RSS claim.
    assert_eq!(resident, 1337);
    assert!(resident < 4096);
}

#[test]
fn persisted_cache_keeps_the_same_pressure_boundary() {
    let inputs = corpus(4500);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("embedding-cache.json");
    populated(&inputs, 2).save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let cache = EmbeddingCache::load(&path, "pressure-fixture", 2);
    assert_eq!(
        inputs
            .iter()
            .filter(|input| cache.get(input).is_some())
            .count(),
        4096
    );
    assert_eq!(scan(cache, &inputs, true), 0);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "simulation must not rewrite the saved cache"
    );
}
