//! Explicit within-workspace path scope for symbol discovery.
//! Synthetic local files only; semantic indexing is disabled.

use codanna::indexing::facade::IndexFacade;
use codanna::mcp::{CodeIntelligenceServer, SearchContextRequest, SearchSymbolsRequest};
use codanna::{IndexError, Settings, StorageError};
use rmcp::handler::server::wrapper::Parameters;
use std::path::Path;
use std::sync::Arc;

struct Fixture {
    _temp: tempfile::TempDir,
    index: IndexFacade,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("src");
        std::fs::create_dir_all(&root).unwrap();

        write_ts(
            &root,
            "active/calendar/presentation.ts",
            "/** Calendar settings active account first day preferences. */\nexport function useAccountPresentation() { return 1; }\n",
        );
        write_ts(
            &root,
            "active/calendar/Calendar.ts",
            "/** Calendar settings active component. */\nexport function Calendar() { return 1; }\n",
        );
        write_ts(
            &root,
            "reference/calendar/Calendar.ts",
            "/** Calendar settings reference implementation. */\nexport function Calendar() { return 2; }\n",
        );
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
        Self { _temp: temp, index }
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
fn subtree_scope_is_applied_before_top_k_and_broad_search_keeps_reference_code() {
    let fixture = Fixture::new();

    let broad = fixture
        .index
        .search("calendar settings", 5, None, None, Some("typescript"))
        .unwrap();
    assert_eq!(broad.len(), 5);
    assert!(
        broad.iter().any(|hit| hit.file_path.starts_with("archive/"))
            || broad.iter().any(|hit| hit.file_path.starts_with("reference/")),
        "unscoped discovery must keep archive/reference code searchable"
    );

    let scoped = fixture
        .index
        .search_scoped(
            "calendar settings",
            1,
            None,
            None,
            Some("typescript"),
            Some("active/calendar"),
        )
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert!(scoped[0].file_path.starts_with("active/calendar/"));
    assert_ne!(scoped[0].name, "settings");

    let reference = fixture
        .index
        .search_scoped(
            "calendar settings",
            5,
            None,
            None,
            Some("typescript"),
            Some(r"reference\calendar"),
        )
        .unwrap();
    assert!(!reference.is_empty());
    assert!(
        reference
            .iter()
            .all(|hit| hit.file_path.starts_with("reference/calendar/"))
    );

    assert!(
        fixture
            .index
            .search_scoped(
                "calendar settings",
                5,
                None,
                None,
                Some("typescript"),
                Some("missing/subtree"),
            )
            .unwrap()
            .is_empty()
    );
}

#[test]
fn root_scope_matches_unscoped_results_and_escape_paths_are_rejected() {
    let fixture = Fixture::new();
    let broad = fixture
        .index
        .search("calendar settings", 5, None, None, Some("typescript"))
        .unwrap();
    let root = fixture
        .index
        .search_scoped(
            "calendar settings",
            5,
            None,
            None,
            Some("typescript"),
            Some("."),
        )
        .unwrap();
    assert_eq!(
        broad.iter().map(|hit| hit.symbol_id).collect::<Vec<_>>(),
        root.iter().map(|hit| hit.symbol_id).collect::<Vec<_>>()
    );

    for invalid in ["", "../reference", "active/../reference", "/active", r"C:\active"] {
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
                IndexError::Storage(StorageError::InvalidFieldValue { ref field, .. })
                    if field == "path_prefix"
            ),
            "{invalid}: {error}"
        );
    }
}

#[test]
fn exact_same_name_lookup_remains_disambiguatable_outside_scoped_discovery() {
    let fixture = Fixture::new();
    let calendars = fixture.index.find_symbols_by_name("Calendar", Some("typescript"));
    assert_eq!(calendars.len(), 2);
    let paths = calendars
        .iter()
        .map(|symbol| symbol.file_path.as_ref())
        .collect::<Vec<_>>();
    assert!(paths.iter().any(|path| path.contains("active/calendar")));
    assert!(paths.iter().any(|path| path.contains("reference/calendar")));
}

#[tokio::test]
async fn mcp_symbol_and_context_search_share_the_same_code_scope() {
    let Fixture { _temp, index } = Fixture::new();
    let server = CodeIntelligenceServer::new(index);

    let response = server
        .search_symbols(Parameters(SearchSymbolsRequest {
            query: "calendar settings".into(),
            limit: 3,
            kind: None,
            module: None,
            lang: Some("typescript".into()),
            path_prefix: Some("active/calendar".into()),
        }))
        .await
        .unwrap();
    assert_ne!(response.is_error, Some(true));
    let rendered = text(&response);
    assert!(rendered.contains("active/calendar/"));
    assert!(!rendered.contains("archive/calendar/"));
    assert!(!rendered.contains("reference/calendar/"));

    let response = server
        .search_context(Parameters(SearchContextRequest {
            query: "calendar settings".into(),
            code_limit: 3,
            document_limit: 1,
            conversation_limit: 1,
            collection: None,
            code_path_prefix: Some("active/calendar".into()),
        }))
        .await
        .unwrap();
    assert_ne!(response.is_error, Some(true));
    let rendered = text(&response);
    let code = rendered.split("## Documents").next().unwrap();
    assert!(code.contains("active/calendar/"));
    assert!(!code.contains("archive/calendar/"));
    assert!(!code.contains("reference/calendar/"));
}
