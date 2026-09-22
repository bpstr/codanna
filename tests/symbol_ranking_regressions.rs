//! Runtime code-symbol discovery regressions selected by PR #51.
//! Synthetic local sources only; semantic indexing stays disabled.

use codanna::Settings;
use codanna::indexing::facade::IndexFacade;
use codanna::mcp::{CodeIntelligenceServer, SearchSymbolsRequest};
use codanna::storage::SearchResult;
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

        for i in 0..48 {
            write_ts(
                &root,
                &format!("generic/settings_{i:02}.ts"),
                &format!(
                    "/** Generic application settings holder {i}. */\nexport function settings() {{ return {i}; }}\n"
                ),
            );
        }
        for i in 0..28 {
            write_ts(
                &root,
                &format!("generic/subscription_{i:02}.ts"),
                &format!(
                    "/** Generic subscription state {i}. */\nexport function subscription() {{ return {i}; }}\n"
                ),
            );
        }
        for i in 0..24 {
            write_ts(
                &root,
                &format!("generic/development_{i:02}.ts"),
                &format!(
                    "/** Generic development flag {i}. */\nexport function development() {{ return {i}; }}\n"
                ),
            );
        }
        for i in 0..16 {
            write_ts(
                &root,
                &format!("generic/feature_{i:02}.ts"),
                &format!(
                    "/** Generic feature content {i}. */\nexport function featureContent() {{ return {i}; }}\n"
                ),
            );
        }

        for (path, body) in [
            (
                "calendar/presentation.ts",
                "/** Calendar settings apply account first day of week preferences. */\nexport function useAccountPresentation() { return 1; }\n",
            ),
            (
                "calendar/module.ts",
                "/** Shared calendar feature module for workspace calendar views. */\nexport function calendarModule() { return 1; }\n",
            ),
            (
                "integration/binding.ts",
                "/** Create integration binding for GitHub subscription development activity and eligible webhook targets. */\nexport function createIntegrationBinding() { return 1; }\n",
            ),
            (
                "account/current.ts",
                "/** Backend resolves account first day of week inheritance from workspace default in GET me. */\nexport function getCurrentAccount() { return 1; }\n",
            ),
            (
                "workspace/preferences.ts",
                "/** Workspace default locale timezone and calendar settings inheritance. */\nexport function resolveWorkspacePreferences() { return 1; }\n",
            ),
            (
                "upload/policy.ts",
                "/** Attachment upload policy enforces the configured mebibyte limit. */\nexport function enforceUploadPolicy() { return 1; }\n",
            ),
            (
                "delivery/retry.ts",
                "/** Delivery retry backoff prevents duplicate processing after acknowledgement loss. */\nexport function retryDeliveryAfterAckLoss() { return 1; }\n",
            ),
        ] {
            write_ts(&root, path, body);
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

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        self.index
            .search(query, limit, None, None, Some("typescript"))
            .unwrap()
    }
}

fn write_ts(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn rank(results: &[SearchResult], expected: &str) -> Option<usize> {
    results
        .iter()
        .position(|result| result.name == expected)
        .map(|index| index + 1)
}

#[test]
fn multi_concept_topic_owners_survive_generic_exact_name_crowding() {
    let fixture = Fixture::new();
    for (query, expected) in [
        ("calendar settings", "useAccountPresentation"),
        ("calendar feature", "calendarModule"),
        (
            "integration binding creation subscription github development activity eligible webhook targets",
            "createIntegrationBinding",
        ),
        (
            "backend resolves account first day of week inheritance from workspace default GET me",
            "getCurrentAccount",
        ),
        (
            "workspace default locale timezone calendar settings inheritance",
            "resolveWorkspacePreferences",
        ),
        (
            "attachment upload policy configured mebibyte limit",
            "enforceUploadPolicy",
        ),
        (
            "delivery retry acknowledgement loss duplicate processing backoff",
            "retryDeliveryAfterAckLoss",
        ),
    ] {
        let results = fixture.search(query, 5);
        assert!(
            rank(&results, expected).is_some(),
            "{expected} missing from top five for {query}: {:?}",
            results
                .iter()
                .map(|hit| hit.name.as_str())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn small_requested_limit_still_has_a_bounded_discovery_floor() {
    let fixture = Fixture::new();
    let results = fixture.search("calendar settings", 1);
    assert_eq!(results.len(), 1);
    assert!(
        !matches!(results[0].name.as_str(), "settings" | "featureContent"),
        "bounded discovery should select a multi-concept owner, got {}",
        results[0].name
    );
    let evidence = format!(
        "{} {} {} {}",
        results[0].name,
        results[0].doc_comment.as_deref().unwrap_or_default(),
        results[0].module_path,
        results[0].file_path
    )
    .to_lowercase();
    assert!(evidence.contains("calendar") && evidence.contains("settings"));
}

#[test]
fn exact_identifiers_single_terms_explicit_syntax_and_filters_keep_existing_path() {
    let fixture = Fixture::new();
    for query in [
        "useAccountPresentation",
        "calendarModule",
        "createIntegrationBinding",
        "getCurrentAccount",
    ] {
        let results = fixture.search(query, 5);
        assert_eq!(results.first().map(|hit| hit.name.as_str()), Some(query));
    }

    assert!(fixture.search("qzvnoexist75391", 5).is_empty());
    assert!(
        fixture
            .index
            .search("calendar settings", 40, None, None, Some("rust"))
            .unwrap()
            .is_empty()
    );

    // Explicit Tantivy syntax bypasses discovery reranking rather than being
    // reparsed as natural-language coverage terms.
    let explicit = fixture.search("calendar AND settings", 20);
    assert!(explicit.len() <= 20);
}

#[tokio::test]
async fn mcp_search_explains_raw_candidate_score_and_coverage_without_changing_limits() {
    let Fixture { _temp, index } = Fixture::new();
    let server = CodeIntelligenceServer::new(index);
    let response = server
        .search_symbols(Parameters(SearchSymbolsRequest {
            query: "calendar settings".into(),
            limit: 5,
            kind: None,
            module: None,
            lang: Some("typescript".into()),
        }))
        .await
        .unwrap();
    assert_ne!(response.is_error, Some(true));
    let text = serde_json::to_value(response).unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Found 5 result(s)"));
    assert!(text.contains("useAccountPresentation"));
    assert!(text.contains("Score:") && text.contains("raw lexical candidate score"));
    assert!(text.contains("Distinct query-term coverage: 2/2"));

    let explicit = server
        .search_symbols(Parameters(SearchSymbolsRequest {
            query: "calendar AND settings".into(),
            limit: 5,
            kind: None,
            module: None,
            lang: Some("typescript".into()),
        }))
        .await
        .unwrap();
    let text = serde_json::to_value(explicit).unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("Distinct query-term coverage:"));
}
