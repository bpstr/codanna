use codanna::project_resolver::persist::{ResolutionPersistence, ResolverBindingStatus};
use codanna::project_resolver::provider::ProjectResolutionProvider;
use codanna::project_resolver::providers::{
    go::GoProvider, python::PythonProvider, typescript::TypeScriptProvider,
};
use codanna::symbol::context::ContextIncludes;
use codanna::{
    Settings,
    indexing::{
        facade::IndexFacade,
        pipeline::{FileContent, init_parser_cache, parse_file},
    },
};
use std::{fs, path::Path, sync::Arc};

fn write(root: &Path, relative: &str, source: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

#[test]
fn f12_keeps_project_aliases_isolated_and_reports_missing_bindings() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for side in ["left", "right"] {
        write(
            root,
            &format!("{side}/src/owner.ts"),
            if side == "left" {
                include_str!("fixtures/retrieval_findings/f12/left/src/owner.ts")
            } else {
                include_str!("fixtures/retrieval_findings/f12/right/src/owner.ts")
            },
        );
        write(
            root,
            &format!("{side}/src/use.ts"),
            if side == "left" {
                include_str!("fixtures/retrieval_findings/f12/left/src/use.ts")
            } else {
                include_str!("fixtures/retrieval_findings/f12/right/src/use.ts")
            },
        );
        write(
            root,
            &format!("{side}/tsconfig.json"),
            if side == "left" {
                include_str!("fixtures/retrieval_findings/f12/left/tsconfig.json")
            } else {
                include_str!("fixtures/retrieval_findings/f12/right/tsconfig.json")
            },
        );
        write(
            root,
            &format!("{side}/go.mod"),
            if side == "left" {
                include_str!("fixtures/retrieval_findings/f12/left/go.mod.fixture")
            } else {
                include_str!("fixtures/retrieval_findings/f12/right/go.mod.fixture")
            },
        );
        write(
            root,
            &format!("{side}/pyproject.toml"),
            if side == "left" {
                include_str!("fixtures/retrieval_findings/f12/left/pyproject.toml")
            } else {
                include_str!("fixtures/retrieval_findings/f12/right/pyproject.toml")
            },
        );
        write(root, &format!("{side}/example.go"), "package fixture\n");
        write(root, &format!("{side}/example.py"), "# fixture\n");
    }
    write(
        root,
        "left-unbound/src/use.ts",
        include_str!("fixtures/retrieval_findings/f12/left-unbound/src/use.ts"),
    );

    let mut settings = Settings {
        workspace_root: Some(root.to_path_buf()),
        index_path: ".codanna/index".into(),
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.to_path_buf()).unwrap();
    for language in ["typescript", "go", "python"] {
        let extension = match language {
            "typescript" => "tsconfig.json",
            "go" => "go.mod",
            "python" => "pyproject.toml",
            _ => unreachable!(),
        };
        settings.languages.get_mut(language).unwrap().config_files = ["left", "right"]
            .map(|side| root.join(side).join(extension))
            .to_vec();
    }

    TypeScriptProvider::new().rebuild_cache(&settings).unwrap();
    GoProvider::new().rebuild_cache(&settings).unwrap();
    PythonProvider::new().rebuild_cache(&settings).unwrap();

    let persistence = ResolutionPersistence::new(&settings.resolution_dir());
    for side in ["left", "right"] {
        assert_eq!(
            persistence
                .diagnose_file("go", &root.join(side).join("example.go"))
                .unwrap()
                .unwrap()
                .status,
            ResolverBindingStatus::Bound
        );
        assert_eq!(
            persistence
                .diagnose_file("python", &root.join(side).join("example.py"))
                .unwrap()
                .unwrap()
                .status,
            ResolverBindingStatus::Bound
        );
    }

    let settings = Arc::new(settings);
    init_parser_cache(Arc::clone(&settings));
    let parsed = parse_file(
        FileContent::new(
            "left/src/owner.ts".into(),
            fs::read_to_string(root.join("left/src/owner.ts")).unwrap(),
            "fixture".into(),
        ),
        &settings,
    )
    .unwrap();
    assert_eq!(parsed.module_path.as_deref(), Some("src.owner"));

    let mut facade = IndexFacade::new(settings).unwrap();
    facade.index_directory(root, false).unwrap();

    for (caller, side) in [("useLeft", "left"), ("useRight", "right")] {
        let caller = facade.find_symbols_by_name(caller, None).pop().unwrap();
        let targets: Vec<_> = facade
            .get_called_functions(caller.id)
            .into_iter()
            .filter(|symbol| symbol.name.as_ref() == "identity")
            .collect();
        assert_eq!(targets.len(), 1, "{caller:?}: {targets:?}");
        assert!(targets[0].file_path.starts_with(side), "{targets:?}");
        let context = facade
            .get_symbol_context(caller.id, ContextIncludes::ALL)
            .unwrap();
        let binding = context.relationships.resolver_binding.as_ref().unwrap();
        assert_eq!(binding.status, ResolverBindingStatus::Bound);
        assert!(
            binding
                .config_path
                .as_ref()
                .is_some_and(|path| path.ends_with(Path::new(&format!("{side}/tsconfig.json"))))
        );
    }

    let unbound = facade
        .find_symbols_by_name("useUnbound", None)
        .pop()
        .unwrap();
    assert!(
        facade
            .get_called_functions(unbound.id)
            .iter()
            .all(|symbol| symbol.name.as_ref() != "identity")
    );
    let context = facade
        .get_symbol_context(unbound.id, ContextIncludes::ALL)
        .unwrap();
    assert_eq!(
        context
            .relationships
            .resolver_binding
            .as_ref()
            .unwrap()
            .status,
        ResolverBindingStatus::ResolverBindingsAbsent
    );
    assert_eq!(
        serde_json::to_value(&context).unwrap()["relationships"]["resolver_binding"]["status"],
        "resolver_bindings_absent"
    );
}
