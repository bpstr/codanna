//! Actual ticket handler measurement. Direct Hit@5 and sidecar recall are distinct.
use codanna::Settings;
use codanna::indexing::{IndexFacade, calculate_hash};
use codanna::mcp::CodeIntelligenceServer;
use codanna::mcp::tools::ticket_context::TicketContextRequest;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use std::sync::Arc;

const ORACLE: &str = include_str!("../contributing/retrieval/evaluations/repository-tasks.json");
const SOURCES: &[(&str, &str)] = &[
    (
        "src/indexing/pipeline/stages/read.rs",
        include_str!("fixtures/repository_task_sources/read.rs.fixture"),
    ),
    (
        "src/indexing/pipeline/stages/discover.rs",
        include_str!("fixtures/repository_task_sources/discover.rs.fixture"),
    ),
    (
        "src/mcp/tools/recall.rs",
        include_str!("fixtures/repository_task_sources/recall.rs.fixture"),
    ),
    (
        "src/embedding_cache.rs",
        include_str!("fixtures/repository_task_sources/embedding_cache.rs.fixture"),
    ),
    (
        "src/memory.rs",
        include_str!("fixtures/repository_task_sources/memory.rs.fixture"),
    ),
];

fn rank(items: &Value, name: &str, path: &str) -> Option<usize> {
    items
        .as_array()
        .unwrap()
        .iter()
        .position(|item| item["name"] == name && item["file_path"] == path)
        .map(|position| position + 1)
}

// The index reader may process a delayed reload from the initial fixture commit.
// The runtime correctly rejects related evidence when that happens mid-query.
// Retry only that explicit state, with a hard cap and a recorded event. Never
// retry missing owners, empty pools, graph errors, budget errors or failed tests.
// This is a test measurement policy, not an automatic retry in the product.
async fn stable_pair(server: &CodeIntelligenceServer, value: &Value) -> (Value, Value, usize) {
    for attempt in 0..3 {
        let original: TicketContextRequest = serde_json::from_value(value.clone()).unwrap();
        let response = server
            .search_ticket_context(Parameters(original))
            .await
            .unwrap();
        assert_ne!(response.is_error, Some(true));
        let direct = response.structured_content.unwrap();
        let mut opted_in = value.clone();
        opted_in["include_related_code"] = json!(true);
        let response = server
            .search_ticket_context(Parameters(serde_json::from_value(opted_in).unwrap()))
            .await
            .unwrap();
        assert_ne!(response.is_error, Some(true));
        let expanded = response.structured_content.unwrap();
        let related = &expanded["code"]["related_code"];
        if matches!(
            related["status"].as_str(),
            Some("not_run_generation_mismatch" | "discarded_generation_changed")
        ) {
            assert_eq!(
                related["items"],
                json!([]),
                "changed generations must not leak related identities"
            );
            println!(
                "ticket_related_generation_retry={}",
                json!({"query":value["query"], "attempt":attempt + 1, "related":related})
            );
            continue;
        }
        return (direct, expanded, attempt);
    }
    panic!("reader did not stabilize within three explicitly recorded attempts: {value}");
}

#[tokio::test]
async fn ticket_related_relevance_preserves_direct_results_and_measures_related_owners() {
    assert_eq!(
        calculate_hash(ORACLE),
        "129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6"
    );
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("corpus");
    for &(path, source) in SOURCES {
        assert_eq!(calculate_hash(source), oracle["files"][path]);
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, source).unwrap();
    }
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.join("src")).unwrap();
    let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
    index.index_directory(&root.join("src"), true).unwrap();
    for task in oracle["tasks"].as_array().unwrap() {
        let matches = index.find_symbols_by_name(task["name"].as_str().unwrap(), Some("rust"));
        assert_eq!(
            matches
                .iter()
                .filter(|symbol| symbol.file_path.as_ref() == task["path"].as_str().unwrap())
                .count(),
            1
        );
    }
    let server = CodeIntelligenceServer::new(index);
    let mut direct_hits = [0usize; 2];
    let mut expanded_hits = [0usize; 2];
    let mut queries = 0;
    let mut generation_retries = 0;
    for task in oracle["tasks"].as_array().unwrap() {
        let name = task["name"].as_str().unwrap();
        let path = task["path"].as_str().unwrap();
        for (family, query) in task["queries"].as_array().unwrap().iter().enumerate() {
            let value = json!({ "query":query, "code_limit":5, "document_limit":1, "conversation_limit":1 });
            let (direct, expanded, retries) = stable_pair(&server, &value).await;
            generation_retries += retries;
            assert_eq!(
                expanded["code"]["items"], direct["code"]["items"],
                "opting in must not reorder or rescore direct candidates"
            );
            assert_eq!(expanded["code"]["semantic_status"], "not_requested");
            assert_eq!(expanded["documents"]["status"], "not_configured");
            assert_eq!(expanded["conversations"]["requested"], false);
            let direct_rank = rank(&direct["code"]["items"], name, path);
            let related_rank = rank(&expanded["code"]["related_code"]["items"], name, path);
            direct_hits[family] += usize::from(direct_rank.is_some());
            expanded_hits[family] += usize::from(direct_rank.is_some() || related_rank.is_some());
            println!(
                "ticket_related_case={}",
                json!({
                    "task":task["id"], "family":family, "query":query, "owner":name, "path":path,
                    "direct_rank_at_5":direct_rank, "related_rank_at_6":related_rank,
                    "generation_retries":retries, "related":expanded["code"]["related_code"],
                })
            );
            if family == 0
                && matches!(
                    task["id"].as_str(),
                    Some("recall-timeout" | "foreign-recall")
                )
            {
                assert!(
                    related_rank.is_some(),
                    "indexed call witness should recover {name} as related evidence"
                );
            }
            queries += 1;
        }
    }
    assert_eq!(queries, 20);
    println!(
        "ticket_related_summary={}",
        json!({
            "queries":queries, "direct_hit_at_5":direct_hits,
            "direct_or_related_hit_at_up_to_11":expanded_hits,
            "direct_results_unchanged":true, "paid_inference":false,
            "generation_retries":generation_retries,
            "measurement":"bounded_retry_only_for_explicit_generation_invalidation",
            "quality_gate_passed":false,
        })
    );
}
