//! Query-time relevance evidence selected by the frozen task ablations.
//! English morphology is a lexical heuristic, not semantic similarity. Existing
//! candidate generation, raw scores, index schema and embedding inputs stay intact.

use super::SearchResult;
use std::collections::BTreeSet;
use tantivy::tokenizer::{Language, LowerCaser, SimpleTokenizer, Stemmer, TextAnalyzer};

const MAX_QUERY_TERMS: usize = 32;
const MAX_QUERY_BYTES: usize = 4096;
const STOPWORDS: &[&str] = &[
    "and", "are", "for", "from", "how", "into", "not", "the", "this", "that", "to", "was", "were",
    "what", "where", "which", "with",
];

fn identifier_words(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut previous: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character.is_uppercase()
            && previous.is_some_and(|previous| {
                previous.is_lowercase()
                    || (previous.is_uppercase()
                        && chars.peek().is_some_and(|next| next.is_lowercase()))
            })
        {
            output.push(' ');
        }
        output.push(if character == '_' { ' ' } else { character });
        previous = Some(character);
    }
    output
}

fn normalized_words(text: &str) -> BTreeSet<String> {
    let separated = identifier_words(text);
    let mut analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(Stemmer::new(Language::English))
        .build();
    let mut stream = analyzer.token_stream(&separated);
    let mut words = BTreeSet::new();
    while stream.advance() {
        let word = &stream.token().text;
        if word.len() >= 3 && !STOPWORDS.contains(&word.as_str()) {
            words.insert(word.clone());
        }
    }
    words
}

pub(super) fn query_terms(query: &str) -> Option<Vec<String>> {
    // This budget bounds only the optional normalization pass. Oversized text
    // keeps the existing lexical query path rather than being silently clipped.
    if query.len() > MAX_QUERY_BYTES {
        return None;
    }
    // Keep identifier and explicit query-language behavior on the existing path.
    // Do not rewrite Boolean/field/phrase expressions or copied code fragments.
    if query.chars().any(|character| {
        matches!(
            character,
            ':' | '"'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '~'
                | '*'
                | '?'
                | '\\'
                | '/'
                | '^'
                | '+'
                | '-'
                | '<'
                | '>'
                | '!'
                | '|'
                | '&'
        )
    }) || query
        .split_whitespace()
        .any(|word| matches!(word, "AND" | "OR" | "NOT" | "IN"))
    {
        return None;
    }
    // Check eligibility before identifier splitting: one camelCase/snake_case
    // identifier must not start using discovery reranking just because it splits.
    // Deduplicate before budgeting so repeated words cannot hide a later concept.
    let original: BTreeSet<_> = query
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(str::to_lowercase)
        .filter(|word| word.len() >= 3 && !STOPWORDS.contains(&word.as_str()))
        .collect();
    if original.len() < 2 {
        return None;
    }
    let terms: Vec<_> = normalized_words(query)
        .into_iter()
        .take(MAX_QUERY_TERMS)
        .collect();
    (terms.len() >= 2).then_some(terms)
}

pub(super) fn coverage(result: &SearchResult, terms: &[String]) -> usize {
    let mut evidence = String::new();
    for field in [
        Some(result.name.as_str()),
        result.doc_comment.as_deref(),
        result.signature.as_deref(),
        result.context.as_deref(),
        Some(result.module_path.as_str()),
        Some(result.file_path.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        evidence.push(' ');
        evidence.push_str(field);
    }
    let words = normalized_words(&evidence);
    terms
        .iter()
        .filter(|term| words.contains(term.as_str()))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SymbolId, SymbolKind};

    fn row(name: &str, doc: &str, path: &str) -> SearchResult {
        SearchResult {
            symbol_id: SymbolId::new(1).unwrap(),
            name: name.into(),
            kind: SymbolKind::Function,
            file_path: path.into(),
            line: 1,
            column: 0,
            doc_comment: Some(doc.into()),
            signature: None,
            module_path: String::new(),
            language_id: Some("rust".into()),
            score: 1.0,
            highlights: Vec::new(),
            context: None,
        }
    }

    #[test]
    fn linguistic_coverage_matches_morphology_and_identifier_parts_not_substrings() {
        let terms = query_terms("vector namespaces").unwrap();
        assert_eq!(
            coverage(&row("vectorNamespace", "", "src/api.rs"), &terms),
            2
        );
        let terms = query_terms("raw response").unwrap();
        assert_eq!(
            coverage(&row("redraw", "response", "src/api.rs"), &terms),
            1
        );
        assert_eq!(
            coverage(
                &row("read_file", "read bytes", "src/recall.rs"),
                &query_terms("read recall").unwrap()
            ),
            2
        );
        assert!(
            normalized_words("HTTPServer read_file árvíz").is_superset(&BTreeSet::from([
                "http".into(),
                "server".into(),
                "read".into(),
                "file".into(),
                "árvíz".into(),
            ]))
        );
    }

    #[test]
    fn linguistic_coverage_preserves_identifier_and_explicit_syntax_paths() {
        for query in [
            "get_current_account",
            "ArchiveService",
            "ui",
            "calendar AND settings",
            "\"calendar settings\"",
            "doc_comment:calendar",
            "calendar -generic",
            "render<T> settings",
            "x IN [a b]",
            "alpha || beta",
            "alpha +beta",
        ] {
            assert!(
                query_terms(query).is_none(),
                "must bypass discovery: {query}"
            );
        }
        assert!(query_terms("calendar settings").is_some());
    }

    #[test]
    fn linguistic_coverage_deduplicates_inflections_and_bounds_query_terms() {
        assert_eq!(
            query_terms("vectors vector namespaces namespace"),
            query_terms("vector namespace")
        );
        let long = (0..100)
            .map(|i| format!("token{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(query_terms(&long).unwrap().len(), MAX_QUERY_TERMS);
        assert_eq!(
            coverage(
                &row("vector", "vector vector vectors", "vector.rs"),
                &query_terms("vector namespace").unwrap()
            ),
            1
        );
    }

    #[test]
    fn linguistic_coverage_bounds_query_bytes_without_losing_late_distinct_terms() {
        let repeated = format!("{}namespace", "vector ".repeat(40));
        assert_eq!(query_terms(&repeated), query_terms("vector namespace"));
        let oversized = "vector namespace ".repeat(MAX_QUERY_BYTES);
        assert!(query_terms(&oversized).is_none());
    }
}
