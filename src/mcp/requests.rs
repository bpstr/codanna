//! MCP tool request types.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindSymbolRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCallsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindCallersRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeImpactRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<u32>,
    #[serde(default = "default_depth", alias = "depth")]
    pub max_depth: u32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchSymbolsRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticSearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticSearchWithContextRequest {
    pub query: String,
    #[serde(default = "default_context_limit")]
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetIndexInfoRequest {}

impl schemars::JsonSchema for GetIndexInfoRequest {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("GetIndexInfoRequest")
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed(concat!(module_path!(), "::GetIndexInfoRequest"))
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::Schema::from(
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })
            .as_object()
            .unwrap()
            .clone(),
        )
    }
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchDocumentsRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    #[serde(default = "default_context_limit")]
    pub limit: u32,
}

/// Unified topic lookup across code, indexed documents, and shared conversation recall.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchContextRequest {
    /// Topic or phrase to investigate across all available context sources.
    pub query: String,
    /// Maximum code-symbol matches (default: 5, max: 10).
    #[serde(default = "default_context_limit")]
    pub code_limit: u32,
    /// Maximum document chunks (default: 5, max: 10).
    #[serde(default = "default_context_limit")]
    pub document_limit: u32,
    /// Maximum conversation messages (default: 5, max: 10).
    #[serde(default = "default_context_limit")]
    pub conversation_limit: u32,
    /// Optional document collection filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
}

fn default_depth() -> u32 {
    3
}

fn default_limit() -> u32 {
    10
}

fn default_context_limit() -> u32 {
    5
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_keys_reject_across_request_structs() {
        assert!(
            serde_json::from_value::<FindSymbolRequest>(json!({"name": "x", "bogus": 1})).is_err()
        );
        assert!(serde_json::from_value::<GetCallsRequest>(json!({"bogus": 1})).is_err());
        assert!(serde_json::from_value::<FindCallersRequest>(json!({"langg": "rust"})).is_err());
        assert!(
            serde_json::from_value::<AnalyzeImpactRequest>(
                json!({"symbol_name": "x", "depths": 2})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<SearchSymbolsRequest>(
                json!({"query": "q", "kindd": "function"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<SemanticSearchRequest>(json!({"query": "q", "treshold": 0.5}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<SemanticSearchWithContextRequest>(
                json!({"query": "q", "x": 1})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<GetIndexInfoRequest>(json!({"bogus": 1})).is_err());
        assert!(
            serde_json::from_value::<SearchDocumentsRequest>(
                json!({"query": "q", "collections": "a"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<SearchContextRequest>(
                json!({"query": "q", "memory_limit": 5})
            )
            .is_err()
        );
    }

    #[test]
    fn search_context_defaults_are_small() {
        let req: SearchContextRequest =
            serde_json::from_value(json!({"query": "status bar"})).unwrap();
        assert_eq!(req.code_limit, 5);
        assert_eq!(req.document_limit, 5);
        assert_eq!(req.conversation_limit, 5);
    }

    #[test]
    fn depth_aliases_max_depth() {
        let req: AnalyzeImpactRequest =
            serde_json::from_value(json!({"symbol_name": "x", "depth": 2})).expect("alias applies");
        assert_eq!(req.max_depth, 2);
        let req: AnalyzeImpactRequest =
            serde_json::from_value(json!({"symbol_name": "x"})).expect("default applies");
        assert_eq!(req.max_depth, 3);
    }

    #[test]
    fn rejection_names_the_field_and_accepted_keys() {
        let err = serde_json::from_value::<GetCallsRequest>(json!({"bogus": 1})).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bogus") && msg.contains("function_name"));
    }
}
