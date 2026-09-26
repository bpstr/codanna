//! Workspace scopes must not alias independently indexed external checkouts.
//! Source-independent storage fixtures; no provider, model, or source-tree scan.

use codanna::indexing::pipeline::FileRegistration;
use codanna::parsing::LanguageId;
use codanna::storage::DocumentIndex;
use codanna::{FileId, Range, Settings, Symbol, SymbolId, SymbolKind};
use std::path::Path;

fn register(index: &DocumentIndex, id: u32, path: &Path, name: &str) {
    index
        .store_file_registration(&FileRegistration {
            path: path.into(),
            file_id: FileId::new(id).unwrap(),
            content_hash: "fixture".into(),
            language_id: LanguageId::new("rust"),
            timestamp: 0,
            mtime: 0,
        })
        .unwrap();
    let mut symbol = Symbol::new(
        SymbolId::new(id).unwrap(),
        name,
        SymbolKind::Function,
        FileId::new(id).unwrap(),
        Range::new(0, 0, 0, 20),
    );
    symbol.doc_comment = Some("calendar settings".into());
    index.index_symbol(&symbol, path.to_str().unwrap()).unwrap();
}

fn fixture() -> (tempfile::TempDir, DocumentIndex) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let external = root.join("external");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&external).unwrap();
    let settings = Settings {
        workspace_root: Some(workspace.clone()),
        indexed_paths_cache: vec![external.clone()],
        ..Default::default()
    };
    let index = DocumentIndex::new(temp.path().join("index"), &settings).unwrap();
    index.start_batch().unwrap();
    register(&index, 1, Path::new("src/active/local.rs"), "localCalendar");
    register(
        &index,
        2,
        &workspace.join("src/active/absolute.rs"),
        "absoluteLocalCalendar",
    );
    register(
        &index,
        3,
        &external.join("src/active/external.rs"),
        "externalCalendar",
    );
    register(
        &index,
        4,
        &external.join("src/reference/only.rs"),
        "externalOnlyCalendar",
    );
    index.commit_batch().unwrap();
    (temp, index)
}

fn names(index: &DocumentIndex, prefix: Option<&str>) -> Vec<String> {
    let mut names: Vec<_> = index
        .search_scoped("calendar", 10, None, None, None, prefix)
        .unwrap()
        .into_iter()
        .map(|hit| hit.name)
        .collect();
    names.sort();
    names
}

#[test]
fn subtree_does_not_alias_shortened_external_root_paths() {
    let (_temp, index) = fixture();
    assert_eq!(
        names(&index, Some("src/active")),
        vec!["absoluteLocalCalendar", "localCalendar"]
    );
    assert!(names(&index, Some("src/reference")).is_empty());
    assert!(names(&index, None).contains(&"externalOnlyCalendar".to_string()));
}

#[test]
fn explicit_workspace_root_excludes_external_but_unscoped_search_keeps_it() {
    let (_temp, index) = fixture();
    assert_eq!(
        names(&index, Some(".")),
        vec!["absoluteLocalCalendar", "localCalendar"]
    );
    assert_eq!(names(&index, None).len(), 4);
}

#[test]
fn exact_workspace_files_and_separator_aliases_remain_equivalent() {
    let (_temp, index) = fixture();
    assert_eq!(
        names(&index, Some("src/active/local.rs")),
        vec!["localCalendar"]
    );
    assert_eq!(
        names(&index, Some("src/active/absolute.rs")),
        vec!["absoluteLocalCalendar"]
    );
    assert_eq!(
        names(&index, Some(r".\src\active\")),
        names(&index, Some("src/active/"))
    );
    assert!(names(&index, Some("src/act")).is_empty());
}
