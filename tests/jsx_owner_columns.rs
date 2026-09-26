//! Column-aware source ownership for same-named JSX owners and raw relationships.
//! Tests use deterministic synthetic data without semantic indexing or providers.

use codanna::indexing::facade::IndexFacade;
use codanna::indexing::pipeline::stages::collect::CollectStage;
use codanna::indexing::pipeline::{ParsedFile, RawRelationship, RawSymbol};
use codanna::parsing::LanguageId;
use codanna::{Range, RelationKind, Settings, SymbolKind};
use crossbeam_channel::bounded;
use std::collections::HashMap;
use std::sync::Arc;

fn collected_owners(declarations: &[Range], sites: &[Range]) -> Vec<Range> {
    let parsed = ParsedFile {
        path: "same_line.tsx".into(),
        content_hash: "synthetic-owner-columns".into(),
        language_id: LanguageId::new("typescript"),
        module_path: None,
        raw_symbols: declarations
            .iter()
            .map(|range| RawSymbol::new("Panel", SymbolKind::Function, *range))
            .collect(),
        raw_imports: Vec::new(),
        raw_exports: None,
        raw_relationships: sites
            .iter()
            .map(|site| RawRelationship::new("Panel", *site, "Calendar", *site, RelationKind::Uses))
            .collect(),
        variable_bindings: Vec::new(),
        this_barrier_spans: Vec::new(),
    };
    let (input, receiver) = bounded(1);
    let (sender, output) = bounded(10);
    input.send(parsed).unwrap();
    drop(input);
    CollectStage::new(100)
        .run(receiver, sender, None, None)
        .unwrap();
    let batches: Vec<_> = output.iter().collect();
    let symbols: HashMap<_, _> = batches
        .iter()
        .flat_map(|batch| &batch.symbols)
        .map(|symbol| (symbol.id, symbol.range))
        .collect();
    batches
        .iter()
        .flat_map(|batch| &batch.unresolved_relationships)
        .map(|relationship| symbols[&relationship.from_id.expect("source owner")])
        .collect()
}

#[test]
fn disjoint_same_line_owners_are_selected_by_column() {
    let first = Range::new(0, 0, 0, 60);
    let second = Range::new(0, 70, 0, 130);
    let sites = [Range::new(0, 10, 0, 20), Range::new(0, 100, 0, 110)];
    for declarations in [[first, second], [second, first]] {
        assert_eq!(collected_owners(&declarations, &sites), vec![first, second]);
    }
}

#[test]
fn same_line_nested_owners_choose_the_innermost_range() {
    let outer = Range::new(0, 0, 0, 130);
    let inner = Range::new(0, 20, 0, 60);
    let site = Range::new(0, 30, 0, 40);
    for declarations in [[inner, outer], [outer, inner]] {
        assert_eq!(collected_owners(&declarations, &[site]), vec![inner]);
    }
}

#[test]
fn coincident_starts_choose_the_narrowest_enclosing_range() {
    let outer = Range::new(0, 0, 4, 80);
    let inner = Range::new(0, 0, 2, 60);
    let site = Range::new(1, 30, 1, 40);
    for declarations in [[inner, outer], [outer, inner]] {
        assert_eq!(collected_owners(&declarations, &[site]), vec![inner]);
    }
}

#[test]
fn shared_boundary_line_respects_start_and_end_columns() {
    let first = Range::new(0, 0, 3, 10);
    let second = Range::new(3, 20, 5, 30);
    let sites = [Range::new(3, 6, 3, 9), Range::new(3, 25, 3, 28)];
    assert_eq!(
        collected_owners(&[first, second], &sites),
        vec![first, second]
    );
}

#[test]
fn name_only_range_fallback_remains_compatible() {
    let first = Range::new(0, 0, 0, 5);
    let last = Range::new(4, 0, 4, 5);
    assert_eq!(
        collected_owners(&[first, last], &[Range::new(2, 10, 2, 20)]),
        vec![last]
    );
}

#[test]
fn same_line_jsx_arrows_do_not_exchange_persisted_uses() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("src");
    std::fs::create_dir(&root).unwrap();
    let source = "function One() { return null; } function Two() { return null; } export function First() { const Panel = () => <One />; return <Panel />; } export function Second() { const Panel = () => <Two />; return <Panel />; }";
    std::fs::write(root.join("view.tsx"), source).unwrap();
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.clone()).unwrap();
    let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
    index.index_directory(&root, true).unwrap();
    let mut panels = index.find_symbols_by_name("Panel", Some("typescript"));
    panels.sort_by_key(|symbol| symbol.range.start_column);
    assert_eq!(panels.len(), 2);
    for (panel, expected) in panels.iter().zip(["One", "Two"]) {
        let edges = index
            .graph_neighbors(panel.id, RelationKind::Uses, false, None)
            .unwrap();
        assert_eq!(edges.len(), 1, "{panel:?}");
        assert_eq!(edges[0].0.name.as_ref(), expected);
        let position = edges[0].1.as_ref().expect("JSX source position");
        assert_eq!(position.line, Some(0));
        assert_eq!(
            position.column,
            Some(source.find(&format!("<{expected} />")).unwrap() as u32)
        );
        assert!(index.get_called_functions(panel.id).is_empty());
    }
}
