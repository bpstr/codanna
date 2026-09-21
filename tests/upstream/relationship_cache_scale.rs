use codanna::config::Settings;
use codanna::indexing::pipeline::types::SymbolLookupCache;
use codanna::storage::DocumentIndex;
use codanna::{FileId, Range, Symbol, SymbolId, SymbolKind};

fn populate_and_check(count: u32) {
    let dir = tempfile::tempdir().unwrap();
    let index = DocumentIndex::new(dir.path(), &Settings::default()).unwrap();
    index.start_batch().unwrap();
    for id in 1..=count {
        let symbol = Symbol::new(
            SymbolId::new(id).unwrap(),
            format!("generated_{id}"),
            SymbolKind::Function,
            FileId::new(1).unwrap(),
            Range::new(id, 0, id, 1),
        );
        index.add_document(&symbol, "generated.rs").unwrap();
    }
    index.commit_batch().unwrap();

    let empty = SymbolLookupCache::for_pending_relationships(&index, &[]).unwrap();
    assert!(
        empty.is_empty(),
        "an empty resolution pass must not hydrate even an existing corpus"
    );
    let cache = SymbolLookupCache::from_index(&index).unwrap();
    assert_eq!(cache.len(), count as usize);
    if let Some(last_id) = SymbolId::new(count) {
        assert!(cache.get(last_id).is_some());
    }
}

#[test]
fn hardening_symbol_cache_from_index_visits_every_symbol() {
    populate_and_check(0);
    populate_and_check(257);
}

#[test]
#[ignore = "writes one million synthetic symbols; run explicitly for the former cache limit"]
fn hardening_symbol_cache_exceeds_former_one_million_limit() {
    populate_and_check(1_000_001);
}
