use codanna::indexing::pipeline::{FileContent, init_parser_cache, parse_file};
use codanna::{RelationKind, Settings};
use std::sync::Arc;

fn calls(source: &str, caller: &str) -> Vec<(String, Option<String>, u32)> {
    calls_in_file(source, caller, "calls.rs")
}

fn calls_in_file(source: &str, caller: &str, file: &str) -> Vec<(String, Option<String>, u32)> {
    let settings = Arc::new(Settings::default());
    init_parser_cache(settings.clone());
    parse_file(
        FileContent::new(file.into(), source.to_owned(), "fixture".into()),
        &settings,
    )
    .expect("fixture must parse")
    .raw_relationships
    .into_iter()
    .filter(|r| r.kind == RelationKind::Calls && r.from_name.as_ref() == caller)
    .map(|r| {
        (
            r.to_name.to_string(),
            r.metadata.and_then(|m| m.receiver.map(|v| v.to_string())),
            r.to_range.start_column,
        )
    })
    .collect()
}

#[test]
fn same_line_free_and_associated_function_both_survive() {
    let src = "struct Boxed; impl Boxed { fn make() -> Self { Boxed } }\nfn make<T>(_: T) {}\nfn entry() { make(Boxed::make()); }";
    let actual = calls(src, "entry");
    assert_eq!(
        actual.len(),
        2,
        "free make and Boxed::make are distinct call sites: {actual:?}"
    );
    assert!(
        actual
            .iter()
            .any(|(name, receiver, _)| name == "make" && receiver.is_none())
    );
    assert!(
        actual
            .iter()
            .any(|(name, receiver, _)| name == "make" && receiver.as_deref() == Some("Boxed"))
    );
}

#[test]
fn different_line_control_preserves_both_calls() {
    let src = "struct Boxed; impl Boxed { fn make() -> Self { Boxed } }\nfn make<T>(_: T) {}\nfn entry() {\n make(\n Boxed::make()\n );\n}";
    assert_eq!(calls(src, "entry").len(), 2);
}

#[test]
fn separate_repeated_call_sites_keep_both_columns() {
    let src = "fn ping() {}\nfn entry() { ping(); ping(); }";
    let actual = calls(src, "entry");
    assert_eq!(
        actual.len(),
        2,
        "two call sites must retain two evidence positions: {actual:?}"
    );
    assert_ne!(actual[0].2, actual[1].2);
}

#[test]
fn generated_line_after_64k_keeps_exact_source_column() {
    let src = format!(
        "fn ping() {{}} fn entry() {{ {}ping(); }}",
        " ".repeat(65_536)
    );
    let expected_column = src.rfind("ping()").unwrap();
    let actual = calls(&src, "entry");
    assert_eq!(actual.len(), 1, "generated call must be extracted");
    assert_eq!(
        actual[0].2 as usize, expected_column,
        "source evidence must not wrap after 65,535 UTF-8 bytes on a line"
    );
}

#[test]
fn typescript_same_line_free_and_static_calls_both_survive() {
    let src = "class Boxed { static make() { return 1; } }\nfunction make(value: unknown) {}\nfunction entry() { make(Boxed.make()); }";
    let actual = calls_in_file(src, "entry", "calls.ts");
    println!("TypeScript same-line call records: {actual:?}");
    assert_eq!(
        actual.len(),
        2,
        "same name and line are not the same call site: {actual:?}"
    );
    assert!(
        actual
            .iter()
            .any(|(name, receiver, _)| name == "make" && receiver.is_none()),
        "the free make() site must survive: {actual:?}"
    );
    assert!(
        actual
            .iter()
            .any(|(name, receiver, _)| name == "make" && receiver.as_deref() == Some("Boxed")),
        "the static Boxed.make() site must survive: {actual:?}"
    );
}

#[test]
fn typescript_split_line_calls_have_no_duplicate_channel_record() {
    let src = "class Boxed { static make() { return 1; } }\nfunction make(value: unknown) {}\nfunction entry() {\n make(\n Boxed.make()\n );\n}";
    let actual = calls_in_file(src, "entry", "calls.ts");
    println!("TypeScript split-line call records: {actual:?}");
    assert_eq!(
        actual.len(),
        2,
        "one physical call must not become two channel records: {actual:?}"
    );
}

#[test]
fn typescript_formatting_does_not_change_resolved_graph() {
    use codanna::indexing::facade::IndexFacade;
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(
        src.join("calls.ts"),
        include_str!("corpus/same_line_calls/calls.ts"),
    )
    .unwrap();
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        workspace_root: Some(src.clone()),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(src.clone()).unwrap();
    let mut facade = IndexFacade::new(Arc::new(settings)).unwrap();
    facade.index_directory(&src, true).unwrap();
    let targets = |name: &str| {
        let callers = facade.find_symbols_by_name(name, None);
        assert_eq!(callers.len(), 1, "fixture must index caller {name}");
        let mut result: Vec<_> = facade
            .get_called_functions(callers[0].id)
            .into_iter()
            .map(|s| (s.name.to_string(), format!("{:?}", s.scope_context)))
            .collect();
        result.sort();
        result.dedup();
        result
    };
    let control = targets("splitLines");
    let same_line = targets("sameLine");
    println!("splitLines targets: {control:?}; sameLine targets: {same_line:?}");
    assert_eq!(
        control.len(),
        2,
        "split-line fixture must reach both actual functions"
    );
    assert_eq!(
        same_line, control,
        "formatting must not remove the free make() edge"
    );
}
