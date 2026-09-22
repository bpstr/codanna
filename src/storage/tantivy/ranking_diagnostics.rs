//! T05 diagnostic ablations on a small, deterministic, provider-free corpus.
//!
//! The assembly below deliberately mirrors query.rs's R0 lexical clauses. Each
//! runtime result's raw score is checked against this assembly so query drift
//! fails rather than silently producing a misleading explanation.

use super::DocumentIndex;
use crate::{FileId, Range, Settings, Symbol, SymbolId, SymbolKind};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeMap;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, FuzzyTermQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Value};
use tantivy::{DocAddress, Searcher, TantivyDocument, Term};

struct Fixture {
    _temp: tempfile::TempDir,
    index: DocumentIndex,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let index = DocumentIndex::new(temp.path(), &Settings::default()).unwrap();
        index.start_batch().unwrap();
        for (id, name, doc, signature, path) in [
            (
                1,
                "useAccountPresentation",
                "Calendar settings select the first weekday for an account.",
                "export function useAccountPresentation()",
                "src/calendar/presentation.ts",
            ),
            (
                2,
                "get_current_account",
                "Read effective account preferences.",
                "fn get_current_account() -> Account",
                "src/account.rs",
            ),
            (
                3,
                "ArchiveService",
                "Store a completed project.",
                "class ArchiveService",
                "src/archive.ts",
            ),
            (
                4,
                "render",
                "Render a typed value.",
                "fn render<T>(value: T) -> View",
                "src/render.rs",
            ),
        ] {
            add(&index, id, name, doc, signature, path);
        }
        for id in 10..58 {
            add(
                &index,
                id,
                "settings",
                "Generic application settings.",
                "const settings = {}",
                &format!("src/generic/{id}.ts"),
            );
        }
        index.commit_batch().unwrap();
        Self { _temp: temp, index }
    }
}

fn add(index: &DocumentIndex, id: u32, name: &str, doc: &str, signature: &str, path: &str) {
    let mut symbol = Symbol::new(
        SymbolId::new(id).unwrap(),
        name,
        SymbolKind::Function,
        FileId::new(id).unwrap(),
        Range::new(0, 0, 0, 50),
    );
    symbol.doc_comment = Some(doc.into());
    symbol.signature = Some(signature.into());
    index.index_symbol(&symbol, path).unwrap();
}

fn parsed(index: &DocumentIndex, fields: Vec<Field>, text: &str) -> Box<dyn Query> {
    let parser = QueryParser::for_index(&index.index, fields);
    parser.parse_query(text).unwrap_or_else(|_| {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        parser.parse_query(&format!("\"{escaped}\"")).unwrap()
    })
}

fn main_clause(index: &DocumentIndex, text: &str) -> Box<dyn Query> {
    parsed(
        index,
        vec![
            index.schema.name_text,
            index.schema.doc_comment,
            index.schema.signature,
            index.schema.context,
        ],
        text,
    )
}

fn fuzzy(index: &DocumentIndex, field: Field, text: &str) -> Box<dyn Query> {
    let _ = index;
    Box::new(FuzzyTermQuery::new(
        Term::from_field_text(field, text),
        1,
        true,
    ))
}

fn combined(index: &DocumentIndex, text: &str) -> BooleanQuery {
    BooleanQuery::new(vec![
        (
            Occur::Must,
            Box::new(BooleanQuery::new(vec![
                (Occur::Should, main_clause(index, text)),
                (Occur::Should, fuzzy(index, index.schema.name_text, text)),
                (Occur::Should, fuzzy(index, index.schema.name, text)),
            ])),
        ),
        (
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(index.schema.doc_type, "symbol"),
                IndexRecordOption::Basic,
            )),
        ),
    ])
}

fn rows(
    index: &DocumentIndex,
    searcher: &Searcher,
    query: &dyn Query,
) -> Vec<(u32, f32, DocAddress)> {
    searcher
        .search(query, &TopDocs::with_limit(200).order_by_score())
        .unwrap()
        .into_iter()
        .map(|(score, address)| {
            let doc: TantivyDocument = searcher.doc(address).unwrap();
            let id = doc
                .get_first(index.schema.symbol_id)
                .and_then(|value| value.as_u64())
                .unwrap();
            (u32::try_from(id).unwrap(), score, address)
        })
        .collect()
}

#[test]
fn ranking_diagnostics_measure_fields_fuzzy_clauses_and_score_trees() {
    let fixture = Fixture::new();
    let index = &fixture.index;
    let searcher = index.reader.searcher();
    let query = "calendar settings";
    let r0 = combined(index, query);
    let raw = rows(index, &searcher, &r0);
    let owner_rank = raw
        .iter()
        .position(|(id, _, _)| *id == 1)
        .map(|rank| rank + 1);
    assert!(
        owner_rank.is_some(),
        "owner must exist in the lexical candidate pool"
    );

    let components: Vec<(&str, Box<dyn Query>)> = vec![
        ("all_analyzed_fields", main_clause(index, query)),
        (
            "analyzed_name_only",
            parsed(index, vec![index.schema.name_text], query),
        ),
        (
            "documentation_only",
            parsed(index, vec![index.schema.doc_comment], query),
        ),
        (
            "signature_only",
            parsed(index, vec![index.schema.signature], query),
        ),
        (
            "whole_query_fuzzy_ngram",
            fuzzy(index, index.schema.name_text, query),
        ),
        (
            "whole_query_fuzzy_name",
            fuzzy(index, index.schema.name, query),
        ),
    ];
    let mut component_rows = Vec::new();
    for (label, component) in components {
        let results = rows(index, &searcher, component.as_ref());
        component_rows.push(json!({
            "clause": label,
            "matches": component.count(&searcher).unwrap(),
            "owner_rank": results.iter().position(|(id, _, _)| *id == 1).map(|rank| rank + 1),
            "top_ids": results.iter().take(5).map(|(id, _, _)| *id).collect::<Vec<_>>(),
        }));
    }
    let mut explained = Vec::new();
    for (id, score, address) in raw.iter().filter(|(id, _, _)| *id == 1 || *id == 10) {
        let explanation = r0.explain(&searcher, *address).unwrap();
        assert!((explanation.value() - score).abs() < 0.0001);
        explained.push(json!({
            "symbol_id": id, "raw_score": score,
            "explanation": serde_json::from_str::<JsonValue>(&explanation.to_pretty_json()).unwrap(),
        }));
    }
    assert_eq!(
        explained.len(),
        2,
        "both the owner and a generic decoy need explanations"
    );
    let selected = index.search(query, 5, None, None, None).unwrap();
    assert!(selected.iter().any(|result| result.symbol_id.value() == 1));
    assert_eq!(
        fuzzy(index, index.schema.name_text, query)
            .count(&searcher)
            .unwrap(),
        0
    );
    assert_eq!(
        fuzzy(index, index.schema.name, query)
            .count(&searcher)
            .unwrap(),
        0
    );
    println!(
        "ranking_clause_evidence={}",
        json!({
            "runtime_source_sha256": crate::indexing::calculate_hash(include_str!("query.rs")),
            "fixture_source_sha256": crate::indexing::calculate_hash(include_str!("ranking_diagnostics.rs")),
            "query": query, "r0_owner_rank": owner_rank,
            "final_owner_rank": selected.iter().position(|result| result.symbol_id.value() == 1).map(|rank| rank + 1),
            "clauses": component_rows, "explained": explained,
        })
    );
}

#[test]
fn ranking_diagnostics_preserve_raw_scores_across_query_families() {
    let fixture = Fixture::new();
    let index = &fixture.index;
    let searcher = index.reader.searcher();
    for (family, query) in [
        ("identifier", "useAccountPresentation"),
        ("identifier_typo", "ArchivService"),
        ("snake_case", "get_current_account"),
        ("camel_case", "ArchiveService"),
        ("topic", "calendar settings"),
        ("phrase", "\"calendar settings\""),
        ("boolean", "calendar AND settings"),
        ("exclusion", "calendar -generic"),
        ("field", "doc_comment:calendar"),
        ("short", "ui"),
        ("code_fragment", "render<T>"),
        ("absent", "qzvnoexist75391"),
    ] {
        let r0 = combined(index, query);
        let raw = rows(index, &searcher, &r0);
        let scores: BTreeMap<_, _> = raw.iter().map(|(id, score, _)| (*id, *score)).collect();
        let results = index.search(query, 5, None, None, None).unwrap();
        for result in &results {
            let raw_score = scores
                .get(&result.symbol_id.value())
                .expect("runtime returned a non-candidate");
            assert!(
                (result.score - raw_score).abs() < 0.0001,
                "score drift in {family}"
            );
        }
        println!(
            "ranking_query_family={}",
            json!({
                "family": family, "query": query, "candidate_count": raw.len(),
                "returned_ids": results.iter().map(|result| result.symbol_id.value()).collect::<Vec<_>>(),
            })
        );
    }
    assert_eq!(
        index.search("ArchivService", 1, None, None, None).unwrap()[0].name,
        "ArchiveService"
    );
    assert_eq!(
        index
            .search("get_current_account", 1, None, None, None)
            .unwrap()[0]
            .name,
        "get_current_account"
    );
    assert!(
        index
            .search("qzvnoexist75391", 5, None, None, None)
            .unwrap()
            .is_empty()
    );
}
