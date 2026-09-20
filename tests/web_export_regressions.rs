//! Complete-graph checks for explicit ES module exports and bounded PHP field types.
//! Every fixture is local and indexes with semantic search disabled.

use codanna::indexing::facade::IndexFacade;
use codanna::symbol::ScopeContext;
use codanna::{Settings, Symbol};
use std::sync::Arc;

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, IndexFacade) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (name, code) in files {
        let path = src.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    }
    let mut settings = Settings {
        workspace_root: Some(src.clone()),
        index_path: dir.path().join("index"),
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(src.clone()).unwrap();
    let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
    index.index_directory(&src, true).unwrap();
    (dir, index)
}

fn unique(index: &IndexFacade, name: &str) -> Symbol {
    let symbols = index.find_symbols_by_name(name, None);
    assert_eq!(symbols.len(), 1, "fixture expects one {name}: {symbols:?}");
    symbols[0].clone()
}

fn called(index: &IndexFacade, name: &str) -> Vec<Symbol> {
    index.get_called_functions(unique(index, name).id)
}

fn assert_calls(index: &IndexFacade, caller: &str, target: &str) {
    let actual = called(index, caller);
    assert_eq!(
        actual.len(),
        1,
        "{caller} must have exactly one target: {actual:?}"
    );
    assert_eq!(actual[0].name.as_ref(), target);
}

#[test]
fn javascript_default_export_and_bare_arrow_keep_definition_identity() {
    let (_dir, index) = fixture(&[
        (
            "storage.js",
            "export function save(v) {} export default function persist(v) {}",
        ),
        (
            "client.js",
            "import save from './storage.js'; export const run = value => save(value);",
        ),
    ]);
    assert_calls(&index, "run", "persist");
}

#[test]
fn named_and_default_aliases_survive_three_barrels() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export default function persist() {}"),
        ("first.ts", "export { default as save } from './source';"),
        ("second.ts", "export { save as write } from './first';"),
        ("third.ts", "export { write as default } from './second';"),
        (
            "client.ts",
            "import local from './third'; export function run() { local(); }",
        ),
    ]);
    assert_calls(&index, "run", "persist");
}

#[test]
fn import_then_local_export_keeps_the_imported_identity() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export function persist() {}"),
        (
            "barrel.ts",
            "import { persist as local } from './source'; export { local as save };",
        ),
        (
            "client.ts",
            "import { save } from './barrel'; export function run() { save(); }",
        ),
    ]);
    assert_calls(&index, "run", "persist");
}

#[test]
fn ordinary_import_does_not_implicitly_export_a_binding() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export function save() {}"),
        ("barrel.ts", "import { save } from './source';"),
        (
            "client.ts",
            "import { save } from './barrel'; export function run() { save(); }",
        ),
    ]);
    assert!(called(&index, "run").is_empty());
}

#[test]
fn star_export_excludes_default_even_with_a_same_named_decoy() {
    let (_dir, index) = fixture(&[
        (
            "source.ts",
            "export default function persist() {} export function save() {}",
        ),
        ("barrel.ts", "export * from './source';"),
        (
            "client.ts",
            "import save from './barrel'; export function run() { save(); }",
        ),
    ]);
    assert!(called(&index, "run").is_empty());
}

#[test]
fn explicit_export_takes_precedence_over_conflicting_star_export() {
    let (_dir, index) = fixture(&[
        ("a.ts", "export function first() {}"),
        ("b.ts", "export function save() {}"),
        (
            "barrel.ts",
            "export * from './b'; export { first as save } from './a';",
        ),
        (
            "client.ts",
            "import { save } from './barrel'; export function run() { save(); }",
        ),
    ]);
    assert_calls(&index, "run", "first");
}

#[test]
fn conflicting_star_exports_fail_closed() {
    let (_dir, index) = fixture(&[
        ("a.ts", "export function save() {}"),
        ("b.ts", "export function save() {}"),
        ("barrel.ts", "export * from './a'; export * from './b';"),
        (
            "client.ts",
            "import { save } from './barrel'; export function run() { save(); }",
        ),
    ]);
    assert!(called(&index, "run").is_empty());
}

#[test]
fn diamond_star_exports_of_the_same_definition_are_unambiguous() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export function persist() {}"),
        ("left.ts", "export * from './source';"),
        ("right.ts", "export * from './source';"),
        (
            "barrel.ts",
            "export * from './left'; export * from './right';",
        ),
        (
            "client.ts",
            "import { persist } from './barrel'; export function run() { persist(); }",
        ),
    ]);
    assert_calls(&index, "run", "persist");
}

#[test]
fn cyclic_barrel_with_reachable_leaf_terminates_and_resolves() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export function persist() {}"),
        ("a.ts", "export * from './b';"),
        ("b.ts", "export * from './a'; export * from './source';"),
        (
            "client.ts",
            "import { persist } from './a'; export function run() { persist(); }",
        ),
    ]);
    assert_calls(&index, "run", "persist");
}

#[test]
fn namespace_import_calls_the_public_member() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export function persist() {}"),
        (
            "client.ts",
            "import * as storage from './source'; export function run() { storage.persist(); }",
        ),
    ]);
    assert_calls(&index, "run", "persist");
}

#[test]
fn namespace_reexport_calls_the_original_definition() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export function persist() {}"),
        ("barrel.ts", "export * as Storage from './source';"),
        (
            "client.ts",
            "import { Storage } from './barrel'; export function run() { Storage.persist(); }",
        ),
    ]);
    assert_calls(&index, "run", "persist");
}

#[test]
fn namespace_private_member_does_not_fall_through_to_a_decoy() {
    let (_dir, index) = fixture(&[
        (
            "source.ts",
            "function persist() {} export function other() {}",
        ),
        ("decoy.ts", "export class Storage { static persist() {} }"),
        ("barrel.ts", "export * as Storage from './source';"),
        (
            "client.ts",
            "import { Storage } from './barrel'; export function run() { Storage.persist(); }",
        ),
    ]);
    assert!(called(&index, "run").is_empty());
}

#[test]
fn typescript_js_specifier_resolves_the_typescript_source() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export function persist() {}"),
        (
            "client.ts",
            "import { persist } from './source.js'; export function run() { persist(); }",
        ),
    ]);
    assert_calls(&index, "run", "persist");
}

#[test]
fn type_only_barrel_preserves_uses_without_creating_runtime_calls() {
    let (_dir, index) = fixture(&[
        ("source.ts", "export class Factory { static create() {} }"),
        ("first.ts", "export type { Factory } from './source';"),
        ("barrel.ts", "export { Factory } from './first';"),
        (
            "client.ts",
            "import { Factory } from './barrel'; export function run(value: Factory) { Factory.create(); }",
        ),
    ]);
    assert!(called(&index, "run").is_empty());
    let uses = index.get_uses(unique(&index, "run").id);
    assert!(
        uses.iter().any(|symbol| symbol.name.as_ref() == "Factory"),
        "type usage should survive: {uses:?}"
    );
}

#[test]
fn exports_are_persisted_for_a_reopened_partial_index() {
    let (dir, index) = fixture(&[
        ("source.ts", "export default function persist() {}"),
        ("barrel.ts", "export { default as save } from './source';"),
        (
            "client.ts",
            "import { save } from './barrel'; export function run() { save(); }",
        ),
    ]);
    let settings = index.settings().clone();
    drop(index);
    let mut index = IndexFacade::new(settings).unwrap();
    let client = dir.path().join("src/client.ts");
    std::fs::write(
        &client,
        "import { save as local } from './barrel'; export function runAgain() { local(); }",
    )
    .unwrap();
    index.index_file(&client).unwrap();
    assert_calls(&index, "runAgain", "persist");
}

#[test]
fn php_promoted_and_nullable_declared_fields_resolve_their_own_type() {
    let (_dir, index) = fixture(&[(
        "billing.php",
        concat!(
            "<?php class Gateway { public function charge() {} }\n",
            "class Decoy { public function charge() {} }\n",
            "class Checkout { public ?Gateway $ordinary;\n",
            "public function __construct(private readonly Gateway $gateway) {}\n",
            "public function pay() { $this->gateway->charge(); }\n",
            "public function retry() { $this->ordinary?->charge(); } }\n",
        ),
    )]);
    for caller in ["pay", "retry"] {
        let targets = called(&index, caller);
        assert_eq!(targets.len(), 1, "{caller}: {targets:?}");
        assert_eq!(targets[0].name.as_ref(), "charge");
        assert!(
            matches!(&targets[0].scope_context, Some(ScopeContext::ClassMember { class_name: Some(name) }) if name.as_ref() == "Gateway"),
            "{caller} must select Gateway::charge: {targets:?}"
        );
    }
}

#[test]
fn php_untyped_and_union_properties_do_not_guess_a_receiver_class() {
    let (_dir, index) = fixture(&[(
        "billing.php",
        concat!(
            "<?php class Gateway { public function charge() {} }\n",
            "class Other { public function charge() {} }\n",
            "class Checkout { public $untyped; public Gateway|Other $either;\n",
            "public function pay() { $this->untyped->charge(); }\n",
            "public function retry() { $this->either->charge(); } }\n",
        ),
    )]);
    assert!(called(&index, "pay").is_empty());
    assert!(called(&index, "retry").is_empty());
}

#[test]
fn tsconfig_alias_missing_export_is_negative_evidence_for_resolution() {
    use codanna::indexing::pipeline::stages::resolve::ResolveStage;
    use codanna::indexing::pipeline::types::{
        ResolutionContext, SymbolLookupCache, UnresolvedRelationship,
    };
    use codanna::parsing::resolution::ImportOrigin;
    use codanna::parsing::typescript::{TypeScriptBehavior, TypeScriptParser};
    use codanna::parsing::{FileExports, LanguageBehavior, LanguageId, LanguageParser};
    use codanna::types::SymbolCounter;
    use codanna::{FileId, RelationKind};
    use std::collections::HashMap;

    let dir = tempfile::tempdir().unwrap();
    let rules_dir = dir.path().join(".codanna/index/resolvers");
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let config = dir
        .path()
        .join("tsconfig.json")
        .to_string_lossy()
        .into_owned();
    let mut hashes = serde_json::Map::new();
    hashes.insert(config.clone(), serde_json::json!("fixture"));
    let mut mappings = serde_json::Map::new();
    mappings.insert(
        format!("{}/**/*.ts", dir.path().display()),
        serde_json::json!(config),
    );
    let mut rules = serde_json::Map::new();
    rules.insert(
        config,
        serde_json::json!({"baseUrl": ".", "paths": {"@store": ["src/storage"]}}),
    );
    std::fs::write(
        rules_dir.join("typescript_resolution.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": "1.0", "hashes": hashes, "mappings": mappings, "rules": rules
        }))
        .unwrap(),
    )
    .unwrap();

    let behavior = Arc::new(TypeScriptBehavior::with_resolution_dir(
        dir.path().join(".codanna"),
    ));
    let language = LanguageId::new("typescript");
    let cache = Arc::new(SymbolLookupCache::new());
    let mut parser = TypeScriptParser::new().unwrap();
    let mut counter = SymbolCounter::new();
    let client_code = "import { save, other } from '@store'; export function runMissing() { save(); } export function runPresent() { other(); }";
    let client_id = FileId::new(3).unwrap();
    for (id, name, code) in [
        (1, "storage", "export function other() {}"),
        (2, "decoy", "export function save() {}"),
        (3, "client", client_code),
    ] {
        let file_id = FileId::new(id).unwrap();
        let path = dir.path().join(format!("src/{name}.ts"));
        std::fs::write(&path, code).unwrap();
        let module = format!("src.{name}");
        behavior.register_file(path.clone(), file_id, module.clone());
        for mut symbol in parser.parse(code, file_id, &mut counter) {
            symbol.file_path = path.to_string_lossy().into_owned().into();
            symbol.language_id = Some(language);
            behavior.configure_symbol(&mut symbol, Some(&module));
            cache.insert(symbol);
        }
        cache.register_file_exports(FileExports {
            file_id,
            file_path: path.to_string_lossy().into_owned(),
            module_path: Some(module),
            exports: parser.find_exports(code).unwrap(),
        });
    }
    let imports = parser.find_imports(client_code, client_id);
    let (scope, imports) = behavior.build_resolution_context_with_pipeline_cache(
        client_id,
        &imports,
        cache.as_ref(),
        &["ts", "tsx"],
    );
    let binding = scope.import_binding("save").unwrap();
    assert_eq!(binding.origin, ImportOrigin::Internal);
    assert!(binding.resolved_symbol.is_none());
    let unresolved_rels = parser
        .find_calls(client_code)
        .into_iter()
        .map(|(from, to, range)| UnresolvedRelationship {
            from_id: cache.lookup_candidates(from).into_iter().next(),
            from_name: from.into(),
            to_name: to.into(),
            file_id: client_id,
            kind: RelationKind::Calls,
            metadata: None,
            to_range: Some(range),
        })
        .collect();
    let context = ResolutionContext {
        file_id: client_id,
        language_id: language,
        imports,
        local_symbols: cache.symbols_in_file(client_id),
        scope,
        unresolved_rels,
        variable_bindings: Vec::new(),
        this_barrier_spans: Vec::new(),
    };
    let mut behaviors = HashMap::<LanguageId, Arc<dyn LanguageBehavior>>::new();
    behaviors.insert(language, behavior);
    let (resolved, _) = ResolveStage::new(cache.clone(), behaviors).resolve(&context);
    assert_eq!(
        resolved.relationships.len(),
        1,
        "missing aliased export must not call the decoy: {resolved:?}"
    );
    assert_eq!(
        cache
            .get(resolved.relationships[0].to_id)
            .unwrap()
            .name
            .as_ref(),
        "other"
    );
}

#[test]
fn typescript_declared_and_constructor_fields_anchor_method_calls() {
    let (_dir, index) = fixture(&[(
        "billing.ts",
        concat!(
            "class Gateway { charge() {} } class Decoy { charge() {} }\n",
            "class Checkout { ordinary?: Gateway;\n",
            "constructor(private readonly gateway: Gateway) {}\n",
            "pay() { this.gateway.charge(); } retry() { this.ordinary?.charge(); }\n",
            "schedule() { const later = () => this.gateway.charge(); return later; } }\n",
        ),
    )]);
    for caller in ["pay", "retry", "later"] {
        let targets = called(&index, caller);
        assert_eq!(targets.len(), 1, "{caller}: {targets:?}");
        assert!(
            matches!(&targets[0].scope_context, Some(ScopeContext::ClassMember { class_name: Some(name) }) if name.as_ref() == "Gateway"),
            "{caller} must select Gateway::charge: {targets:?}"
        );
    }
    let gateway = unique(&index, "gateway");
    assert_eq!(gateway.kind, codanna::SymbolKind::Field);
}

#[test]
fn typescript_field_type_stops_at_an_ordinary_function_this_boundary() {
    let (_dir, index) = fixture(&[(
        "billing.ts",
        concat!(
            "class Gateway { charge() {} }\n",
            "class Checkout { constructor(private gateway: Gateway) {}\n",
            "schedule() { function detached() { this.gateway.charge(); } return detached; } }\n",
        ),
    )]);
    assert!(called(&index, "detached").is_empty());
}

#[test]
fn php_qualified_property_type_does_not_capture_a_bare_named_decoy() {
    let (_dir, index) = fixture(&[(
        "billing.php",
        concat!(
            "<?php class Gateway { public function charge() {} }\n",
            "class Checkout { public \\External\\Gateway $gateway;\n",
            "public function pay() { $this->gateway->charge(); } }\n",
        ),
    )]);
    assert!(called(&index, "pay").is_empty());
}

#[test]
fn dense_barrel_diamond_exhaustion_cannot_publish_a_partial_winner() {
    use codanna::FileId;
    use codanna::indexing::pipeline::types::SymbolLookupCache;
    use codanna::parsing::typescript::TypeScriptParser;
    use codanna::parsing::{ExportResolution, FileExports, LanguageId, LanguageParser};
    use codanna::types::SymbolCounter;

    let cache = SymbolLookupCache::new();
    let mut parser = TypeScriptParser::new().unwrap();
    let mut counter = SymbolCounter::new();
    let mut register = |id: u32, name: &str, code: &str| {
        let file_id = FileId::new(id).unwrap();
        let path = format!("/fixture/{name}.ts");
        for mut symbol in parser.parse(code, file_id, &mut counter) {
            symbol.file_path = path.clone().into();
            symbol.module_path = Some(name.into());
            symbol.language_id = Some(LanguageId::new("typescript"));
            cache.insert(symbol);
        }
        cache.register_file_exports(FileExports {
            file_id,
            file_path: path,
            module_path: Some(name.into()),
            exports: parser.find_exports(code).unwrap(),
        });
    };
    register(1, "leaf", "export function persist() {}");
    register(2, "client", "import { persist } from './root';");
    // Each level has two aliases to the same next level: the graph is small,
    // but recursively expanding every path would visit over a million slots.
    for level in (0..20).rev() {
        let next = if level == 19 {
            "leaf".to_string()
        } else {
            format!("level{}", level + 1)
        };
        register(
            10 + level * 3,
            &format!("left{level}"),
            &format!("export * from './{next}';"),
        );
        register(
            11 + level * 3,
            &format!("right{level}"),
            &format!("export * from './{next}';"),
        );
        register(
            12 + level * 3,
            &format!("level{level}"),
            &format!("export * from './left{level}'; export * from './right{level}';"),
        );
    }
    // A cheap first branch finds persist; the later expensive branch must not
    // be silently ignored when its traversal budget runs out.
    register(
        100,
        "root",
        "export * from './leaf'; export * from './level0';",
    );
    assert_eq!(
        cache.resolve_export(FileId::new(2).unwrap(), "./root", "persist", &["ts"]),
        ExportResolution::Ambiguous
    );
}

#[test]
fn export_depth_exhaustion_cannot_hide_a_conflicting_target() {
    use codanna::indexing::pipeline::types::SymbolLookupCache;
    use codanna::parsing::{Export, ExportResolution, FileExports};
    use codanna::{FileId, Range, SymbolId, SymbolKind};
    let cache = SymbolLookupCache::new();
    for id in [1, 2] {
        cache.insert(Symbol::new(
            SymbolId::new(id).unwrap(),
            "persist",
            SymbolKind::Function,
            FileId::new(id).unwrap(),
            Range::new(0, 0, 0, 10),
        ));
    }
    let local_export = || Export {
        exported_name: "persist".into(),
        local_name: Some("persist".into()),
        source: None,
        is_glob: false,
        is_type_only: false,
        is_namespace: false,
    };
    let star = |source: String| Export {
        exported_name: "*".into(),
        local_name: None,
        source: Some(source),
        is_glob: true,
        is_type_only: false,
        is_namespace: false,
    };
    let register = |id: u32, name: &str, exports| {
        cache.register_file_exports(FileExports {
            file_id: FileId::new(id).unwrap(),
            file_path: format!("/fixture/{name}.ts"),
            module_path: Some(name.into()),
            exports,
        })
    };
    register(1, "leaf", vec![local_export()]);
    register(2, "other", vec![local_export()]);
    register(3, "client", Vec::new());
    for level in 0..70 {
        register(
            10 + level,
            &format!("level{level}"),
            vec![star(if level == 69 {
                "./other".into()
            } else {
                format!("./level{}", level + 1)
            })],
        );
    }
    register(
        100,
        "root",
        vec![star("./leaf".into()), star("./level0".into())],
    );
    assert_eq!(
        cache.resolve_export(FileId::new(3).unwrap(), "./root", "persist", &["ts"]),
        ExportResolution::Ambiguous
    );
}
