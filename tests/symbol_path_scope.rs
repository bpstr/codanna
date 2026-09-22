//! Explicit workspace-relative symbol discovery scope; no semantic provider.

use codanna::indexing::facade::IndexFacade;
use codanna::mcp::{CodeIntelligenceServer, SearchContextRequest, SearchSymbolsRequest};
use codanna::storage::SearchResult;
use codanna::{IndexError, Settings, StorageError};
use rmcp::handler::server::wrapper::Parameters;
use std::path::Path;
use std::sync::Arc;

const ACTIVE: &str = "src/active/calendar";
const REFERENCE: &str = "src/reference/calendar";

struct Fixture {
    temp: tempfile::TempDir,
    index: IndexFacade,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("src");
        std::fs::create_dir_all(&root).unwrap();
        for (path, name, description) in [
            (
                "active/calendar/presentation.ts",
                "useAccountPresentation",
                "active account first day preferences",
            ),
            (
                "active/calendar/Calendar.ts",
                "Calendar",
                "active component",
            ),
            (
                "reference/calendar/Calendar.ts",
                "Calendar",
                "reference implementation",
            ),
            (
                "active/calendar-old/decoy.ts",
                "oldCalendar",
                "sibling prefix decoy",
            ),
            (
                "active/calendar/Calendar.ts.backup.ts",
                "backupCalendar",
                "file prefix decoy",
            ),
        ] {
            write_ts(
                &root,
                path,
                &format!(
                    "/** Calendar settings {description}. */\nexport function {name}() {{ return 1; }}\n"
                ),
            );
        }
        for i in 0..96 {
            write_ts(
                &root,
                &format!("archive/calendar/settings_{i:02}.ts"),
                &format!(
                    "/** Archived calendar settings prototype {i}. */\nexport function settings() {{ return {i}; }}\n"
                ),
            );
        }
        let mut settings = Settings {
            workspace_root: Some(temp.path().to_path_buf()),
            index_path: temp.path().join("index"),
            ..Default::default()
        };
        settings.semantic_search.enabled = false;
        settings.add_indexed_path(root.clone()).unwrap();
        let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
        index.index_directory(&root, true).unwrap();
        Self { temp, index }
    }

    fn scoped(&self, prefix: &str, limit: usize) -> Vec<SearchResult> {
        self.index
            .search_scoped(
                "calendar settings",
                limit,
                None,
                None,
                Some("typescript"),
                Some(prefix),
            )
            .unwrap()
    }
}

fn write_ts(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn text(response: &rmcp::model::CallToolResult) -> String {
    serde_json::to_value(response).unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn subtree_scope_precedes_top_k_and_broad_search_keeps_reference_code() {
    let fixture = Fixture::new();
    let broad = fixture
        .index
        .search("calendar settings", 200, None, None, Some("typescript"))
        .unwrap();
    assert!(
        broad
            .iter()
            .any(|hit| hit.file_path.starts_with("src/archive/"))
    );
    assert!(
        broad
            .iter()
            .any(|hit| hit.file_path.starts_with("src/reference/"))
    );

    let scoped = fixture.scoped(ACTIVE, 1);
    assert_eq!(scoped.len(), 1);
    assert!(scoped[0].file_path.starts_with("src/active/calendar/"));
    assert_ne!(scoped[0].name, "settings");

    let reference = fixture.scoped(r"src\reference\calendar", 5);
    assert_eq!(reference.len(), 1);
    assert_eq!(reference[0].file_path, "src/reference/calendar/Calendar.ts");
    assert!(fixture.scoped("src/missing/subtree", 5).is_empty());
    // The indexed source root is not the workspace root. Never silently retry
    // a wrong workspace-relative prefix against an arbitrary indexed root.
    assert!(fixture.scoped("active/calendar", 5).is_empty());
}

#[test]
fn exact_file_scope_and_subtree_boundaries_exclude_prefix_neighbors() {
    let fixture = Fixture::new();
    let file = fixture.scoped("src/active/calendar/Calendar.ts", 10);
    assert_eq!(file.len(), 1);
    assert_eq!(file[0].name, "Calendar");
    assert_eq!(file[0].file_path, "src/active/calendar/Calendar.ts");

    let directory = fixture.scoped(ACTIVE, 10);
    assert_eq!(directory.len(), 3);
    assert!(
        directory
            .iter()
            .all(|hit| hit.file_path.starts_with("src/active/calendar/"))
    );
    assert!(!directory.iter().any(|hit| hit.name == "oldCalendar"));
    let aliases = fixture.scoped("./src//active/./calendar/", 10);
    assert_eq!(
        directory
            .iter()
            .map(|hit| hit.symbol_id)
            .collect::<Vec<_>>(),
        aliases.iter().map(|hit| hit.symbol_id).collect::<Vec<_>>()
    );
}

#[test]
fn root_scope_matches_unscoped_results_and_escape_paths_are_rejected() {
    let fixture = Fixture::new();
    let broad = fixture
        .index
        .search("calendar settings", 5, None, None, Some("typescript"))
        .unwrap();
    let root = fixture.scoped(".", 5);
    assert_eq!(
        broad.iter().map(|hit| hit.symbol_id).collect::<Vec<_>>(),
        root.iter().map(|hit| hit.symbol_id).collect::<Vec<_>>()
    );
    for invalid in [
        "",
        " ",
        "../reference",
        "src/../reference",
        "/src",
        r"C:\src",
        r"\\server\share",
    ] {
        let error = fixture
            .index
            .search_scoped(
                "calendar settings",
                5,
                None,
                None,
                Some("typescript"),
                Some(invalid),
            )
            .unwrap_err();
        assert!(
            matches!(
                error,
                IndexError::Storage(StorageError::InvalidFieldValue { ref field, .. }) if field == "path_prefix"
            ),
            "{invalid}: {error}"
        );
    }
}

#[test]
fn exact_same_name_lookup_and_language_filters_remain_independent() {
    let fixture = Fixture::new();
    let calendars = fixture
        .index
        .find_symbols_by_name("Calendar", Some("typescript"));
    assert_eq!(calendars.len(), 2);
    assert!(
        calendars
            .iter()
            .any(|symbol| symbol.file_path.as_ref() == "src/active/calendar/Calendar.ts")
    );
    assert!(
        calendars
            .iter()
            .any(|symbol| symbol.file_path.as_ref() == "src/reference/calendar/Calendar.ts")
    );
    assert!(
        fixture
            .index
            .search_scoped(
                "calendar settings",
                10,
                None,
                None,
                Some("rust"),
                Some(ACTIVE),
            )
            .unwrap()
            .is_empty()
    );
}

#[test]
fn scope_tracks_reindex_deletion_and_reopen_without_global_fallback() {
    let mut fixture = Fixture::new();
    let relative = "src/reference/calendar/Calendar.ts";
    let path = fixture.temp.path().join(relative);
    assert_eq!(fixture.scoped(REFERENCE, 5).len(), 1);
    std::fs::remove_file(&path).unwrap();
    fixture
        .index
        .index_directory(&fixture.temp.path().join("src"), false)
        .unwrap();
    assert!(fixture.scoped(REFERENCE, 5).is_empty());

    write_ts(
        fixture.temp.path(),
        relative,
        "/** Calendar settings replacement. */\nexport function replacementCalendar() { return 3; }\n",
    );
    fixture
        .index
        .index_directory(&fixture.temp.path().join("src"), false)
        .unwrap();
    let results = fixture.scoped(REFERENCE, 5);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "replacementCalendar");
    let settings = fixture.index.settings().clone();
    drop(fixture.index);
    let reopened = IndexFacade::new(Arc::new(settings)).unwrap();
    let results = reopened
        .search_scoped("calendar settings", 5, None, None, None, Some(REFERENCE))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "replacementCalendar");
}

#[tokio::test]
async fn mcp_symbol_and_context_search_share_the_same_code_scope() {
    let Fixture { temp: _temp, index } = Fixture::new();
    let server = CodeIntelligenceServer::new(index);
    let request = serde_json::from_value::<SearchSymbolsRequest>(serde_json::json!({
        "query": "calendar settings", "limit": 3, "lang": "typescript", "path_prefix": ACTIVE,
    }))
    .unwrap();
    let response = server.search_symbols(Parameters(request)).await.unwrap();
    assert_ne!(response.is_error, Some(true));
    let rendered = text(&response);
    assert!(rendered.contains("src/active/calendar/"), "{rendered}");
    assert!(!rendered.contains("src/archive/calendar/"));
    assert!(!rendered.contains("src/reference/calendar/"));

    let request = serde_json::from_value::<SearchContextRequest>(serde_json::json!({
        "query": "calendar settings", "code_limit": 3, "document_limit": 1,
        "conversation_limit": 1, "code_path_prefix": ACTIVE,
    }))
    .unwrap();
    let response = server.search_context(Parameters(request)).await.unwrap();
    assert_ne!(response.is_error, Some(true));
    let rendered = text(&response);
    let code = rendered.split("## Documents").next().unwrap();
    assert!(code.contains("src/active/calendar/"), "{code}");
    assert!(!code.contains("src/archive/calendar/"));
    assert!(!code.contains("src/reference/calendar/"));
}

#[tokio::test]
async fn invalid_context_scope_is_a_tool_error_before_other_sources() {
    let Fixture { temp: _temp, index } = Fixture::new();
    let server = CodeIntelligenceServer::new(index);
    for invalid in ["", "../reference", "/src"] {
        let request = serde_json::from_value::<SearchContextRequest>(serde_json::json!({
            "query": "calendar settings", "code_path_prefix": invalid,
        }))
        .unwrap();
        let response = server.search_context(Parameters(request)).await.unwrap();
        assert_eq!(response.is_error, Some(true), "{}", text(&response));
        let rendered = text(&response);
        assert!(rendered.contains("code_path_prefix"));
        assert!(!rendered.contains("## Documents"));
        assert!(!rendered.contains("## Conversations"));
    }
}
