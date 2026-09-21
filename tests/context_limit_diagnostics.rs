//! Search-context limits identify caller mistakes without relaxing resource budgets.
//! Request tests require no index or provider; direct calls fail before retrieval.

use codanna::indexing::facade::IndexFacade;
use codanna::mcp::requests::{MAX_CONTEXT_LIMIT, SearchContextRequest};
use codanna::mcp::server::CodeIntelligenceServer;
use codanna::Settings;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use std::sync::Arc;

const FIELDS: [&str; 3] = ["code_limit", "document_limit", "conversation_limit"];

fn request_with(field: &str, value: Value) -> Value {
    let mut request = json!({"query": "calendar preference"});
    request[field] = value;
    request
}

fn assert_diagnostic(message: &str, field: &str) {
    assert!(message.contains(field), "{field}: {message}");
    assert!(message.contains("integer"), "{message}");
    assert!(message.contains("between 1 and 10"), "{message}");
    assert!(!message.contains('\u{1b}'), "ANSI in error: {message:?}");
}

#[test]
fn original_zero_conversation_limit_names_the_offending_field() {
    let error = serde_json::from_value::<SearchContextRequest>(json!({
        "query": "calendar preference",
        "code_limit": 10,
        "document_limit": 10,
        "conversation_limit": 0
    }))
    .unwrap_err();
    assert_diagnostic(&error.to_string(), "conversation_limit");
    assert!(error.to_string().contains("received 0"));
}

#[test]
fn every_limit_rejects_out_of_range_and_non_integer_values_with_its_field_name() {
    let invalid = [
        json!(0),
        json!(MAX_CONTEXT_LIMIT + 1),
        json!(u32::MAX),
        json!(u64::MAX),
        json!(-1),
        json!(1.5),
        json!("5"),
        json!(true),
        Value::Null,
        json!([]),
        json!({}),
    ];
    for field in FIELDS {
        for value in &invalid {
            let request = request_with(field, value.clone());
            // Both serde entry points are used by adapters in practice. Do not
            // accidentally accept a stringified number at either boundary.
            let value_error = serde_json::from_value::<SearchContextRequest>(request.clone())
                .unwrap_err()
                .to_string();
            let text_error = serde_json::from_str::<SearchContextRequest>(&request.to_string())
                .unwrap_err()
                .to_string();
            assert_diagnostic(&value_error, field);
            assert_diagnostic(&text_error, field);
        }
    }
}

#[test]
fn accepted_boundaries_defaults_and_roundtrips_remain_unchanged() {
    let default: SearchContextRequest =
        serde_json::from_value(json!({"query": "calendar"})).unwrap();
    for field in FIELDS {
        assert_eq!(serde_json::to_value(&default).unwrap()[field], 5);
        for limit in [1, MAX_CONTEXT_LIMIT] {
            let request: SearchContextRequest =
                serde_json::from_value(request_with(field, json!(limit))).unwrap();
            let encoded = serde_json::to_value(&request).unwrap();
            assert_eq!(encoded[field], limit);
            let decoded: SearchContextRequest = serde_json::from_value(encoded.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
        }
    }
    assert!(
        serde_json::from_value::<SearchContextRequest>(json!({
            "query": "calendar", "conversation_limt": 1
        }))
        .unwrap_err()
        .to_string()
        .contains("conversation_limt")
    );
}

#[test]
fn advertised_schema_matches_runtime_limits_and_optional_defaults() {
    let schema = serde_json::to_value(rmcp::schemars::schema_for!(SearchContextRequest)).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    for field in FIELDS {
        let property = &schema["properties"][field];
        assert_eq!(property["type"], "integer", "{property}");
        assert_eq!(property["minimum"], 1, "{property}");
        assert_eq!(property["maximum"], MAX_CONTEXT_LIMIT, "{property}");
        assert_eq!(property["default"], 5, "{property}");
        assert!(
            !schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|required| required == field)
        );
    }
}

#[tokio::test]
async fn direct_calls_reject_limits_before_any_retrieval_or_recall() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let facade = IndexFacade::new(Arc::new(settings)).unwrap();
    let server = CodeIntelligenceServer::new(facade);
    for field in FIELDS {
        for limit in [0, MAX_CONTEXT_LIMIT + 1, u32::MAX] {
            let mut request: SearchContextRequest =
                serde_json::from_value(json!({"query": "calendar"})).unwrap();
            match field {
                "code_limit" => request.code_limit = limit,
                "document_limit" => request.document_limit = limit,
                "conversation_limit" => request.conversation_limit = limit,
                _ => unreachable!(),
            }
            let result = server.search_context(Parameters(request)).await.unwrap();
            assert_eq!(result.is_error, Some(true));
            let encoded = serde_json::to_value(result).unwrap();
            let message = encoded["content"][0]["text"].as_str().unwrap();
            assert_diagnostic(message, field);
            assert!(!message.contains("## Code"));
            assert!(!message.contains("## Conversations"));
        }
    }
}
