//! Historical BM25 ablations must not use the production coverage order.
//! Drain these small fixtures completely, preserve their raw scores, then use
//! deterministic identity ties. This does not recreate Tantivy's DocAddress ties.

use codanna::indexing::facade::IndexFacade;
use codanna::storage::SearchResult;

pub fn raw_search(
    index: &IndexFacade,
    query: &str,
    limit: usize,
    language: Option<&str>,
) -> Vec<SearchResult> {
    let corpus = index.symbol_count().max(1);
    assert!(
        corpus <= 1000,
        "raw baseline requires a complete fixture drain"
    );
    let mut rows = index.search(query, corpus, None, None, language).unwrap();
    rows.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.symbol_id.0.cmp(&b.symbol_id.0))
    });
    rows.truncate(limit);
    rows
}
