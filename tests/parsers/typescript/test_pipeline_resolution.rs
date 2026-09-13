//! Tests for parallel pipeline resolution with TypeScript path aliases
//!
//! Tests that `build_resolution_context_with_pipeline_cache()` works with
//! TypeScript settings including tsconfig path aliases like `@/components/*`.

use codanna::config::Settings;
use codanna::indexing::pipeline::types::{CallerContext, SymbolLookupCache};
use codanna::parsing::resolution::ProjectResolutionEnhancer;
use codanna::parsing::typescript::resolution::TypeScriptProjectEnhancer;
use codanna::parsing::{Import, LanguageId, ParserFactory, PipelineSymbolCache};
use codanna::types::{FileId, Range, SymbolId};
use codanna::{Symbol, SymbolKind, Visibility};
use std::path::Path;
use std::sync::Arc;

/// Serialize the resolver-rules fixture instead of hand-formatting it:
/// a native Windows path interpolated into a JSON string parses as
/// invalid escape sequences and the load fails.
fn resolution_rules_json(tsconfig: &Path, mapping_root: &Path) -> String {
    let tsconfig_str = tsconfig.to_string_lossy().to_string();
    let mut hashes = serde_json::Map::new();
    hashes.insert(tsconfig_str.clone(), serde_json::json!("test"));
    let mut mappings = serde_json::Map::new();
    mappings.insert(
        format!("{}/**/*.ts", mapping_root.display()),
        serde_json::json!(tsconfig_str),
    );
    let mut rules = serde_json::Map::new();
    rules.insert(
        tsconfig_str,
        serde_json::json!({
            "baseUrl": ".",
            "paths": {"@components/*": ["src/components/*"]}
        }),
    );
    serde_json::to_string_pretty(&serde_json::json!({
        "version": "1.0",
        "hashes": hashes,
        "mappings": mappings,
        "rules": rules,
    }))
    .expect("serialize resolution rules")
}

/// Test that PipelineSymbolCache multi-tier resolution works for local symbols.
#[test]
fn test_pipeline_cache_local_resolution() {
    let cache = SymbolLookupCache::new();

    // Add local symbol
    let file_id = FileId::new(1).unwrap();
    let mut sym = Symbol::new(
        SymbolId::new(1).unwrap(),
        "helper",
        SymbolKind::Function,
        file_id,
        Range::new(5, 0, 10, 1),
    );
    sym.language_id = Some(LanguageId::new("typescript"));
    cache.insert(sym);

    // Resolve from same file - should find local
    let caller = CallerContext::from_file(file_id, LanguageId::new("typescript"));
    let result = cache.resolve("helper", &caller, Some(&Range::new(15, 0, 15, 10)), &[]);

    assert_eq!(
        result,
        codanna::parsing::ResolveResult::Found(SymbolId::new(1).unwrap()),
        "Local symbol should resolve"
    );
}

/// Test that PipelineSymbolCache filters by language.
#[test]
fn test_pipeline_cache_language_filter() {
    let cache = SymbolLookupCache::new();

    // Add same-named symbols in different languages
    // Set visibility to Public for cross-file resolution
    let mut ts_sym = Symbol::new(
        SymbolId::new(1).unwrap(),
        "parse",
        SymbolKind::Function,
        FileId::new(1).unwrap(),
        Range::new(5, 0, 10, 1),
    );
    ts_sym.language_id = Some(LanguageId::new("typescript"));
    ts_sym.visibility = Visibility::Public;
    cache.insert(ts_sym);

    let mut py_sym = Symbol::new(
        SymbolId::new(2).unwrap(),
        "parse",
        SymbolKind::Function,
        FileId::new(2).unwrap(),
        Range::new(5, 0, 10, 1),
    );
    py_sym.language_id = Some(LanguageId::new("python"));
    py_sym.visibility = Visibility::Public;
    cache.insert(py_sym);

    // Resolve from TypeScript file - should find only TypeScript symbol
    let caller = CallerContext::from_file(FileId::new(3).unwrap(), LanguageId::new("typescript"));
    let result = cache.resolve("parse", &caller, None, &[]);

    assert_eq!(
        result,
        codanna::parsing::ResolveResult::Found(SymbolId::new(1).unwrap()),
        "Should resolve to TypeScript symbol, not Python"
    );
}

/// Test build_resolution_context_with_pipeline_cache with local symbols.
#[test]
fn test_behavior_pipeline_cache_local_symbols() {
    let settings = Settings::default();
    let factory = ParserFactory::new(Arc::new(settings));
    let behavior = factory
        .create_behavior_from_registry(LanguageId::new("typescript"))
        .unwrap();

    let cache = SymbolLookupCache::new();
    let file_id = FileId::new(1).unwrap();

    // Add local symbol
    let mut sym = Symbol::new(
        SymbolId::new(1).unwrap(),
        "localHelper",
        SymbolKind::Function,
        file_id,
        Range::new(5, 0, 10, 1),
    );
    sym.language_id = Some(LanguageId::new("typescript"));
    sym.visibility = Visibility::Public;
    cache.insert(sym);

    // Build resolution context (no imports) - returns (scope, enhanced_imports)
    let extensions = &["ts", "tsx", "js", "jsx"];
    let (scope, _enhanced_imports) =
        behavior.build_resolution_context_with_pipeline_cache(file_id, &[], &cache, extensions);

    // Should resolve local symbol
    let resolved = scope.resolve("localHelper");
    assert_eq!(
        resolved,
        Some(SymbolId::new(1).unwrap()),
        "Local symbol should resolve in pipeline context"
    );
}

/// Test that path alias enhancement works with TypeScriptProjectEnhancer.
#[test]
fn test_path_alias_enhancement() {
    // Build rules like tsconfig would provide
    let rules = codanna::project_resolver::persist::ResolutionRules {
        base_url: None,
        paths: vec![
            (
                "@/components/*".to_string(),
                vec!["./src/components/*".to_string()],
            ),
            ("@/utils/*".to_string(), vec!["./src/utils/*".to_string()]),
            ("@/*".to_string(), vec!["./src/*".to_string()]),
        ]
        .into_iter()
        .collect(),
    };

    let enhancer = TypeScriptProjectEnhancer::new(rules);
    let file_id = FileId::new(1).unwrap();

    // Test alias enhancement
    let enhanced = enhancer.enhance_import_path("@/components/Button", file_id);
    assert_eq!(
        enhanced,
        Some("./src/components/Button".to_string()),
        "@/components/* should resolve"
    );

    let enhanced = enhancer.enhance_import_path("@/utils/helpers", file_id);
    assert_eq!(
        enhanced,
        Some("./src/utils/helpers".to_string()),
        "@/utils/* should resolve"
    );

    // Relative paths should not be enhanced
    let enhanced = enhancer.enhance_import_path("./local", file_id);
    assert_eq!(enhanced, None, "Relative paths should not be enhanced");
}

/// Test that the pipeline cache handles import resolution.
#[test]
fn test_pipeline_cache_import_resolution() {
    let cache = SymbolLookupCache::new();

    // Add symbol that matches an import
    let button_file = FileId::new(100).unwrap();
    let mut button_sym = Symbol::new(
        SymbolId::new(1).unwrap(),
        "Button",
        SymbolKind::Function,
        button_file,
        Range::new(10, 0, 20, 1),
    );
    button_sym.language_id = Some(LanguageId::new("typescript"));
    button_sym.module_path = Some("components/Button".into());
    button_sym.visibility = Visibility::Public;
    cache.insert(button_sym);

    // Create import that should match
    let test_file = FileId::new(1).unwrap();
    let imports = vec![Import {
        file_id: test_file,
        path: "./components/Button".to_string(), // Enhanced path
        imported_name: None,
        alias: None,
        is_glob: false,
        is_type_only: false,
    }];

    // Resolve "Button" with import context
    let caller = CallerContext::from_file(test_file, LanguageId::new("typescript"));
    let result = cache.resolve("Button", &caller, None, &imports);

    assert_eq!(
        result,
        codanna::parsing::ResolveResult::Found(SymbolId::new(1).unwrap()),
        "a single matching import must not be ambiguous"
    );
}

/// Creates isolated test environment with its own .codanna, tsconfig, and source files.
/// Tests that build_resolution_context_with_pipeline_cache resolves path aliases correctly.
#[test]
fn test_behavior_pipeline_cache_isolated() {
    use std::fs;
    use tempfile::TempDir;

    // Create isolated test environment
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Create directory structure
    let codanna_dir = temp_path.join(".codanna/index/resolvers");
    let src_dir = temp_path.join("src/components");
    fs::create_dir_all(&codanna_dir).expect("Failed to create .codanna");
    fs::create_dir_all(&src_dir).expect("Failed to create src/components");

    // Create tsconfig.json
    let tsconfig = r#"{
        "compilerOptions": {
            "baseUrl": ".",
            "paths": {
                "@components/*": ["src/components/*"]
            }
        }
    }"#;
    fs::write(temp_path.join("tsconfig.json"), tsconfig).expect("Failed to write tsconfig");

    // Create Button.ts
    let button_source = "export function Button() { return 'button'; }";
    fs::write(src_dir.join("Button.ts"), button_source).expect("Failed to write Button.ts");

    // Create resolution rules (what TypeScriptProvider would persist)
    let resolution_rules = resolution_rules_json(&temp_path.join("tsconfig.json"), temp_path);
    fs::write(
        codanna_dir.join("typescript_resolution.json"),
        resolution_rules,
    )
    .expect("Failed to write resolution rules");

    // Now run the actual test
    use codanna::parsing::typescript::behavior::TypeScriptBehavior;
    use codanna::parsing::{LanguageBehavior, TypeScriptParser};
    use codanna::types::SymbolCounter;

    let behavior = TypeScriptBehavior::with_resolution_dir(temp_path.join(".codanna"));
    let button_path = src_dir.join("Button.ts");

    // Parse the file
    let source = fs::read_to_string(&button_path).expect("Failed to read Button.ts");
    let mut parser = TypeScriptParser::new().expect("Failed to create parser");
    let button_file = FileId::new(100).unwrap();
    let mut counter = SymbolCounter::new();
    let symbols = parser.parse(&source, button_file, &mut counter);

    // Compute module path using the API
    let extensions = &["ts", "tsx", "js", "jsx"];
    let module_path = behavior.module_path_from_file(&button_path, temp_path, extensions);
    assert!(
        module_path.is_some(),
        "module_path_from_file should return a path for Button.ts"
    );

    // Populate cache
    let cache = SymbolLookupCache::new();
    for mut symbol in symbols {
        symbol.language_id = Some(LanguageId::new("typescript"));
        behavior.configure_symbol(&mut symbol, module_path.as_deref());
        cache.insert(symbol);
    }

    // Require the exact parsed target rather than permitting ambiguity.
    let button_candidates = cache.lookup_candidates("Button");
    assert_eq!(button_candidates.len(), 1, "exactly one parsed Button");
    let expected_button = button_candidates[0];

    // Test resolution with path alias import
    let app_file = FileId::new(1).unwrap();
    let app_path = temp_path.join("src/app.ts");
    fs::write(&app_path, "import { Button } from '@components/Button';").unwrap();
    behavior.register_file(app_path, app_file, "src.app".to_owned());
    let imports = vec![Import {
        file_id: app_file,
        path: "@components/Button".to_string(),
        imported_name: None,
        alias: None,
        is_glob: false,
        is_type_only: false,
    }];

    let extensions = &["ts", "tsx", "js", "jsx"];
    let (scope, enhanced_imports) = behavior
        .build_resolution_context_with_pipeline_cache(app_file, &imports, &cache, extensions);

    // Verify the enhanced imports have the path alias resolved
    assert_eq!(enhanced_imports.len(), 1);
    assert_eq!(enhanced_imports[0].path, "src.components.Button");

    let resolved = scope.resolve("Button");

    assert_eq!(
        resolved,
        Some(expected_button),
        "alias resolves to the parsed Button"
    );
}

/// Out-of-tree regression: module_path_from_file must resolve when the
/// indexed tree (and its persisted absolute mapping globs) lies outside the
/// workspace holding `.codanna`. The pre-fix code stripped the file path to
/// workspace-relative before the glob lookup, which fails against absolute
/// globs and silently nulls every TypeScript module path
/// (wiki: story-bug-core-provider-config-lookup-out-of-tree).
#[test]
fn test_module_path_from_file_out_of_tree_absolute_mappings() {
    use std::fs;
    use tempfile::TempDir;

    let workspace = TempDir::new().expect("workspace dir");
    let repo = TempDir::new().expect("repo dir");

    let resolvers_dir = workspace.path().join(".codanna/index/resolvers");
    fs::create_dir_all(&resolvers_dir).expect("create resolvers dir");
    let src_dir = repo.path().join("src/components");
    fs::create_dir_all(&src_dir).expect("create src");
    fs::write(
        src_dir.join("Button.ts"),
        "export function Button() { return 'button'; }",
    )
    .expect("write Button.ts");

    let tsconfig_path = repo.path().join("tsconfig.json");
    fs::write(
        &tsconfig_path,
        r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@components/*": ["src/components/*"]}}}"#,
    )
    .expect("write tsconfig");

    // What TypeScriptProvider persists for an absolute config_files entry:
    // absolute mapping globs keyed to the absolute tsconfig path.
    let rules = resolution_rules_json(&tsconfig_path, repo.path());
    fs::write(resolvers_dir.join("typescript_resolution.json"), rules).expect("write rules");

    use codanna::parsing::LanguageBehavior;
    use codanna::parsing::typescript::behavior::TypeScriptBehavior;

    let behavior = TypeScriptBehavior::with_resolution_dir(workspace.path().join(".codanna"));
    let module_path = behavior.module_path_from_file(
        &src_dir.join("Button.ts"),
        repo.path(),
        &["ts", "tsx", "js", "jsx"],
    );

    assert_eq!(
        module_path.as_deref(),
        Some("src.components.Button"),
        "absolute file path must match absolute mapping globs out-of-tree"
    );
}

/// Test three-level visibility model via CallerContext.
///
/// Visibility rules:
/// 1. Same file = always visible (even Private)
/// 2. Same module = always visible (even Private)
/// 3. Different module = must be Public
#[test]
fn test_caller_context_visibility_model() {
    let cache = SymbolLookupCache::new();
    let lang = LanguageId::new("typescript");

    // File 1: src/components/Button.ts (module: src.components)
    let file1 = FileId::new(1).unwrap();
    let mut public_button = Symbol::new(
        SymbolId::new(1).unwrap(),
        "Button",
        SymbolKind::Function,
        file1,
        Range::new(1, 0, 10, 1),
    );
    public_button.language_id = Some(lang);
    public_button.module_path = Some("src.components".into());
    public_button.visibility = Visibility::Public;
    cache.insert(public_button);

    // File 1: Private helper in same file
    let mut private_helper = Symbol::new(
        SymbolId::new(2).unwrap(),
        "helper",
        SymbolKind::Function,
        file1,
        Range::new(15, 0, 20, 1),
    );
    private_helper.language_id = Some(lang);
    private_helper.module_path = Some("src.components".into());
    private_helper.visibility = Visibility::Private;
    cache.insert(private_helper);

    // File 2: src/components/Icon.ts (same module: src.components)
    let file2 = FileId::new(2).unwrap();
    let mut private_icon = Symbol::new(
        SymbolId::new(3).unwrap(),
        "Icon",
        SymbolKind::Function,
        file2,
        Range::new(1, 0, 10, 1),
    );
    private_icon.language_id = Some(lang);
    private_icon.module_path = Some("src.components".into());
    private_icon.visibility = Visibility::Private;
    cache.insert(private_icon);

    // File 3: src/utils/format.ts (different module: src.utils)
    let file3 = FileId::new(3).unwrap();
    let mut private_format = Symbol::new(
        SymbolId::new(4).unwrap(),
        "format",
        SymbolKind::Function,
        file3,
        Range::new(1, 0, 10, 1),
    );
    private_format.language_id = Some(lang);
    private_format.module_path = Some("src.utils".into());
    private_format.visibility = Visibility::Private;
    cache.insert(private_format);

    // Test 1: Same file - Private helper visible
    let caller_same_file = CallerContext::new(file1, Some("src.components".into()), lang, ".");
    let result = cache.resolve("helper", &caller_same_file, None, &[]);
    assert_eq!(
        result,
        codanna::parsing::ResolveResult::Found(SymbolId::new(2).unwrap()),
        "Private symbol should be visible from same file"
    );

    // Test 2: Same module, different file - Private Icon visible
    let caller_same_module = CallerContext::new(
        file1,                         // calling from file1
        Some("src.components".into()), // same module as Icon
        lang,
        ".",
    );
    let result = cache.resolve("Icon", &caller_same_module, None, &[]);
    assert_eq!(
        result,
        codanna::parsing::ResolveResult::Found(SymbolId::new(3).unwrap()),
        "Private symbol should be visible from same module"
    );

    // Test 3: Different module - Private format NOT visible
    let caller_diff_module = CallerContext::new(
        file1,                         // calling from file1
        Some("src.components".into()), // different from src.utils
        lang,
        ".",
    );
    let result = cache.resolve("format", &caller_diff_module, None, &[]);
    assert_eq!(
        result,
        codanna::parsing::ResolveResult::NotFound,
        "Private symbol should NOT be visible from different module"
    );

    // Test 4: Different module - Public Button IS visible
    let caller_from_utils = CallerContext::new(
        file3,                    // calling from utils
        Some("src.utils".into()), // different from src.components
        lang,
        ".",
    );
    let result = cache.resolve("Button", &caller_from_utils, None, &[]);
    assert_eq!(
        result,
        codanna::parsing::ResolveResult::Found(SymbolId::new(1).unwrap()),
        "Public symbol should be visible from different module"
    );
}
