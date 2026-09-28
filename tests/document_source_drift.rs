//! Hash-based drift inspection does not mutate stored ingestion state.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore};
use codanna::vector::VectorDimension;
use std::collections::BTreeMap;

#[test]
fn source_drift_distinguishes_changed_missing_new_and_unchanged_without_ingestion() {
    let temp = tempfile::tempdir().unwrap();
    let unchanged = temp.path().join("a.md");
    let changed = temp.path().join("b.md");
    let missing = temp.path().join("c.md");
    let new = temp.path().join("d.md");
    for path in [&unchanged, &changed, &missing] {
        std::fs::write(path, "Original source policy").unwrap();
    }
    let index = temp.path().join("index");
    let mut store = DocumentStore::new(&index, VectorDimension::new(2).unwrap()).unwrap();
    store.index_collection("fixture", &CollectionConfig {
        paths: vec![unchanged.clone(), changed.clone(), missing.clone()], ..Default::default()
    }, &ChunkingConfig { min_chunk_chars: 1, ..Default::default() }).unwrap();
    std::fs::write(&changed, "Changed source policy").unwrap();
    std::fs::remove_file(&missing).unwrap();
    std::fs::write(&new, "New source policy").unwrap();
    let before = store.get_indexed_paths();
    let report = store.source_drift("fixture", &[new], 10, 4096);
    let statuses: BTreeMap<_, _> = report.files.iter()
        .map(|entry| (entry.path.file_name().unwrap().to_str().unwrap(), entry.status)).collect();
    assert_eq!(statuses, BTreeMap::from([
        ("a.md","unchanged"), ("b.md","changed"), ("c.md","missing"), ("d.md","new")
    ]));
    assert!(!report.truncated);
    assert_eq!(store.get_indexed_paths(), before);
    let bounded = store.source_drift("fixture", &[], 1, 1);
    assert!(bounded.truncated);
    assert_eq!(bounded.files.len(), 1);
    assert_eq!(bounded.files[0].status, "byte_budget_exceeded");
    let indexed_hash = report.files.iter().find(|entry| entry.status == "changed")
        .unwrap().indexed_sha256.clone();
    drop(store);
    let reopened = DocumentStore::new(&index, VectorDimension::new(2).unwrap()).unwrap();
    let report = reopened.source_drift("fixture", &[], 10, 4096);
    assert_eq!(report.files.iter().find(|entry| entry.status == "changed")
        .unwrap().indexed_sha256, indexed_hash);
}
