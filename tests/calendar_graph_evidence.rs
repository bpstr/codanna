//! Public MCP graph contracts for the synthetic calendar workspace in PR #43.
//! No model, provider, production index, or transcript is used.

use codanna::indexing::facade::IndexFacade;
use codanna::indexing::pipeline::{ResolvedRelationship, stages::WriteStage};
use codanna::mcp::requests::{AnalyzeImpactRequest, FindCallersRequest, GetCallsRequest};
use codanna::mcp::server::CodeIntelligenceServer;
use codanna::{FileId, Range, RelationKind, Settings, Symbol, SymbolId, SymbolKind};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const ORACLE: &str = include_str!("../contributing/retrieval/calendar-graph-evidence/cases.json");

fn fixture() -> (
    tempfile::TempDir,
    CodeIntelligenceServer,
    HashMap<String, Symbol>,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    for (path, content) in [
        (
            "active/calendar.ts",
            include_str!("fixtures/calendar_graph_evidence/workspace/active/calendar.ts"),
        ),
        (
            "reference/calendar.ts",
            include_str!("fixtures/calendar_graph_evidence/workspace/reference/calendar.ts"),
        ),
    ] {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.clone()).unwrap();
    let mut facade = IndexFacade::new(Arc::new(settings)).unwrap();
    facade.index_directory(&root, true).unwrap();

    // Resolve current-generation IDs from the independent oracle, never from a
    // historical Assign run. Matching the name alone would mix active/reference.
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let mut targets = HashMap::new();
    for (key, selector) in oracle["selectors"].as_object().unwrap() {
        let matches: Vec<_> = facade
            .find_symbols_by_name(selector["name"].as_str().unwrap(), None)
            .into_iter()
            .filter(|symbol| {
                symbol.kind == SymbolKind::Function
                    && symbol
                        .file_path
                        .to_string()
                        .replace('\\', "/")
                        .ends_with(selector["path"].as_str().unwrap())
            })
            .collect();
        assert_eq!(matches.len(), 1, "ambiguous fixture selector: {selector}");
        targets.insert(key.clone(), matches.into_iter().next().unwrap());
    }
    (temp, CodeIntelligenceServer::new(facade), targets)
}

fn text(result: &CallToolResult) -> String {
    let value = serde_json::to_value(result).unwrap();
    value["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn graph(result: &CallToolResult) -> &Value {
    assert_ne!(result.is_error, Some(true));
    &result.structured_content.as_ref().expect("graph evidence")["graph"]
}

fn assert_empty(result: &CallToolResult, operation: &str, target: &Symbol, depth: u32) {
    let evidence = graph(result);
    assert_eq!(evidence["schema_version"], 1);
    assert_eq!(evidence["operation"], operation);
    assert_eq!(evidence["status"], "empty");
    assert_eq!(evidence["query_status"], "completed");
    assert_eq!(evidence["scope"], "resolved_indexed_relationships");
    assert_eq!(evidence["source_coverage"], "unknown");
    assert_eq!(evidence["freshness"], "unknown");
    assert!(evidence["index_generation"].is_null());
    assert_eq!(evidence["returned"], 0);
    assert_eq!(evidence["max_depth"], depth);
    assert_eq!(evidence["target"]["symbol_id"], target.id.value());
    assert_eq!(evidence["target"]["line"], target.range.start_line + 1);
    let rendered = text(result);
    assert!(rendered.contains("indexed"), "{rendered}");
    assert!(rendered.contains("unknown"), "{rendered}");
    assert!(
        !rendered.contains("doesn't call any functions"),
        "{rendered}"
    );
    assert!(!rendered.contains("No functions call "), "{rendered}");
    assert!(
        !rendered.contains("No symbols would be impacted"),
        "{rendered}"
    );
}

#[tokio::test]
async fn isolated_and_external_calls_do_not_claim_source_absence() {
    let (_temp, server, targets) = fixture();
    for key in ["isolated", "external"] {
        let target = &targets[key];
        let result = server
            .get_calls(Parameters(GetCallsRequest {
                function_name: None,
                symbol_id: Some(target.id.value()),
            }))
            .await
            .unwrap();
        assert_empty(&result, "get_calls", target, 1);
    }
}

#[tokio::test]
async fn empty_callers_and_impact_remain_scoped_to_the_active_definition() {
    let (_temp, server, targets) = fixture();
    let target = &targets["isolated"];
    let callers = server
        .find_callers(Parameters(FindCallersRequest {
            function_name: None,
            symbol_id: Some(target.id.value()),
        }))
        .await
        .unwrap();
    assert_empty(&callers, "find_callers", target, 1);

    let impact = server
        .analyze_impact(Parameters(AnalyzeImpactRequest {
            symbol_name: None,
            symbol_id: Some(target.id.value()),
            max_depth: 3,
        }))
        .await
        .unwrap();
    assert_empty(&impact, "analyze_impact", target, 3);

    let reference = server
        .find_callers(Parameters(FindCallersRequest {
            function_name: None,
            symbol_id: Some(targets["reference"].id.value()),
        }))
        .await
        .unwrap();
    assert_eq!(graph(&reference)["status"], "resolved");
    assert!(text(&reference).contains("referenceConsumer"));
    assert!(!text(&callers).contains("referenceConsumer"));
}

#[tokio::test]
async fn local_calls_preserve_reverse_symmetry_and_requested_impact_depth() {
    let (_temp, server, targets) = fixture();
    let calls = server
        .get_calls(Parameters(GetCallsRequest {
            function_name: None,
            symbol_id: Some(targets["caller"].id.value()),
        }))
        .await
        .unwrap();
    assert_eq!(graph(&calls)["status"], "resolved");
    assert_eq!(graph(&calls)["returned"], 1);
    assert!(text(&calls).contains("calendarWeekStart"));
    assert!(text(&calls).contains("called at"));

    let callers = server
        .find_callers(Parameters(FindCallersRequest {
            function_name: None,
            symbol_id: Some(targets["callee"].id.value()),
        }))
        .await
        .unwrap();
    assert_eq!(graph(&callers)["returned"], 1);
    assert!(text(&callers).contains("readCalendarWeekStart"));

    for depth in [1, 2] {
        let impact = server
            .analyze_impact(Parameters(AnalyzeImpactRequest {
                symbol_name: None,
                symbol_id: Some(targets["callee"].id.value()),
                max_depth: depth,
            }))
            .await
            .unwrap();
        assert_eq!(graph(&impact)["max_depth"], depth);
        assert_eq!(graph(&impact)["returned"], depth);
        let rendered = text(&impact);
        assert!(rendered.contains("readCalendarWeekStart"));
        assert_eq!(rendered.contains("displayCalendarWeekStart"), depth == 2);
        assert!(!rendered.contains("would be affected"), "{rendered}");
    }
}

#[tokio::test]
async fn missing_and_ambiguous_targets_are_not_successful_empty_graphs() {
    let (_temp, server, _targets) = fixture();
    for name in ["missingCalendarImplementation", "isolatedCalendarToken"] {
        let result = server
            .get_calls(Parameters(GetCallsRequest {
                function_name: Some(name.into()),
                symbol_id: None,
            }))
            .await
            .unwrap();
        assert!(
            result
                .structured_content
                .as_ref()
                .is_none_or(|value| value["graph"]["status"] != "empty")
        );
        let rendered = text(&result);
        assert!(rendered.contains(name), "{rendered}");
        if name == "missingCalendarImplementation" {
            assert!(rendered.contains("not found"), "{rendered}");
        } else {
            assert!(rendered.contains("symbol_id"), "{rendered}");
            assert!(rendered.contains("active/calendar.ts"), "{rendered}");
            assert!(rendered.contains("reference/calendar.ts"), "{rendered}");
        }
    }

    let missing = server
        .get_calls(Parameters(GetCallsRequest {
            function_name: None,
            symbol_id: Some(u32::MAX),
        }))
        .await
        .unwrap();
    assert!(text(&missing).contains("Symbol not found"));
    assert!(missing.structured_content.is_none());
}

#[tokio::test]
async fn impact_budget_failure_is_not_rendered_as_an_empty_graph() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let facade = IndexFacade::new(Arc::new(settings)).unwrap();
    let index = facade.document_index();
    index.start_batch().unwrap();
    let mut writer = WriteStage::new(index.clone());
    for id in 1..=1002 {
        let symbol = Symbol::new(
            SymbolId::new(id).unwrap(),
            format!("node{id}"),
            SymbolKind::Function,
            FileId::new(1).unwrap(),
            Range::new(id - 1, 0, id - 1, 10),
        );
        index.index_symbol(&symbol, "dense.rs").unwrap();
        if id > 1 {
            writer
                .write_one(ResolvedRelationship::new(
                    symbol.id,
                    SymbolId::new(1).unwrap(),
                    RelationKind::Calls,
                ))
                .unwrap();
        }
    }
    writer.flush().unwrap();
    let server = CodeIntelligenceServer::new(facade);
    let result = server
        .analyze_impact(Parameters(AnalyzeImpactRequest {
            symbol_name: None,
            symbol_id: Some(1),
            max_depth: 1,
        }))
        .await;
    assert!(result.is_err(), "budget failures must not become empty");
}

#[test]
fn frozen_oracle_retains_negative_claim_and_depth_controls() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    assert_eq!(oracle["version"], 1);
    assert_eq!(oracle["semantic_search"], "disabled");
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 10);
    assert_eq!(
        oracle["cases"][6]["forbidden_targets"],
        json!(["transitive"])
    );
}
