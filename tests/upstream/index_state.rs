//! Fork adaptations of upstream's missing-root and deferred-writer regressions.
use codanna::indexing::{
    facade::IndexFacade,
    pipeline::{PendingResolution, SymbolLookupCache},
};
use codanna::parsing::resolution::{FilePresence, RelativeImportLookup};
use codanna::parsing::{LanguageBehavior, PipelineSymbolCache, TypeScriptBehavior};
use codanna::{
    IndexPersistence, RelationKind, Settings,
    storage::{DocumentIndex, IndexMetadata},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

fn fixture() -> (tempfile::TempDir, Arc<Settings>, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let tests = dir.path().join("tests");
    std::fs::create_dir_all(src.join("pkg")).unwrap();
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(src.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(src.join("pkg/mod.py"), "def callee():\n    return 42\n").unwrap();
    std::fs::write(
        tests.join("caller.py"),
        "from pkg.mod import callee\n\ndef caller():\n    return callee()\n",
    )
    .unwrap();
    let mut settings = Settings {
        index_path: dir.path().join("index"),
        workspace_root: None,
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(src.clone()).unwrap();
    settings.add_indexed_path(tests.clone()).unwrap();
    (dir, Arc::new(settings), src, tests)
}

fn assert_call(facade: &IndexFacade) {
    let callers = facade.find_symbols_by_name("caller", None);
    assert_eq!(callers.len(), 1);
    let rows = facade
        .document_index()
        .get_relationships_from(callers[0].id, RelationKind::Calls)
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "one durable edge, not replayed duplicates: {rows:?}"
    );
    assert_eq!(
        facade.get_symbol(rows[0].1).unwrap().name.as_ref(),
        "callee"
    );
}

#[test]
fn upstream_repair_missing_root_metadata_does_not_duplicate_existing_files() {
    let (_dir, settings, src, tests) = fixture();
    let mut facade = IndexFacade::new(Arc::clone(&settings)).unwrap();
    facade
        .index_directories_with_options(&[src.clone(), tests.clone()], false, false, false, None)
        .unwrap();
    let before = (
        facade.symbol_count(),
        facade.relationship_count(),
        facade.find_symbols_by_name("caller", None)[0].id,
    );
    for _ in 0..2 {
        facade
            .sync_with_config(
                Some(vec![src.clone()]),
                &[src.clone(), tests.clone()],
                false,
            )
            .unwrap();
        assert_eq!(
            (
                facade.symbol_count(),
                facade.relationship_count(),
                facade.find_symbols_by_name("caller", None)[0].id
            ),
            before
        );
        assert_call(&facade);
    }
    let persistence = IndexPersistence::new(settings.index_path.clone());
    let semantic = settings.index_path.join("semantic");
    std::fs::create_dir_all(&semantic).unwrap();
    std::fs::write(semantic.join("sentinel"), b"do not rewrite vectors").unwrap();
    persistence.save_metadata(&facade).unwrap();
    let metadata = IndexMetadata::load(&settings.index_path).unwrap();
    let mut actual = metadata.indexed_paths.unwrap();
    actual.sort();
    let mut expected = vec![src.clone(), tests.clone()];
    expected.sort();
    assert_eq!(actual, expected);
    assert_eq!(
        std::fs::read(semantic.join("sentinel")).unwrap(),
        b"do not rewrite vectors"
    );
    drop(facade);
    let mut reopened = persistence.load_facade_lite(Arc::clone(&settings)).unwrap();
    reopened
        .sync_with_config(Some(expected.clone()), &expected, false)
        .unwrap();
    assert_call(&reopened);
    std::fs::write(
        tests.join("caller.py"),
        "from pkg.mod import callee\n\ndef caller():\n    return callee() + 1\n",
    )
    .unwrap();
    reopened.index_directory(&tests, false).unwrap();
    assert_call(&reopened);
}

#[test]
fn upstream_repair_writer_start_failure_retains_pending_and_preserves_ids_on_retry() {
    let (_dir, settings, src, tests) = fixture();
    let mut facade = IndexFacade::new(Arc::clone(&settings)).unwrap();
    let mut pending = PendingResolution::default();
    facade
        .index_directory_deferred(&tests, false, &mut pending)
        .unwrap();
    facade
        .index_directory_deferred(&src, false, &mut pending)
        .unwrap();
    let caller = facade.find_symbols_by_name("caller", None)[0].id;
    let other = DocumentIndex::new(settings.index_path.join("tantivy"), &settings).unwrap();
    other.start_batch().unwrap();
    let result = facade.resolve_deferred(&mut pending);
    other.rollback_batch().unwrap(); // Always release the external writer before assertions.
    let error = result.expect_err("a busy writer must not become successful zero-row resolution");
    assert!(error.is_writer_unavailable(), "unexpected error: {error}");
    assert!(
        facade
            .document_index()
            .get_relationships_from(caller, RelationKind::Calls)
            .unwrap()
            .is_empty()
    );
    facade.resolve_deferred(&mut pending).unwrap();
    assert_eq!(
        facade.find_symbols_by_name("caller", None)[0].id,
        caller,
        "retry must not rerun Phase 1"
    );
    assert_call(&facade);
    // Successful work is drained, so an accidental second call cannot append rows.
    facade.resolve_deferred(&mut pending).unwrap();
    assert_call(&facade);
}

#[test]
fn upstream_repair_complete_inventory_includes_symbol_free_files_and_is_path_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("src");
    std::fs::create_dir_all(root.join("a/dir")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    for file in [
        "a/empty.ts",
        "a/service.client.ts",
        "a/dir/index.ts",
        "b/target.ts",
    ] {
        std::fs::write(root.join(file), "// intentionally has no symbols\n").unwrap();
    }
    let mut settings = Settings {
        index_path: dir.path().join("index"),
        workspace_root: None,
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.clone()).unwrap();
    let mut facade = IndexFacade::new(Arc::new(settings)).unwrap();
    facade.index_directory(&root, false).unwrap();
    let cache = SymbolLookupCache::from_index(facade.document_index()).unwrap();
    let normalize = |path: &Path| {
        facade
            .document_index()
            .to_portable_file_path(&path.to_string_lossy())
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf())
    };
    let importer = normalize(&root.join("a/caller.ts"));
    let behavior = TypeScriptBehavior::new();
    let lookup = |specifier| {
        behavior.relative_import_lookup(
            &cache,
            "missingSymbol",
            specifier,
            importer.to_str().unwrap(),
            &["ts", "tsx"],
        )
    };
    assert_eq!(
        cache.file_presence(&normalize(&root.join("a/empty.ts"))),
        FilePresence::Present
    );
    assert_eq!(
        lookup("./empty"),
        RelativeImportLookup::Unknown,
        "file without a symbol is not absent"
    );
    assert_eq!(
        lookup("./service.js"),
        RelativeImportLookup::Unknown,
        "dotted same-stem evidence must survive"
    );
    assert_eq!(
        lookup("./dir"),
        RelativeImportLookup::Unknown,
        "directory/index evidence must survive"
    );
    assert_eq!(
        lookup("./target"),
        RelativeImportLookup::NoIndexedFile,
        "other root's target does not prove this file exists"
    );
    let partial = SymbolLookupCache::new();
    assert_eq!(
        behavior.relative_import_lookup(
            &partial,
            "missingSymbol",
            "./target",
            importer.to_str().unwrap(),
            &["ts"]
        ),
        RelativeImportLookup::Unknown
    );
}

#[test]
fn upstream_repair_effective_config_preserves_and_overrides_relative_redirects() {
    for javascript in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base.json");
        let child = dir.path().join("child.json");
        std::fs::write(&base, r#"{"compilerOptions":{"rootDirs":["src","generated"],"moduleSuffixes":[".native",""]}}"#).unwrap();
        std::fs::write(&child, r#"{"extends":"./base.json","compilerOptions":{}}"#).unwrap();
        let options = |path: &Path| -> (Option<Vec<String>>, Option<Vec<String>>) {
            if javascript {
                let config = codanna::parsing::javascript::jsconfig::resolve_extends_chain(
                    path,
                    &mut Default::default(),
                )
                .unwrap();
                (
                    config.compilerOptions.rootDirs,
                    config.compilerOptions.moduleSuffixes,
                )
            } else {
                let config = codanna::parsing::typescript::tsconfig::resolve_extends_chain(
                    path,
                    &mut Default::default(),
                )
                .unwrap();
                (
                    config.compilerOptions.rootDirs,
                    config.compilerOptions.moduleSuffixes,
                )
            }
        };
        let (roots, suffixes) = options(&child);
        assert_eq!(roots.unwrap(), ["src", "generated"]);
        assert_eq!(suffixes.unwrap(), [".native", ""]);
        std::fs::write(
            &child,
            r#"{"extends":"./base.json","compilerOptions":{"rootDirs":[],"moduleSuffixes":[]}}"#,
        )
        .unwrap();
        assert_eq!(
            options(&child),
            (Some(vec![]), Some(vec![])),
            "explicit [] overrides inheritance"
        );
    }
}
