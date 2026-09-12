//! Test TypeScript alias resolution without indexing
//!
//! Tests that TypeScript path aliases are properly enhanced during parsing

use codanna::FileId;
use codanna::parsing::resolution::ProjectResolutionEnhancer;
use codanna::parsing::typescript::behavior::TypeScriptBehavior;
use codanna::parsing::typescript::resolution::TypeScriptProjectEnhancer;
use codanna::parsing::{Import, LanguageBehavior};
use codanna::project_resolver::persist::{ResolutionIndex, ResolutionPersistence, ResolutionRules};

#[test]
fn test_import_enhancement_with_aliases() {
    // Create resolution rules like tsconfig would provide
    let rules = ResolutionRules {
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

    // Create enhancer with the rules
    let enhancer = TypeScriptProjectEnhancer::new(rules);
    let file_id = FileId::new(1).unwrap();

    // Test various alias patterns
    let test_cases = vec![
        ("@/components/Button", Some("./src/components/Button")),
        ("@/components/ui/dialog", Some("./src/components/ui/dialog")),
        ("@/utils/helpers", Some("./src/utils/helpers")),
        ("@/lib/api", Some("./src/lib/api")),
        ("./relative/path", None), // Relative paths should not be enhanced
        ("../parent/path", None),  // Parent paths should not be enhanced
        ("react", None),           // External packages should not be enhanced
    ];

    for (import_path, expected) in test_cases {
        let result = enhancer.enhance_import_path(import_path, file_id);

        match expected {
            Some(expected_path) => {
                assert_eq!(
                    result.as_deref(),
                    Some(expected_path),
                    "Import '{import_path}' should be enhanced to '{expected_path}'"
                );
            }
            None => {
                assert_eq!(
                    result, None,
                    "Import '{import_path}' should not be enhanced"
                );
            }
        }
    }

    println!("All import enhancements work correctly!");
}

/// Persist a complete deterministic mapping, using serialization rather than
/// interpolating native paths into JSON. No global directory or environment edits.
fn workspace(component_dir: &str) -> (tempfile::TempDir, TypeScriptBehavior) {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(component_dir)).unwrap();
    std::fs::write(dir.path().join("app.ts"), "export function app() {}\n").unwrap();
    let config = dir.path().join("tsconfig.json");
    std::fs::write(&config, "{}").unwrap();
    let mut index = ResolutionIndex::new();
    index.add_mapping(&format!("{}/**/*.ts", dir.path().display()), &config);
    index.set_rules(
        &config,
        ResolutionRules {
            base_url: None,
            paths: [(
                "@components/*".to_owned(),
                vec![format!("./{component_dir}/*")],
            )]
            .into(),
        },
    );
    ResolutionPersistence::new(&dir.path().join(".codanna"))
        .save("typescript", &index)
        .unwrap();
    let behavior = TypeScriptBehavior::with_resolution_dir(dir.path().join(".codanna"));
    behavior.register_file(
        dir.path().join("app.ts"),
        FileId::new(1).unwrap(),
        "app".to_owned(),
    );
    (dir, behavior)
}

#[test]
fn test_module_path_computation() {
    let (dir, behavior) = workspace("src/components");
    for (relative, expected) in [
        ("src/components/Button.ts", "src.components.Button"),
        ("src/components/index.ts", "src.components"),
        ("src/utils/helpers.ts", "src.utils.helpers"),
    ] {
        let file = dir.path().join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "export const fixture = 1;").unwrap();
        assert_eq!(
            behavior
                .module_path_from_file(&file, dir.path(), &["ts"])
                .as_deref(),
            Some(expected)
        );
    }
}

fn enhanced_path(behavior: &TypeScriptBehavior) -> String {
    let file_id = FileId::new(1).unwrap();
    let import = Import {
        file_id,
        path: "@components/Button".to_owned(),
        alias: None,
        is_glob: false,
        is_type_only: false,
    };
    let imports = [import];
    let cache = codanna::indexing::pipeline::types::SymbolLookupCache::new();
    let (_, enhanced) =
        behavior.build_resolution_context_with_pipeline_cache(file_id, &imports, &cache, &["ts"]);
    assert_eq!(enhanced.len(), 1);
    enhanced[0].path.clone()
}

#[test]
fn test_typescript_behavior_add_import() {
    let (_dir, behavior) = workspace("src/components");
    let file_id = FileId::new(1).unwrap();
    behavior.add_import(Import {
        file_id,
        path: "@components/Button".to_owned(),
        alias: None,
        is_glob: false,
        is_type_only: false,
    });
    let imports = behavior.get_imports_for_file(file_id);
    assert_eq!(imports.len(), 1);
    assert_eq!(
        imports[0].path, "@components/Button",
        "raw imports are stored unchanged"
    );
    assert_eq!(enhanced_path(&behavior), "src.components.Button");
}

#[test]
fn hardening_review_typescript_workspaces_do_not_share_alias_rules() {
    let original_dir = std::env::current_dir().unwrap();
    let (_first, a) = workspace("src/first");
    let (_second, b) = workspace("src/second");
    // Same-thread calls within the TTL catch the old thread-local cache leak.
    assert_eq!(enhanced_path(&a), "src.first.Button");
    assert_eq!(enhanced_path(&b), "src.second.Button");
    assert_eq!(enhanced_path(&a), "src.first.Button");
    std::thread::scope(|scope| {
        scope.spawn(|| assert_eq!(enhanced_path(&a), "src.first.Button"));
        scope.spawn(|| assert_eq!(enhanced_path(&b), "src.second.Button"));
    });
    assert_eq!(std::env::current_dir().unwrap(), original_dir);
}

#[test]
fn test_resolution_with_project_rules() {
    let (dir, behavior) = workspace("src/components");
    let index = ResolutionPersistence::new(&dir.path().join(".codanna"))
        .load("typescript")
        .unwrap();
    assert_eq!(index.rules.len(), 1);
    assert_eq!(
        index.get_config_for_file(&dir.path().join("app.ts")),
        Some(&dir.path().join("tsconfig.json"))
    );
    assert_eq!(enhanced_path(&behavior), "src.components.Button");

    // An unregistered file must not borrow whichever config a HashMap yields.
    let other = FileId::new(2).unwrap();
    let imports = [Import {
        file_id: other,
        path: "@components/Button".to_owned(),
        alias: None,
        is_glob: false,
        is_type_only: false,
    }];
    let cache = codanna::indexing::pipeline::types::SymbolLookupCache::new();
    let (_, enhanced) =
        behavior.build_resolution_context_with_pipeline_cache(other, &imports, &cache, &["ts"]);
    assert_eq!(enhanced[0].path, "@components.Button");
}
