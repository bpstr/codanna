//! Loop bindings are not imported module identities. Source snippets are
//! synthetic; no model, provider, or production workspace participates.

use codanna::indexing::facade::IndexFacade;
use codanna::parsing::javascript::JavaScriptParser;
use codanna::parsing::typescript::TypeScriptParser;
use codanna::parsing::LanguageParser;
use codanna::{IndexPersistence, RelationKind, Settings, Symbol};
use std::path::Path;
use std::sync::Arc;

fn assert_only_restored_use(source: &str, target: &str) {
    let mut parser = TypeScriptParser::new().unwrap();
    let uses = parser.find_uses(source);
    assert_eq!(uses.len(), 1, "only the out-of-loop binding is known: {source}\n{uses:?}");
    assert_eq!(uses[0].0, "Page");
    assert_eq!(uses[0].1, target);
    let offset = source.find(&format!("<{target} restored />")).unwrap();
    let prefix = &source[..offset];
    assert_eq!(uses[0].2.start_line, prefix.bytes().filter(|b| *b == b'\n').count() as u32);
    assert_eq!(uses[0].2.start_column, prefix.rsplit('\n').next().unwrap().len() as u32);
}

#[test]
fn loop_header_namespace_bindings_do_not_escape_their_scope() {
    for header in [
        "for (const ui of items)",
        "for (let ui of items)",
        "for (const {namespace: ui} of items)",
        "for (const [ui] of items)",
        "for await (const ui of items)",
        "for (let ui = other; ready; )",
        "for (let {namespace: ui} = other; ready; )",
        // A string-valued for-in key is deliberately not a component provider.
        "for (let ui in items)",
    ] {
        let source = format!("async function Page(items, other, ready) {{\n  {header} {{ consume(<ui.Calendar />); }}\n  return <ui.Calendar restored />;\n}}\n");
        assert_only_restored_use(&source, "ui.Calendar");
    }
}

#[test]
fn bare_component_names_are_also_shadowed_by_loop_bindings() {
    for header in [
        "for (const Calendar of items)",
        "for (const {component: Calendar} of items)",
        "for (let Calendar = other; ready; )",
    ] {
        let source = format!("function Page(items, other, ready) {{\n  {header} {{ consume(<Calendar />); }}\n  return <Calendar restored />;\n}}\n");
        assert_only_restored_use(&source, "Calendar");
    }
}

#[test]
fn callback_references_use_the_same_loop_binding_guard_in_js_and_ts() {
    for header in [
        "for (const handler of items)",
        "for (const {callback: handler} of items)",
        "for (const [handler] of items)",
        "for (let handler = other; ready; )",
    ] {
        let source = format!("function handler() {{}}\nfunction install(items, other, ready) {{\n  {header} {{ register(handler); }}\n  register(handler);\n}}\n");
        let mut parsers: Vec<Box<dyn LanguageParser>> = vec![
            Box::new(TypeScriptParser::new().unwrap()),
            Box::new(JavaScriptParser::new().unwrap()),
        ];
        for parser in &mut parsers {
            let references = parser.find_references(&source);
            assert_eq!(references.len(), 1, "{header}: {references:?}");
            assert_eq!(references[0].source_name, "install");
            assert_eq!(references[0].target_name, "handler");
            assert_eq!(references[0].range.start_line, 3);
        }
    }
}

#[test]
fn property_keys_and_initializer_values_are_not_loop_binding_names() {
    for header in [
        "for (const entry of [ui])",
        "for (const {ui: renamed} of items)",
        "for (const {value = ui} of items)",
        "for (let copy = ui; ready; )",
    ] {
        let source = format!("function Page(items, ready) {{\n  {header} {{ return <ui.Calendar restored />; }}\n}}\n");
        assert_only_restored_use(&source, "ui.Calendar");
    }
}

fn target(index: &IndexFacade, file: &str, name: &str) -> Symbol {
    let matches: Vec<_> = index.find_symbols_by_name(name, Some("typescript"))
        .into_iter().filter(|symbol| Path::new(symbol.file_path.as_ref()).ends_with(file)).collect();
    assert_eq!(matches.len(), 1, "{file}:{name}: {matches:?}");
    matches[0].clone()
}

#[test]
fn loop_binding_changes_repair_stored_uses_after_reopen_without_decoy_edges() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    for file in ["provider.tsx", "decoy.tsx"] {
        std::fs::write(source.join(file), "export function Calendar() { return null; }\n").unwrap();
    }
    let view = "import * as ui from './provider';\nexport function Loop(items) { for (const ui of items) { consume(<ui.Calendar />); } return null; }\nexport function Real() { return <ui.Calendar />; }\n";
    let view_path = source.join("view.tsx");
    std::fs::write(&view_path, view).unwrap();
    let mut settings = Settings { workspace_root: Some(source.clone()), index_path: temp.path().join("index"), ..Default::default() };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(source.clone()).unwrap();
    let settings = Arc::new(settings);
    let mut index = IndexFacade::new(Arc::clone(&settings)).unwrap();
    index.index_directory(&source, true).unwrap();
    let check = |index: &IndexFacade, loop_uses_provider: bool| {
        let provider = target(index, "provider.tsx", "Calendar");
        for (name, expected) in [("Loop", usize::from(loop_uses_provider)), ("Real", 1)] {
            let owner = target(index, "view.tsx", name);
            let edges = index.graph_neighbors(owner.id, RelationKind::Uses, false, None).unwrap();
            assert_eq!(edges.len(), expected, "{name}: {edges:?}");
            for (callee, _) in edges { assert_eq!(callee.id, provider.id); }
            assert!(!index.get_called_functions(owner.id).iter().any(|callee| callee.name.as_ref() == "Calendar"));
        }
    };
    check(&index, false);
    let persistence = IndexPersistence::new(settings.index_path.clone());
    persistence.save_facade(&index).unwrap();
    drop(index);
    index = persistence.load_facade_lite(settings).unwrap();
    check(&index, false);
    std::fs::write(&view_path, view.replace("const ui of", "const entry of")).unwrap();
    index.index_file(&view_path).unwrap();
    check(&index, true);
    std::fs::write(&view_path, view).unwrap();
    index.index_file(&view_path).unwrap();
    check(&index, false);
}
