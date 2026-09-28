//! Definition lookup contracts using a deterministic index and no providers.

use codanna::indexing::facade::IndexFacade;
use codanna::mcp::{CodeIntelligenceServer, FindSymbolRequest};
use codanna::{FileId, Range, Settings, Symbol, SymbolId, SymbolKind};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture() -> (tempfile::TempDir, CodeIntelligenceServer) {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let facade = IndexFacade::new(Arc::new(settings)).unwrap();
    let index = facade.document_index();
    index.start_batch().unwrap();
    for (id, path) in [(1, "first.ts"), (2, "second.ts")] {
        let mut symbol = Symbol::new(
            SymbolId::new(id).unwrap(), "shared", SymbolKind::Function,
            FileId::new(id).unwrap(), Range::new(0, 0, 2, 1),
        ).with_signature("function shared(): number").with_doc("Fixture definition.");
        symbol.language_id = Some(codanna::parsing::LanguageId::new("typescript"));
        index.index_symbol(&symbol, path).unwrap();
    }
    index.commit_batch().unwrap();
    (temp, CodeIntelligenceServer::new(facade))
}

fn request(value: Value) -> FindSymbolRequest {
    serde_json::from_value(value).unwrap()
}

fn text(response: &CallToolResult) -> String {
    response.content.iter().filter_map(|block| match block {
        ContentBlock::Text(text) => Some(text.text.as_str()),
        _ => None,
    }).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn typed_id_disambiguates_definition_and_preserves_legacy_lookup() {
    let (_temp, server) = fixture();
    let names = server.find_symbol(Parameters(request(json!({"name":"shared"})))).await.unwrap();
    assert_eq!(names.structured_content.unwrap()["pagination"]["total"], 2);
    let typed = server.find_symbol(Parameters(request(json!({"symbol_id":2})))).await.unwrap();
    let legacy = server.find_symbol(Parameters(request(json!({"name":"symbol_id:2"})))).await.unwrap();
    assert_eq!(text(&typed), text(&legacy));
    assert_eq!(typed.structured_content, legacy.structured_content);
    let rendered = text(&typed);
    assert!(rendered.contains("second.ts") && rendered.contains("Fixture definition."));
    assert!(!rendered.contains("first.ts"));
}

#[tokio::test]
async fn unknown_ids_and_language_filters_do_not_fall_back_to_namesakes() {
    let (_temp, server) = fixture();
    for input in [json!({"symbol_id":3}), json!({"symbol_id":2,"lang":"go"})] {
        let response = server.find_symbol(Parameters(request(input))).await.unwrap();
        assert_eq!(response.structured_content.as_ref().unwrap()["pagination"]["total"], 0);
        let rendered = text(&response);
        assert!(!rendered.contains("first.ts") && !rendered.contains("second.ts"));
    }
    let response = server.find_symbol(Parameters(request(json!({"symbol_id":2,"offset":1})))).await.unwrap();
    let page = &response.structured_content.unwrap()["pagination"];
    assert_eq!(page["total"], 1);
    assert_eq!(page["returned"], 0);
}

#[tokio::test]
async fn missing_zero_and_conflicting_targets_fail_before_lookup() {
    let (_temp, server) = fixture();
    for input in [
        json!({}), json!({"name":"   "}), json!({"symbol_id":0}),
        json!({"name":"shared","symbol_id":2}),
        json!({"name":"symbol_id:1","symbol_id":2}),
    ] {
        assert!(server.find_symbol(Parameters(request(input))).await.is_err());
    }
    for input in [
        json!({"symbol_id":-1}), json!({"symbol_id":"2"}),
        json!({"symbol_id":4294967296u64}),
    ] {
        assert!(serde_json::from_value::<FindSymbolRequest>(input).is_err());
    }
}

#[test]
fn advertised_schema_accepts_id_only_definition_followups() {
    let schema = serde_json::to_value(rmcp::schemars::schema_for!(FindSymbolRequest)).unwrap();
    assert!(schema["properties"].get("symbol_id").is_some());
    let required = schema["required"].as_array();
    assert!(required.is_none_or(|fields| !fields.iter().any(|field| field == "name")));
}
