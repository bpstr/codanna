//! Memoize the expensive coverage key without changing ranking or raw scores.

use super::SearchResult;
use std::cmp::Ordering;

fn tie_break(left: &SearchResult, right: &SearchResult) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.file_path.cmp(&right.file_path))
        .then_with(|| left.line.cmp(&right.line))
        .then_with(|| left.column.cmp(&right.column))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.symbol_id.value().cmp(&right.symbol_id.value()))
}

pub(super) fn sort_coverage_once(
    results: &mut Vec<SearchResult>,
    limit: usize,
    coverage: impl FnMut(&SearchResult) -> usize,
) {
    // Compute before moving any results: a failed key calculation must not
    // discard caller-owned evidence. Only scalar keys and moved rows are stored;
    // documentation, signatures, paths and names are never cloned here.
    let keys: Vec<_> = results.iter().map(coverage).collect();
    let mut ranked: Vec<_> = results.drain(..).zip(keys).collect();
    ranked.sort_by(|(left, left_key), (right, right_key)| {
        right_key.cmp(left_key).then_with(|| tie_break(left, right))
    });
    results.extend(ranked.into_iter().take(limit).map(|(row, _)| row));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SymbolId, SymbolKind};
    use std::cell::Cell;

    fn rows() -> Vec<SearchResult> {
        (0..128)
            .map(|slot| {
                let id = (slot * 37) % 128 + 1;
                SearchResult {
                    symbol_id: SymbolId::new(id).unwrap(),
                    name: format!("owner{}", id % 5),
                    kind: SymbolKind::Function,
                    file_path: format!("src/{}.rs", id % 3),
                    line: id % 7 + 1,
                    column: id % 4,
                    doc_comment: Some("calendar preferences ".repeat(200)),
                    signature: Some("fn owner()".into()),
                    module_path: "src".into(),
                    language_id: Some("rust".into()),
                    score: (id % 11) as f32 / 11.0,
                    highlights: Vec::new(),
                    context: None,
                }
            })
            .collect()
    }

    #[test]
    fn ranking_efficiency_preserves_complete_order_with_one_key_per_candidate() {
        let mut old = rows();
        let mut new = old.clone();
        let old_calls = Cell::new(0usize);
        let new_calls = Cell::new(0usize);
        let key = |row: &SearchResult| row.symbol_id.value() as usize % 4;
        old.sort_by(|left, right| {
            old_calls.set(old_calls.get() + 2);
            key(right)
                .cmp(&key(left))
                .then_with(|| tie_break(left, right))
        });
        sort_coverage_once(&mut new, 128, |row| {
            new_calls.set(new_calls.get() + 1);
            key(row)
        });
        assert_eq!(
            serde_json::to_value(&new).unwrap(),
            serde_json::to_value(&old).unwrap()
        );
        assert_eq!(new_calls.get(), 128);
        assert!(old_calls.get() > new_calls.get());
        println!(
            "ranking_key_calls: candidates=128 before={} after={}",
            old_calls.get(),
            new_calls.get()
        );
        sort_coverage_once(&mut new, 5, key);
        assert_eq!(
            serde_json::to_value(new).unwrap(),
            serde_json::to_value(&old[..5]).unwrap()
        );
    }

    #[test]
    fn ranking_efficiency_retains_total_float_order_and_original_score_bits() {
        let mut items = rows();
        items.truncate(6);
        for (row, score) in
            items
                .iter_mut()
                .zip([f32::NAN, f32::NEG_INFINITY, -0.0, 0.0, 1.0, f32::INFINITY])
        {
            row.score = score;
        }
        let mut expected = items.clone();
        expected.sort_by(tie_break);
        sort_coverage_once(&mut items, 20, |_| 1);
        assert_eq!(
            items
                .iter()
                .map(|row| (row.symbol_id, row.score.to_bits()))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|row| (row.symbol_id, row.score.to_bits()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ranking_efficiency_does_not_drop_results_when_key_extraction_panics() {
        let mut items = rows();
        let before = serde_json::to_value(&items).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sort_coverage_once(&mut items, 5, |_| panic!("synthetic key failure"));
        }));
        assert!(result.is_err());
        assert_eq!(serde_json::to_value(&items).unwrap(), before);
        let mut empty = Vec::new();
        sort_coverage_once(&mut empty, 5, |_| panic!("no keys expected"));
        assert!(empty.is_empty());
    }
}
