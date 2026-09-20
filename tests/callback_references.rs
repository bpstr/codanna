//! Callback registrations retain references without inventing immediate calls.
use codanna::indexing::facade::IndexFacade;
use codanna::{RelationKind, Settings};
use std::sync::Arc;

fn index(files: &[(&str, &str)]) -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    for (name, content) in files {
        std::fs::write(source.join(name), content).unwrap();
    }
    let mut settings = Settings {
        workspace_root: Some(source.clone()),
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(source.clone()).unwrap();
    let mut facade = IndexFacade::new(Arc::new(settings)).unwrap();
    facade.index_directory(&source, true).unwrap();
    (temp, facade)
}

#[test]
fn registered_route_handler_is_a_reference_with_source_evidence() {
    for extension in ["ts", "js"] {
        let source =
            "function handle() {}\nfunction install(router) { router.get('/health', handle); }";
        let (_temp, facade) = index(&[(&format!("routes.{extension}"), source)]);
        let install = facade.find_symbols_by_name("install", None).remove(0);
        let handle = facade.find_symbols_by_name("handle", None).remove(0);
        let references = facade
            .document_index()
            .get_relationships_from(install.id, RelationKind::References)
            .unwrap();
        assert_eq!(references.len(), 1, "{extension} registration reference");
        assert_eq!(references[0].1, handle.id);
        assert!(
            facade.get_dependencies(install.id)[&RelationKind::References]
                .iter()
                .any(|symbol| symbol.id == handle.id)
        );
        assert!(
            facade.get_dependents(handle.id)[&RelationKind::References]
                .iter()
                .any(|symbol| symbol.id == install.id)
        );
        assert!(
            facade
                .get_impact_radius(handle.id, Some(1))
                .contains(&install.id)
        );
        let metadata = references[0].2.metadata.as_ref().unwrap();
        assert_eq!(metadata.context.as_deref(), Some("argument_reference"));
        assert_eq!(metadata.line, Some(1));
        assert_eq!(
            metadata.column,
            Some(source.lines().nth(1).unwrap().find("handle").unwrap() as u32)
        );
        assert!(
            facade
                .get_called_functions(install.id)
                .iter()
                .all(|callee| callee.id != handle.id)
        );
    }
}

#[test]
fn local_bindings_do_not_reference_same_named_global_handler() {
    for binding in [
        "const handler = () => {}; router.get('/x', handler);",
        "const { handler } = other; router.get('/x', handler);",
        "const [handler] = other; router.get('/x', handler);",
        "try {} catch (handler) { router.get('/x', handler); }",
    ] {
        let source =
            format!("function handler() {{}}\nfunction install(router, other) {{ {binding} }}");
        let (_temp, facade) = index(&[("shadow.ts", &source)]);
        let install = facade.find_symbols_by_name("install", None).remove(0);
        let global = facade
            .find_symbols_by_name("handler", None)
            .into_iter()
            .find(|symbol| symbol.range.start_line == 0)
            .unwrap();
        let references = facade
            .document_index()
            .get_relationships_from(install.id, RelationKind::References)
            .unwrap();
        assert!(
            references.iter().all(|(_, target, _)| *target != global.id),
            "local binding must block a global name guess: {binding}"
        );
    }
}

#[test]
fn imported_callback_alias_retains_provider_identity() {
    let (_temp, facade) = index(&[
        ("handler.ts", "export function actualHandler() {}"),
        ("decoy.ts", "export function callback() {}"),
        (
            "client.ts",
            "import { actualHandler as callback } from './handler';\nexport function install(router) { router.get('/x', callback); }",
        ),
    ]);
    let install = facade.find_symbols_by_name("install", None).remove(0);
    let actual = facade.find_symbols_by_name("actualHandler", None).remove(0);
    let references = facade
        .document_index()
        .get_relationships_from(install.id, RelationKind::References)
        .unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].1, actual.id);
    assert!(
        facade
            .get_called_functions(install.id)
            .iter()
            .all(|callee| callee.id != actual.id)
    );
}

#[test]
fn parameter_shadow_does_not_reference_same_named_global_handler() {
    for parameters in [
        "router, handler",
        "router: unknown, handler: () => void",
        "router, {handler}",
    ] {
        let source = format!(
            "function handler() {{}}\nfunction install({parameters}) {{ router.get('/x', handler); }}"
        );
        let (_temp, facade) = index(&[("shadow.ts", &source)]);
        let install = facade.find_symbols_by_name("install", None).remove(0);
        let references = facade
            .document_index()
            .get_relationships_from(install.id, RelationKind::References)
            .unwrap();
        assert!(
            references.is_empty(),
            "parameter evidence must block a global name guess: {parameters}"
        );
    }
}

#[test]
fn passing_strings_or_members_does_not_invent_callback_targets() {
    let (_temp, facade) = index(&[(
        "routes.ts",
        "function handler() {}\nfunction install(router, object) { router.get('/a', 'handler'); router.get('/b', object.handler); }",
    )]);
    let install = facade.find_symbols_by_name("install", None).remove(0);
    assert!(
        facade
            .document_index()
            .get_relationships_from(install.id, RelationKind::References)
            .unwrap()
            .is_empty()
    );
}
