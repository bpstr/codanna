//! TypeScript, JavaScript, and PHP parser and complete-graph regressions.
//! Indexes use disposable local directories with semantic search disabled.
use codanna::parsing::javascript::JavaScriptParser;
use codanna::parsing::php::PhpParser;
use codanna::parsing::typescript::{TypeScriptBehavior, TypeScriptParser};
use codanna::parsing::{LanguageBehavior, LanguageId, LanguageParser};
use codanna::types::SymbolCounter;
use codanna::{FileId, SymbolKind};

fn fid() -> FileId {
    FileId::new(1).unwrap()
}

#[test]
fn php_alias_import_retains_visible_name() {
    let code = "<?php use App\\Billing\\Gateway as Payments;";
    let imports = PhpParser::new().unwrap().find_imports(code, fid());
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].path, "App\\Billing\\Gateway");
    assert_eq!(imports[0].alias.as_deref(), Some("Payments"));
}

#[test]
fn php_grouped_imports_expand_prefix_and_alias() {
    let code = "<?php use App\\Billing\\{Gateway, Receipt as Invoice};";
    let imports = PhpParser::new().unwrap().find_imports(code, fid());
    assert_eq!(imports.len(), 2);
    assert!(imports.iter().any(|i| i.path == "App\\Billing\\Gateway"));
    assert!(
        imports
            .iter()
            .any(|i| i.path == "App\\Billing\\Receipt" && i.alias.as_deref() == Some("Invoice"))
    );
}

#[test]
fn php_nullsafe_call_is_still_a_call_site() {
    let code = "<?php function pay(?Gateway $gateway) { $gateway?->charge(); }";
    let mut parser = PhpParser::new().unwrap();
    let calls = parser.find_method_calls(code);
    assert!(calls.iter().any(|c| c.caller == "pay"
        && c.method_name == "charge"
        && c.receiver.as_deref() == Some("$gateway")));
}

#[test]
fn php_qualified_inheritance_is_extracted() {
    let code = "<?php class Checkout extends \\Framework\\BaseController implements \\Framework\\Action {}";
    let mut parser = PhpParser::new().unwrap();
    assert!(
        parser
            .find_extends(code)
            .iter()
            .any(|(a, b, _)| *a == "Checkout" && b.ends_with("BaseController"))
    );
    assert!(
        parser
            .find_implementations(code)
            .iter()
            .any(|(a, b, _)| *a == "Checkout" && b.ends_with("Action"))
    );
}

#[test]
fn php_promoted_dependency_is_a_field() {
    let code = "<?php class Checkout { public function __construct(private readonly Gateway $gateway) {} }";
    let symbols = PhpParser::new()
        .unwrap()
        .parse(code, fid(), &mut SymbolCounter::new());
    assert!(
        symbols
            .iter()
            .any(|s| s.name.as_ref() == "gateway" && s.kind == SymbolKind::Field)
    );
}

#[test]
fn php_simple_parameter_type_emits_usage() {
    let code = "<?php class Invoice {} function archive(Invoice $invoice): void {}";
    let uses = PhpParser::new().unwrap().find_uses(code);
    assert!(
        uses.iter()
            .any(|(a, b, _)| *a == "archive" && *b == "Invoice")
    );
}

#[test]
fn php_property_declaration_emits_every_field() {
    let code = "<?php class Invoice { public string $id, $reference; }";
    let symbols = PhpParser::new()
        .unwrap()
        .parse(code, fid(), &mut SymbolCounter::new());
    assert!(symbols.iter().any(|s| s.name.as_ref() == "id"));
    assert!(symbols.iter().any(|s| s.name.as_ref() == "reference"));
}

#[test]
fn typescript_bare_arrow_parameter_is_not_the_caller_name() {
    let code = "function persist(value: string) {} const save = value => persist(value);";
    let calls = TypeScriptParser::new().unwrap().find_calls(code);
    assert!(
        calls
            .iter()
            .any(|(a, b, _)| *a == "save" && *b == "persist")
    );
    assert!(
        !calls
            .iter()
            .any(|(a, b, _)| *a == "value" && *b == "persist")
    );
}

#[test]
fn javascript_bare_arrow_parameter_is_not_the_caller_name() {
    let code = "function persist(value) {} const save = value => persist(value);";
    let calls = JavaScriptParser::new().unwrap().find_calls(code);
    assert!(
        calls
            .iter()
            .any(|(a, b, _)| *a == "save" && *b == "persist")
    );
    assert!(
        !calls
            .iter()
            .any(|(a, b, _)| *a == "value" && *b == "persist")
    );
}

#[test]
fn typescript_parenthesized_arrow_control_already_works() {
    let code = "function persist(value: string) {} const save = (value: string) => persist(value);";
    let calls = TypeScriptParser::new().unwrap().find_calls(code);
    assert!(
        calls
            .iter()
            .any(|(a, b, _)| *a == "save" && *b == "persist")
    );
}

#[test]
fn php_ordinary_receiver_control_already_works() {
    let code = "<?php function pay(Gateway $gateway) { $gateway->charge(); }";
    let calls = PhpParser::new().unwrap().find_method_calls(code);
    assert!(calls.iter().any(|c| c.caller == "pay"
        && c.method_name == "charge"
        && c.receiver.as_deref() == Some("$gateway")));
}

#[test]
fn typescript_default_import_does_not_bind_same_named_named_export() {
    use codanna::indexing::pipeline::types::SymbolLookupCache;
    let provider_code =
        "export function save(value: string) {} export default function persist(value: string) {}";
    let client_code = "import save from './storage'; export function run() { save('invoice'); }";
    let provider_id = fid();
    let client_id = FileId::new(2).unwrap();
    let mut counter = SymbolCounter::new();
    let mut parser = TypeScriptParser::new().unwrap();
    let mut provider = parser.parse(provider_code, provider_id, &mut counter);
    let mut client = parser.parse(client_code, client_id, &mut counter);
    let expected = provider
        .iter()
        .find(|s| s.name.as_ref() == "persist")
        .unwrap()
        .id;
    let wrong = provider
        .iter()
        .find(|s| s.name.as_ref() == "save")
        .unwrap()
        .id;
    let cache = SymbolLookupCache::new();
    for s in &mut provider {
        s.file_path = "/probe/storage.ts".into();
        s.module_path = Some("storage".into());
        s.language_id = Some(LanguageId::new("typescript"));
        cache.insert(s.clone());
    }
    for s in &mut client {
        s.file_path = "/probe/client.ts".into();
        s.module_path = Some("client".into());
        s.language_id = Some(LanguageId::new("typescript"));
        cache.insert(s.clone());
    }
    // Supply the same explicit export facts as the production indexing stage.
    // A local alias cannot identify a provider's default slot by name alone.
    cache.register_file_exports(codanna::parsing::FileExports {
        file_id: provider_id,
        file_path: "/probe/storage.ts".into(),
        module_path: Some("storage".into()),
        exports: parser.find_exports(provider_code).unwrap(),
    });
    cache.register_file_exports(codanna::parsing::FileExports {
        file_id: client_id,
        file_path: "/probe/client.ts".into(),
        module_path: Some("client".into()),
        exports: parser.find_exports(client_code).unwrap(),
    });
    let behavior = TypeScriptBehavior::with_resolution_dir("/probe/no-rules");
    behavior.register_file("/probe/storage.ts".into(), provider_id, "storage".into());
    behavior.register_file("/probe/client.ts".into(), client_id, "client".into());
    let imports = parser.find_imports(client_code, client_id);
    let (scope, _) = behavior.build_resolution_context_with_pipeline_cache(
        client_id,
        &imports,
        &cache,
        &["ts", "tsx"],
    );
    assert_ne!(
        scope.resolve("save"),
        Some(wrong),
        "default import must not bind the homonymous named export"
    );
    assert_eq!(scope.resolve("save"), Some(expected));
}

fn indexed_fixture(
    files: &[(&str, &str)],
) -> (tempfile::TempDir, codanna::indexing::facade::IndexFacade) {
    use codanna::indexing::facade::IndexFacade;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    for (name, code) in files {
        std::fs::write(src.join(name), code).unwrap();
    }
    let mut settings = codanna::Settings {
        workspace_root: Some(src.clone()),
        index_path: dir.path().join("index"),
        ..codanna::Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(src.clone()).unwrap();
    let mut facade = IndexFacade::new(std::sync::Arc::new(settings)).unwrap();
    facade.index_directory(&src, true).unwrap();
    (dir, facade)
}

#[test]
fn full_graph_typescript_arrow_keeps_caller_identity() {
    let (_dir, facade) = indexed_fixture(&[(
        "arrows.ts",
        concat!(
            "function persist(value: string) {}\n",
            "export const save = value => persist(value);\n",
            "export const control = (value: string) => persist(value);\n",
        ),
    )]);
    let save = facade.find_symbols_by_name("save", Some("typescript"));
    let control = facade.find_symbols_by_name("control", Some("typescript"));
    assert_eq!(save.len(), 1);
    assert_eq!(control.len(), 1);
    assert!(
        facade
            .get_called_functions(control[0].id)
            .iter()
            .any(|s| s.name.as_ref() == "persist"),
        "parenthesized arrow control must work"
    );
    let actual = facade.get_called_functions(save[0].id);
    assert!(
        actual.iter().any(|s| s.name.as_ref() == "persist"),
        "bare arrow must call persist; got {actual:?}"
    );
}

#[test]
fn full_graph_typescript_default_import_calls_default_definition() {
    let (_dir, facade) = indexed_fixture(&[
        (
            "storage.ts",
            concat!(
                "export function save(value: string) { return 'named:' + value; }\n",
                "export default function persist(value: string) { return 'default:' + value; }\n",
            ),
        ),
        (
            "client.ts",
            "import save from './storage';\nexport function run() { return save('invoice'); }\n",
        ),
    ]);
    let run = facade.find_symbols_by_name("run", Some("typescript"));
    assert_eq!(run.len(), 1);
    let called = facade.get_called_functions(run[0].id);
    let names: Vec<_> = called.iter().map(|s| s.name.as_ref()).collect();
    assert!(
        !names.contains(&"save"),
        "default import must not call unrelated named export: {names:?}"
    );
    assert_eq!(names, vec!["persist"]);
}

#[test]
fn full_graph_typescript_explicit_named_import_control() {
    let (_dir, facade) = indexed_fixture(&[
        (
            "storage.ts",
            "export function save(value: string) {}\nexport default function persist(value: string) {}\n",
        ),
        (
            "client.ts",
            "import { save } from './storage';\nexport function run() { save('invoice'); }\n",
        ),
    ]);
    let run = facade.find_symbols_by_name("run", Some("typescript"));
    assert_eq!(run.len(), 1);
    let called = facade.get_called_functions(run[0].id);
    let names: Vec<_> = called.iter().map(|s| s.name.as_ref()).collect();
    assert_eq!(names, vec!["save"]);
}
