//! Deterministic storage, service, and graph regressions.
//! No providers, models, credentials, network calls, or production indexes.

use std::collections::HashSet;
use std::sync::Arc;

use codanna::indexing::facade::IndexFacade;
use codanna::indexing::pipeline::{ResolvedRelationship, stages::WriteStage};
use codanna::mcp::service::{FindSymbolTarget, find_dotted_members, resolve_find_symbol_target};
use codanna::storage::DocumentIndex;
use codanna::{FileId, Range, RelationKind, ScopeContext, Settings, Symbol, SymbolId, SymbolKind};

fn symbol(id: u32, name: &str, kind: SymbolKind) -> Symbol {
    Symbol::new(
        SymbolId::new(id).unwrap(),
        name,
        kind,
        FileId::new(1).unwrap(),
        Range::new(id - 1, 0, id - 1, 20),
    )
}

fn collision_index(count: u32) -> (tempfile::TempDir, DocumentIndex) {
    let temp = tempfile::tempdir().unwrap();
    let index = DocumentIndex::new(temp.path(), &Settings::default()).unwrap();
    index.start_batch().unwrap();
    for id in 1..=count {
        let method = symbol(id, "save", SymbolKind::Method)
            .with_scope(ScopeContext::ClassMember {
                class_name: Some(format!("Store{id:03}").into()),
            })
            .with_module_path("fixture");
        index.index_symbol(&method, "stores.py").unwrap();
    }
    index.commit_batch().unwrap();
    (temp, index)
}

#[test]
fn exact_name_lookup_reports_every_same_named_symbol() {
    let (_temp, index) = collision_index(101);
    assert_eq!(index.count_symbols().unwrap(), 101);
    let found = index.find_symbols_by_name("save", None).unwrap();
    println!("indexed=101; exact-name results={}", found.len());
    assert_eq!(
        found.len(),
        101,
        "exact lookup silently truncated a common name"
    );
}

#[test]
fn class_qualification_does_not_lose_a_symbol_after_global_candidate_limiting() {
    let (_temp, index) = collision_index(101);
    let missing: Vec<_> = (1..=101)
        .filter_map(|id| {
            let query = format!("Store{id:03}.save");
            let found = find_dotted_members(&query, |name| {
                index.find_symbols_by_name(name, None).unwrap()
            });
            found.is_empty().then_some(query)
        })
        .collect();
    println!("qualified indexed members reported missing: {missing:?}");
    assert!(
        missing.is_empty(),
        "qualification was applied after truncation: {missing:?}"
    );
}

#[test]
fn exact_code_fragment_survives_query_parser_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let index = DocumentIndex::new(temp.path(), &Settings::default()).unwrap();
    let fixture = symbol(1, "merge_items", SymbolKind::Function)
        .with_signature("fn merge_items(input: std::collections::HashMap<String, String>)");
    index.start_batch().unwrap();
    index.index_symbol(&fixture, "src/collections.rs").unwrap();
    index.commit_batch().unwrap();

    // Inspect the real Tantivy parser against the persisted Codanna schema.
    // This proves the fixture actually enters fallback, rather than guessing
    // which punctuation the parser accepts.
    let raw = tantivy::Index::open_in_dir(temp.path()).unwrap();
    raw.tokenizers().register(
        "ngram",
        tantivy::tokenizer::TextAnalyzer::builder(
            tantivy::tokenizer::NgramTokenizer::new(3, 10, false).unwrap(),
        )
        .build(),
    );
    let fields = ["name_text", "doc_comment", "signature", "context"]
        .into_iter()
        .map(|name| raw.schema().get_field(name).unwrap())
        .collect();
    let parser = tantivy::query::QueryParser::for_index(&raw, fields);
    let query = "std::collections::HashMap";
    let parse_error = parser.parse_query(query).err();
    println!("fragment={query}; Tantivy parser error={parse_error:?}");
    assert!(
        parse_error.is_some(),
        "update the fixture: it no longer exercises fallback"
    );
    assert_eq!(
        index.search("HashMap", 10, None, None, None).unwrap().len(),
        1
    );
    let found = index.search(query, 10, None, None, None).unwrap();
    println!(
        "bare type results=1; qualified code-fragment results={}",
        found.len()
    );
    assert_eq!(
        found.len(),
        1,
        "raw query text was not analyzed with the indexed field tokenizer"
    );
}

#[test]
fn impact_handles_a_diamond_cycle_and_self_recursion() {
    let temp = tempfile::tempdir().unwrap();
    let index = Arc::new(DocumentIndex::new(temp.path(), &Settings::default()).unwrap());
    index.start_batch().unwrap();
    for id in 1..=4 {
        index
            .index_symbol(
                &symbol(id, &format!("node{id}"), SymbolKind::Function),
                "graph.rs",
            )
            .unwrap();
    }
    let mut writer = WriteStage::new(index.clone());
    // 2 -> 1 <- 3, 4 -> 2, 4 -> 3, 1 -> 4 closes a cycle; 1 -> 1 is recursion.
    for (from, to) in [(2, 1), (3, 1), (4, 2), (4, 3), (1, 4), (1, 1)] {
        writer
            .write_one(ResolvedRelationship::new(
                SymbolId::new(from).unwrap(),
                SymbolId::new(to).unwrap(),
                RelationKind::Calls,
            ))
            .unwrap();
    }
    writer.flush().unwrap();
    let graph = index.graph_view();
    let first = graph.impact(SymbolId::new(1).unwrap(), 1, None).unwrap();
    assert_eq!(
        first.iter().map(|id| id.value()).collect::<HashSet<_>>(),
        HashSet::from([2, 3])
    );
    let all = graph.impact(SymbolId::new(1).unwrap(), 10, None).unwrap();
    assert_eq!(
        all.iter().map(|id| id.value()).collect::<HashSet<_>>(),
        HashSet::from([2, 3, 4])
    );
    assert_eq!(all.len(), 3, "diamond or cycle duplicated a node");
}

#[test]
fn dense_graph_preserves_complete_local_enumeration_and_reports_network_budget_failure() {
    let temp = tempfile::tempdir().unwrap();
    let index = Arc::new(DocumentIndex::new(temp.path(), &Settings::default()).unwrap());
    index.start_batch().unwrap();
    let mut writer = WriteStage::new(index.clone());
    for id in 1..=1002 {
        index
            .index_symbol(
                &symbol(id, &format!("node{id}"), SymbolKind::Function),
                "generated.rs",
            )
            .unwrap();
        if id > 1 {
            writer
                .write_one(ResolvedRelationship::new(
                    SymbolId::new(id).unwrap(),
                    SymbolId::new(1).unwrap(),
                    RelationKind::Calls,
                ))
                .unwrap();
        }
    }
    writer.flush().unwrap();
    let graph = index.graph_view();
    assert_eq!(
        graph
            .impact(SymbolId::new(1).unwrap(), 1, None)
            .unwrap()
            .len(),
        1001
    );
    assert!(
        graph
            .impact(SymbolId::new(1).unwrap(), 1, Some((1000, 20_000)))
            .is_err()
    );
    let page = graph
        .relationships_page(
            &[SymbolId::new(1).unwrap()],
            true,
            &[RelationKind::Calls],
            0,
            1000,
        )
        .unwrap();
    assert_eq!(page.total, 1001);
    assert_eq!(page.next_offset, Some(1000));
    let tail = graph
        .relationships_page(
            &[SymbolId::new(1).unwrap()],
            true,
            &[RelationKind::Calls],
            1000,
            1000,
        )
        .unwrap();
    assert_eq!(tail.edges.len(), 1);
}

#[test]
fn id_lookup_keeps_language_filter_consistent_with_name_lookup() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let facade = IndexFacade::new(Arc::new(settings)).unwrap();
    let mut rust_only = symbol(1, "rust_only", SymbolKind::Function);
    rust_only.language_id = Some(codanna::parsing::registry::LanguageId::new("rust"));
    let index = facade.document_index();
    index.start_batch().unwrap();
    index.index_symbol(&rust_only, "only.rs").unwrap();
    index.commit_batch().unwrap();
    let count = |target| match target {
        FindSymbolTarget::Symbols { symbols, .. } => symbols.len(),
        FindSymbolTarget::InvalidId(_) => panic!("valid ID rejected"),
    };
    assert_eq!(
        count(resolve_find_symbol_target(
            &facade,
            "rust_only",
            Some("python")
        )),
        0
    );
    assert_eq!(
        count(resolve_find_symbol_target(
            &facade,
            "symbol_id:1",
            Some("python")
        )),
        0,
        "symbol_id bypasses the requested language filter"
    );
}
