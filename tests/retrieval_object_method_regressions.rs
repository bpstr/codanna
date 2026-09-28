//! Reduced object-method retrieval witnesses. Source is synthetic and local.
//! No embedding backend is enabled.

use codanna::indexing::facade::IndexFacade;
use codanna::parsing::LanguageParser;
use codanna::parsing::typescript::TypeScriptParser;
use codanna::types::SymbolCounter;
use codanna::{FileId, IndexPersistence, ScopeContext, Settings, Symbol, SymbolKind};
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
fn returned_object_methods_have_callable_symbol_endpoints() {
    assert_method_endpoints(ADAPTER, &["listColumn", "loadTasks"]);
}

#[test]
fn bound_object_methods_have_callable_symbol_endpoints() {
    assert_method_endpoints(BOUND, &["listBoundColumn"]);
}

#[test]
fn object_methods_restore_class_scope_and_keep_named_nested_owners() {
    let code = "class Host { build() { const nested = { run() { function inner() {} return inner(); } }; return nested; } after() {} }";
    let mut parser = TypeScriptParser::new().unwrap();
    let symbols = parser.parse(code, FileId::new(1).unwrap(), &mut SymbolCounter::new());
    for name in ["build", "run", "inner", "after"] {
        assert_eq!(
            symbols.iter().filter(|s| s.name.as_ref() == name).count(),
            1
        );
    }
    let run = symbols.iter().find(|s| s.name.as_ref() == "run").unwrap();
    assert!(matches!(
        &run.scope_context,
        Some(ScopeContext::Local { parent_name: Some(parent), .. }) if parent.as_ref() == "build"
    ));
    let inner = symbols.iter().find(|s| s.name.as_ref() == "inner").unwrap();
    assert!(matches!(
        &inner.scope_context,
        Some(ScopeContext::Local { parent_name: Some(parent), .. }) if parent.as_ref() == "run"
    ));
    let after = symbols.iter().find(|s| s.name.as_ref() == "after").unwrap();
    assert!(matches!(
        &after.scope_context,
        Some(ScopeContext::ClassMember { class_name: Some(class) }) if class.as_ref() == "Host"
    ));
}

#[test]
fn function_bindings_do_not_duplicate_nested_object_methods() {
    let code = "export const factory = () => ({ nested() { return 1; } });";
    let mut parser = TypeScriptParser::new().unwrap();
    let symbols = parser.parse(code, FileId::new(1).unwrap(), &mut SymbolCounter::new());
    for name in ["factory", "nested"] {
        assert_eq!(
            symbols.iter().filter(|s| s.name.as_ref() == name).count(),
            1
        );
    }
}

#[test]
fn computed_keys_and_function_properties_do_not_gain_guessed_endpoints() {
    let code = "const key = 'dynamic'; const object = { [key]() {}, arrow: () => {}, expression: function() {} };";
    let mut parser = TypeScriptParser::new().unwrap();
    let symbols = parser.parse(code, FileId::new(1).unwrap(), &mut SymbolCounter::new());
    assert!(
        symbols
            .iter()
            .all(|symbol| !matches!(symbol.kind, SymbolKind::Method | SymbolKind::Function))
    );
}

#[test]
fn accessors_keep_separate_ranges_and_signatures() {
    let code = "const object = { get value() { return 1; }, set value(next: number) {} };";
    let mut parser = TypeScriptParser::new().unwrap();
    let symbols = parser.parse(code, FileId::new(1).unwrap(), &mut SymbolCounter::new());
    let accessors: Vec<_> = symbols
        .iter()
        .filter(|s| s.name.as_ref() == "value")
        .collect();
    assert_eq!(accessors.len(), 2);
    assert_ne!(accessors[0].range, accessors[1].range);
    assert!(
        accessors[0]
            .signature
            .as_deref()
            .unwrap()
            .contains("get value")
    );
    assert!(
        accessors[1]
            .signature
            .as_deref()
            .unwrap()
            .contains("set value")
    );
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
        let edges = index.get_called_functions_with_metadata(id);
        let metadata = edges
            .iter()
            .find(|(symbol, _)| symbol.id == implementation.id)
            .and_then(|(_, metadata)| metadata.as_ref())
            .expect("persisted call location");
        let caller = index
            .get_calling_functions(implementation.id)
            .into_iter()
            .find(|symbol| symbol.id == id)
            .unwrap();
        let code = if Path::new(caller.file_path.as_ref()).ends_with("adapter.ts") {
            ADAPTER
        } else {
            BOUND
        };
        let line = code.lines().nth(metadata.line.unwrap() as usize).unwrap();
        assert!(line[metadata.column.unwrap() as usize..].starts_with("mergePage("));
    }
}

#[test]
fn duplicate_method_names_keep_callers_separate_across_reopen_and_edit() {
    let (temp, mut index) = fixture();
    let path = temp.path().join("src/duplicates.ts");
    let code = "import { mergePage as real } from './page';\nimport { mergePage as decoy } from './reference';\nexport const first = { run() { return real([], [1]); } };\nexport const second = { run() { return decoy([], [2]); } };\n";
    std::fs::write(&path, code).unwrap();
    index.index_file(&path).unwrap();
    let settings = Arc::clone(index.settings());
    IndexPersistence::new(settings.index_path.clone())
        .save_facade(&index)
        .unwrap();
    drop(index);
    index = IndexPersistence::new(settings.index_path.clone())
        .load_facade_lite(settings)
        .unwrap();
    let real = target(&index, "page.ts", "mergePage");
    let decoy = target(&index, "reference.ts", "mergePage");
    let callers = |index: &IndexFacade, id| {
        index
            .get_calling_functions(id)
            .into_iter()
            .filter(|symbol| Path::new(symbol.file_path.as_ref()).ends_with("duplicates.ts"))
            .map(|symbol| symbol.range.start_line)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(callers(&index, real.id), BTreeSet::from([2]));
    assert_eq!(callers(&index, decoy.id), BTreeSet::from([3]));
    std::fs::write(
        &path,
        code.replace("return real([], [1])", "return decoy([], [1])"),
    )
    .unwrap();
    index.index_file(&path).unwrap();
    assert!(callers(&index, real.id).is_empty());
    assert_eq!(callers(&index, decoy.id), BTreeSet::from([2, 3]));
}
