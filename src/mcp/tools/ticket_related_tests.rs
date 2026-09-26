//! Local fixed-graph contracts; no model, provider, filesystem source discovery or recall.
use super::ticket_context::TicketContextRequest;
use super::ticket_related::{self, MAX_RELATED, MAX_SEEDS};
use crate::indexing::facade::IndexFacade;
use crate::indexing::pipeline::{ResolvedRelationship, stages::WriteStage};
use crate::mcp::CodeIntelligenceServer;
use crate::relationship::RelationshipMetadata;
use crate::{FileId, Range, RelationKind, Settings, Symbol, SymbolId, SymbolKind};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::json;
use std::sync::Arc;

fn fixture(count: u32, edges: &[(u32, u32)]) -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let index = IndexFacade::new(Arc::new(settings)).unwrap();
    let storage = index.document_index();
    storage.start_batch().unwrap();
    for id in 1..=count {
        let name = if id == 1 {
            "requestHandler".to_owned()
        } else {
            format!("implementation{id}")
        };
        let mut symbol = Symbol::new(
            SymbolId::new(id).unwrap(),
            name,
            SymbolKind::Function,
            FileId::new(1).unwrap(),
            Range::new(id, 0, id, 20),
        );
        if id == 1 {
            symbol.doc_comment = Some("conversation timeout request handler".into());
        }
        storage.index_symbol(&symbol, "src/handlers.rs").unwrap();
    }
    let mut writer = WriteStage::new(storage.clone());
    for &(source, target) in edges {
        writer
            .write_one(
                ResolvedRelationship::new(
                    SymbolId::new(source).unwrap(),
                    SymbolId::new(target).unwrap(),
                    RelationKind::Calls,
                )
                .with_metadata(RelationshipMetadata {
                    line: Some(9),
                    column: Some(3),
                    ..Default::default()
                }),
            )
            .unwrap();
    }
    writer.flush().unwrap();
    (temp, index)
}

#[test]
fn ticket_related_deduplicates_targets_and_preserves_callsite_identity() {
    let (_temp, index) = fixture(5, &[(1, 3), (1, 3), (2, 3), (1, 2), (3, 5), (3, 1)]);
    let report = ticket_related::collect(
        &index,
        &[1, 2],
        None,
        Some(index.document_index().generation()),
    );
    assert_eq!(report.status, "completed_bounded");
    assert_eq!(report.items.len(), 1);
    let target = &report.items[0];
    assert_eq!(target.symbol_id, 3);
    assert_eq!(target.name, "implementation3");
    assert_eq!(
        target.via.len(),
        2,
        "duplicate physical edges must not add seed votes"
    );
    assert_eq!(target.via[0].seed_symbol_id, 1);
    assert_eq!(target.via[0].call_line, Some(10));
    assert_eq!(target.via[0].call_column, Some(3));
    assert_eq!(target.via[0].seed_file_path, "src/handlers.rs");
    assert_eq!(report.distinct_targets, 1);
    assert_eq!(report.source_coverage, "unknown");
    assert_eq!(report.freshness, "unknown");
    assert!(report.render().contains("Indexed Calls from direct rank 1"));
    assert!(
        !report.render().contains("implementation5"),
        "one hop must not become recursive expansion"
    );
}

#[test]
fn ticket_related_observes_seed_and_result_budgets() {
    let edges: Vec<_> = (5..=18).map(|id| (1, id)).chain([(4, 20)]).collect();
    let (_temp, index) = fixture(20, &edges);
    let report = ticket_related::collect(
        &index,
        &[1, 2, 3, 4],
        None,
        Some(index.document_index().generation()),
    );
    assert_eq!(report.probes.len(), MAX_SEEDS);
    assert_eq!(report.items.len(), MAX_RELATED);
    assert_eq!(report.distinct_targets, 14);
    assert_eq!(report.omitted_targets, 8);
    assert!(!report.items.iter().any(|item| item.symbol_id == 20));
    let again = ticket_related::collect(
        &index,
        &[1, 1, 2, 3, 4],
        None,
        Some(index.document_index().generation()),
    );
    assert_eq!(
        again.probes.len(),
        MAX_SEEDS,
        "duplicate seeds do not consume the seed budget twice"
    );
}

#[test]
fn ticket_related_keeps_empty_missing_and_dangling_states_distinct() {
    let (_temp, index) = fixture(3, &[(1, 99)]);
    let generation = Some(index.document_index().generation());
    let dangling = ticket_related::collect(&index, &[1], None, generation);
    assert_eq!(dangling.status, "partial");
    assert_eq!(dangling.probes[0].indexed_edges, Some(1));
    assert_eq!(dangling.probes[0].unhydrated_edges, 1);
    assert!(dangling.items.is_empty());
    let empty = ticket_related::collect(&index, &[2], None, generation);
    assert_eq!(empty.status, "empty_indexed_neighborhoods");
    assert_eq!(empty.probes[0].indexed_edges, Some(0));
    let missing = ticket_related::collect(&index, &[98], None, generation);
    assert_eq!(missing.status, "partial");
    assert_eq!(missing.probes[0].status, "missing_seed");
    assert!(missing.probes[0].indexed_edges.is_none());
    let invalid = ticket_related::collect(&index, &[0], None, generation);
    assert_eq!(invalid.probes[0].status, "invalid_seed");
}

#[test]
fn ticket_related_over_budget_neighborhood_does_not_become_empty_success() {
    let edges: Vec<_> = (2..=34).map(|id| (1, id)).collect();
    let (_temp, index) = fixture(34, &edges);
    let report = ticket_related::collect(
        &index,
        &[1],
        None,
        Some(index.document_index().generation()),
    );
    assert_eq!(report.status, "partial");
    assert_eq!(report.probes[0].status, "edge_budget_exceeded");
    assert!(report.items.is_empty());
    assert!(report.probes[0].warning.as_ref().unwrap().contains("32"));
}

#[test]
fn ticket_related_rejects_unregistered_seeds_and_generation_mismatch_before_expansion() {
    let (_temp, index) = fixture(2, &[(1, 2)]);
    let generation = index.document_index().generation();
    let scoped = ticket_related::collect(&index, &[1], Some("."), Some(generation));
    assert_eq!(scoped.status, "partial");
    assert_eq!(scoped.probes[0].status, "seed_outside_scope");
    assert!(scoped.probes[0].indexed_edges.is_none());
    assert!(scoped.items.is_empty());
    let stale = ticket_related::collect(&index, &[1], None, Some(generation.saturating_add(1)));
    assert_eq!(stale.status, "not_run_generation_mismatch");
    assert!(stale.items.is_empty());
    assert!(stale.probes.is_empty());
    assert_eq!(
        ticket_related::collect(&index, &[], None, Some(generation)).status,
        "not_run_no_direct_matches"
    );
    assert_eq!(
        ticket_related::collect(&index, &[1; 11], None, Some(generation)).status,
        "not_run_seed_budget_exceeded"
    );
}

#[test]
fn ticket_related_schema_requires_an_actual_boolean_and_defaults_off() {
    let request: TicketContextRequest =
        serde_json::from_value(json!({"query":"conversation timeout"})).unwrap();
    assert!(!request.include_related_code);
    for invalid in [json!("true"), json!(1), json!(null), json!([])] {
        assert!(
            serde_json::from_value::<TicketContextRequest>(json!({
                "query":"conversation timeout", "include_related_code":invalid,
            }))
            .is_err()
        );
    }
}

#[tokio::test]
async fn ticket_related_sidecar_preserves_direct_items_and_default_response_shape() {
    let (_temp, index) = fixture(4, &[(1, 2), (1, 3), (2, 4)]);
    let server = CodeIntelligenceServer::new(index);
    let request = json!({"query":"conversation timeout", "code_limit":1});
    let disabled = server
        .search_ticket_context(Parameters(serde_json::from_value(request.clone()).unwrap()))
        .await
        .unwrap();
    let direct = disabled.structured_content.as_ref().unwrap();
    assert!(direct["code"].get("related_code").is_none());
    assert_eq!(direct["graph"]["query_status"], "not_run");
    let mut enabled = request;
    enabled["include_related_code"] = json!(true);
    let response = server
        .search_ticket_context(Parameters(serde_json::from_value(enabled).unwrap()))
        .await
        .unwrap();
    let data = response.structured_content.as_ref().unwrap();
    assert_eq!(data["code"]["items"], direct["code"]["items"]);
    assert_eq!(
        data["code"]["related_code"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(data["graph"]["query_status"], "completed_bounded");
    let encoded = serde_json::to_value(&response).unwrap();
    let rendered = encoded["content"][0]["text"].as_str().unwrap();
    assert!(rendered.contains("Related implementations"));
    assert!(rendered.contains("implementation2"));
    assert!(!rendered.contains("implementation4"));
    assert!(!rendered.contains("Graph traversal was not run"));
    assert!(
        data["code"]["related_code"]["items"][0]
            .get("fusion_score")
            .is_none()
    );
}

#[tokio::test]
async fn evidence_v1_reverse_profiles_retrieve_consumers_beyond_direct_matches() {
    let (_temp, index) = fixture(18, &(2..=18).map(|id| (id, 1)).collect::<Vec<_>>());
    let storage = index.document_index();
    let mut owner = index
        .get_all_symbols()
        .into_iter()
        .find(|s| s.id.value() == 2)
        .unwrap();
    owner.visibility = crate::Visibility::Public;
    storage.start_batch().unwrap();
    storage.index_symbol(&owner, "src/handlers.rs").unwrap();
    storage.commit_batch().unwrap();
    let server = CodeIntelligenceServer::new(index);
    let baseline = server
        .search_ticket_context(Parameters(
            serde_json::from_value(json!({"query":"requestHandler", "code_limit":1})).unwrap(),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(baseline["code"]["items"][0]["name"], "requestHandler");
    let first = server
        .search_ticket_context(Parameters(
            serde_json::from_value(
                json!({"query":"requestHandler", "profile":"coverage", "coverage_limit":10}),
            )
            .unwrap(),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let coverage = &first["code"]["coverage"];
    assert_eq!(coverage["total_candidates"], 18);
    assert_eq!(coverage["items"].as_array().unwrap().len(), 10);
    let next = server.search_ticket_context(Parameters(serde_json::from_value(json!({"query":"requestHandler", "profile":"coverage", "coverage_limit":10, "coverage_offset":10, "coverage_snapshot":coverage["snapshot"]})).unwrap())).await.unwrap().structured_content.unwrap();
    assert_eq!(
        next["code"]["coverage"]["items"].as_array().unwrap().len(),
        8
    );
    let owner = server
        .search_ticket_context(Parameters(
            serde_json::from_value(
                json!({"query":"requestHandler", "profile":"implementation_owner", "code_limit":1}),
            )
            .unwrap(),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_ne!(owner["code"]["items"][0]["name"], "requestHandler");
    assert_eq!(
        owner["code"]["items"][0]["relationships"][0]["direction"],
        "incoming"
    );
}
