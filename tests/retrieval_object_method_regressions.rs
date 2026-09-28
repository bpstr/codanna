//! Reduced object-method retrieval witnesses. Source is synthetic and local.
//! No embedding backend is enabled. Known gaps are opt-in ignored tests until fixed.

use codanna::indexing::facade::IndexFacade;
use codanna::parsing::LanguageParser;
use codanna::parsing::typescript::TypeScriptParser;
use codanna::types::SymbolCounter;
use codanna::{FileId, Settings, Symbol, SymbolKind};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

const PAGE: &str = include_str!("fixtures/retrieval_shapes/page.ts");
const ADAPTER: &str = include_str!("fixtures/retrieval_shapes/adapter.ts");
const BOUND: &str = include_str!("fixtures/retrieval_shapes/bound-object.ts");
const REFERENCE: &str = include_str!("fixtures/retrieval_shapes/reference.ts");

#[test]
fn returned_object_calls_keep_method_owners_through_anonymous_callbacks() {
    let mut parser = TypeScriptParser::new().unwrap();
    let calls: Vec<_> = parser
        .find_calls(ADAPTER)
        .into_iter()
        .filter(|(_, to, _)| *to == "mergePage")
        .collect();
    assert_eq!(calls.len(), 3, "retain both methods and the named control");
    let owners: BTreeSet<_> = calls.iter().map(|(from, _, _)| *from).collect();
    assert_eq!(
        owners,
        BTreeSet::from(["listColumn", "loadTasks", "testHelper"])
    );
    for (_, _, range) in calls {
        let line = ADAPTER.lines().nth(range.start_line as usize).unwrap();
        assert!(line[range.start_column as usize..].starts_with("mergePage("));
    }
}

fn assert_method_endpoints(code: &str, names: &[&str]) {
    let mut parser = TypeScriptParser::new().unwrap();
    let mut counter = SymbolCounter::new();
    let symbols = parser.parse(code, FileId::new(1).unwrap(), &mut counter);
    for name in names {
        let endpoints: Vec<_> = symbols
            .iter()
            .filter(|symbol| symbol.name.as_ref() == *name)
            .collect();
        assert_eq!(endpoints.len(), 1, "exactly one endpoint for {name}");
        let symbol = endpoints[0];
        assert!(matches!(
            symbol.kind,
            SymbolKind::Method | SymbolKind::Function
        ));
        let start = code.lines().nth(symbol.range.start_line as usize).unwrap();
        assert!(
            start.contains(name),
            "endpoint starts at its method declaration"
        );
        assert!(symbol.range.end_line > symbol.range.start_line);
    }
}

#[test]
#[ignore = "Known gap: object method definitions are not emitted as caller endpoints"]
fn returned_object_methods_have_callable_symbol_endpoints() {
    assert_method_endpoints(ADAPTER, &["listColumn", "loadTasks"]);
}

#[test]
#[ignore = "Known gap: ordinary object variable initializers are not traversed for symbols"]
fn bound_object_methods_have_callable_symbol_endpoints() {
    assert_method_endpoints(BOUND, &["listBoundColumn"]);
}

fn fixture() -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("src");
    std::fs::create_dir_all(&root).unwrap();
    for (name, code) in [
        ("page.ts", PAGE),
        ("adapter.ts", ADAPTER),
        ("bound-object.ts", BOUND),
        ("reference.ts", REFERENCE),
    ] {
        std::fs::write(root.join(name), code).unwrap();
    }
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.clone()).unwrap();
    let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
    index.index_directory(&root, true).unwrap();
    (temp, index)
}

fn target(index: &IndexFacade, path: &str, name: &str) -> Symbol {
    let found: Vec<_> = index
        .find_symbols_by_name(name, Some("typescript"))
        .into_iter()
        .filter(|symbol| Path::new(symbol.file_path.as_ref()).ends_with(Path::new(path)))
        .collect();
    assert_eq!(found.len(), 1, "unambiguous target {path}:{name}");
    found[0].clone()
}

#[test]
fn named_control_resolves_to_the_imported_implementation() {
    let (_temp, index) = fixture();
    let implementation = target(&index, "page.ts", "mergePage");
    let helper = target(&index, "adapter.ts", "testHelper");
    assert!(
        index
            .get_calling_functions(implementation.id)
            .iter()
            .any(|symbol| symbol.id == helper.id)
    );
    assert!(
        index
            .get_called_functions(helper.id)
            .iter()
            .any(|symbol| symbol.id == implementation.id)
    );
    let reference = target(&index, "reference.ts", "mergePage");
    assert!(index.get_calling_functions(reference.id).is_empty());
}

#[test]
#[ignore = "Known gap: parser call evidence has no object-method symbol IDs to resolve from"]
fn persisted_callers_include_object_methods_without_namesake_leakage() {
    let (_temp, index) = fixture();
    let implementation = target(&index, "page.ts", "mergePage");
    let expected: BTreeSet<_> = [
        ("adapter.ts", "listColumn"),
        ("adapter.ts", "loadTasks"),
        ("adapter.ts", "testHelper"),
        ("bound-object.ts", "listBoundColumn"),
    ]
    .into_iter()
    .map(|(path, name)| target(&index, path, name).id)
    .collect();
    let actual: BTreeSet<_> = index
        .get_calling_functions(implementation.id)
        .into_iter()
        .map(|symbol| symbol.id)
        .collect();
    assert_eq!(
        actual, expected,
        "callers are unique functions, not call sites"
    );
    let reference = target(&index, "reference.ts", "mergePage");
    assert!(index.get_calling_functions(reference.id).is_empty());
    for id in expected {
        assert!(
            index
                .get_called_functions(id)
                .iter()
                .any(|symbol| symbol.id == implementation.id)
        );
    }
}
