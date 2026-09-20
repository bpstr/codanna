//! Full local IndexFacade pipeline probes: discover -> parse -> resolve -> persist -> graph query.
//! All fixtures are deterministic. Embeddings and remote providers remain disabled.

use codanna::indexing::facade::IndexFacade;
use codanna::{ScopeContext, Settings, Symbol};
use std::{fs, sync::Arc};

fn index_fixture(filename: &str, content: &str) -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join(filename), content).unwrap();
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        workspace_root: None,
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(src.clone()).unwrap();
    let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
    index.index_directory(&src, false).unwrap();
    (temp, index)
}

fn symbol(index: &IndexFacade, name: &str) -> Symbol {
    let symbols = index.find_symbols_by_name(name, None);
    assert_eq!(symbols.len(), 1, "expected unique {name}: {symbols:?}");
    symbols[0].clone()
}

fn class_of(symbol: &Symbol) -> Option<&str> {
    match symbol.scope_context.as_ref() {
        Some(ScopeContext::ClassMember {
            class_name: Some(name),
        }) => Some(name.as_ref()),
        _ => None,
    }
}

fn method_targets(index: &IndexFacade, caller: &str, method: &str) -> Vec<Symbol> {
    let called = index.get_called_functions(symbol(index, caller).id);
    let methods: Vec<_> = called
        .into_iter()
        .filter(|s| s.name.as_ref() == method)
        .collect();
    println!(
        "{caller} -> {method}: {:?}",
        methods
            .iter()
            .map(|s| (s.id, class_of(s), s.range.start_line))
            .collect::<Vec<_>>()
    );
    methods
}

#[test]
fn end_to_end_rust_annotation_prevents_factory_owner_false_edge() {
    let (_temp, index) = index_fixture("factory.rs", include_str!("fixtures/rust_factory.rs"));
    let inferred = method_targets(&index, "inferred", "consume");
    let annotated = method_targets(&index, "annotated", "consume");
    assert!(
        !inferred.iter().any(|s| class_of(s) == Some("Client")),
        "factory returns Token, never Client"
    );
    assert_eq!(annotated.len(), 1, "explicit type must resolve");
    assert_eq!(class_of(&annotated[0]), Some("Token"));
}

#[test]
fn end_to_end_python_field_annotation_does_not_change_local_receiver() {
    let (_temp, index) = index_fixture(
        "attributes.py",
        include_str!("fixtures/python_attribute_binding.py"),
    );
    let methods = method_targets(&index, "execute", "run");
    assert_eq!(methods.len(), 1, "local First receiver must resolve");
    assert_eq!(class_of(&methods[0]), Some("First"));
}

#[test]
fn end_to_end_python_dispatch_uses_c3_not_nearest_ancestor() {
    let (_temp, index) = index_fixture("mro.py", include_str!("fixtures/python_mro.py"));
    let methods = method_targets(&index, "execute", "run");
    assert_eq!(methods.len(), 1);
    assert_eq!(class_of(&methods[0]), Some("A"));
}

#[test]
fn end_to_end_python_super_resolves_first_bases_inherited_method() {
    let (_temp, index) = index_fixture("mro.py", include_str!("fixtures/python_mro.py"));
    let methods = method_targets(&index, "via_super", "run");
    assert_eq!(methods.len(), 1);
    assert_eq!(class_of(&methods[0]), Some("A"));
}

#[test]
fn end_to_end_go_generic_calls_preserve_every_arity() {
    let (_temp, index) = index_fixture("generic.go", include_str!("fixtures/go_generic_calls.go"));
    let called = index.get_called_functions(symbol(&index, "GenericCalls").id);
    let names: Vec<_> = called.iter().map(|s| s.name.as_ref()).collect();
    println!("GenericCalls calls: {names:?}");
    assert!(names.contains(&"TwoTypes"), "positive control should work");
    for target in ["Identity", "Pair", "Empty"] {
        assert!(
            names.contains(&target),
            "generic call to {target} missing; got {names:?}"
        );
    }
    assert!(
        !names.contains(&"Number"),
        "the generic Number type conversion must not become a function call: {names:?}"
    );
}

#[test]
fn end_to_end_go_structural_implementations_are_discoverable() {
    let (_temp, index) = index_fixture("interfaces.go", include_str!("fixtures/go_interfaces.go"));
    let implementations = index.get_implementations(symbol(&index, "Reader").id);
    let names: Vec<_> = implementations.iter().map(|s| s.name.as_ref()).collect();
    println!("Reader implementations: {names:?}");
    assert!(
        names.contains(&"Socket"),
        "pointer receiver implementation should be represented"
    );
    assert!(
        names.contains(&"Wrapper"),
        "promoted method implementation should be represented"
    );
    assert!(!names.contains(&"WrongSignature"));
}
