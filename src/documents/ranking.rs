//! Bounded coverage ranking and source diversity.
//! Lexical candidate retrieval still requires a literal hit; semantic candidates
//! are constrained by the caller's original cosine cutoff and positive margin.
use std::collections::{BTreeSet, HashMap, HashSet};

use tantivy::tokenizer::TextAnalyzer;

use super::store::SearchResult;

fn analyzed(text: &str, analyzer: &mut TextAnalyzer) -> BTreeSet<String> {
    let mut stream = analyzer.token_stream(text);
    let mut terms = BTreeSet::new();
    while stream.advance() {
        terms.insert(stream.token().text.clone());
    }
    terms
}

/// Coverage precedes raw BM25 so a repeated heading matching one query term
/// cannot crowd out evidence addressing several terms. English stemming is only
/// a secondary ranking signal over candidates with an exact analyzed term hit.
pub(super) struct LexicalCoverage {
    literal: TextAnalyzer,
    stemmed: TextAnalyzer,
    query: BTreeSet<String>,
    stemmed_query: BTreeSet<String>,
}

impl LexicalCoverage {
    pub(super) fn new(text: &str, mut literal: TextAnalyzer, mut stemmed: TextAnalyzer) -> Self {
        let query = analyzed(text, &mut literal);
        let stemmed_query = analyzed(text, &mut stemmed);
        Self {
            literal,
            stemmed,
            query,
            stemmed_query,
        }
    }

    pub(super) fn score(&mut self, text: &str) -> (usize, usize) {
        let literal = analyzed(text, &mut self.literal);
        let stemmed = analyzed(text, &mut self.stemmed);
        (
            self.query.intersection(&literal).count(),
            self.stemmed_query.intersection(&stemmed).count(),
        )
    }
}

/// Preserve relevance ordering while preventing repeated sources and sections
/// from exhausting the budget. Relax quotas only to fill a sparse result set.
pub(super) fn diversify(results: Vec<SearchResult>, limit: usize) -> Vec<SearchResult> {
    if limit == 0 {
        return Vec::new();
    }
    let per_source = (limit / 2).max(1);
    let mut selected = Vec::with_capacity(limit.min(results.len()));
    let mut retained = HashSet::new();
    let mut sources = HashMap::new();
    let mut sections = HashSet::new();
    for pass in 0..3 {
        for (index, result) in results.iter().enumerate() {
            if retained.contains(&index) {
                continue;
            }
            let source = (&result.collection, &result.source_path);
            let section = (source, &result.heading_context);
            if pass < 2 && sources.get(&source).copied().unwrap_or(0) >= per_source {
                continue;
            }
            if pass == 0 && sections.contains(&section) {
                continue;
            }
            retained.insert(index);
            *sources.entry(source).or_insert(0) += 1;
            sections.insert(section);
            selected.push(index);
            if selected.len() == limit {
                break;
            }
        }
        if selected.len() == limit {
            break;
        }
    }
    // Move previews instead of duplicating all candidate text.
    let mut results: Vec<_> = results.into_iter().map(Some).collect();
    selected
        .into_iter()
        .filter_map(|index| results[index].take())
        .collect()
}
