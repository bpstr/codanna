//! MCP tool request types.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

/// Server-owned resource budgets. Internal full-index enumerations are not search APIs.
pub const MAX_SEARCH_LIMIT: u32 = 1000;
pub const MAX_IMPACT_DEPTH: u32 = 10;
pub const MAX_CONTEXT_LIMIT: u32 = 10;

pub(crate) fn validate_search_limit(limit: u32) -> Result<(), rmcp::model::ErrorData> {
    if (1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Ok(());
    }
    Err(rmcp::model::ErrorData::invalid_params(
        format!("limit must be between 1 and {MAX_SEARCH_LIMIT}"),
        None,
    ))
}

pub(crate) fn validate_impact_depth(depth: u32) -> Result<(), rmcp::model::ErrorData> {
    if (1..=MAX_IMPACT_DEPTH).contains(&depth) {
        return Ok(());
    }
    Err(rmcp::model::ErrorData::invalid_params(
        format!("max_depth must be between 1 and {MAX_IMPACT_DEPTH}"),
        None,
    ))
}

fn deserialize_limit<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = u32::deserialize(d)?;
    validate_search_limit(value).map_err(serde::de::Error::custom)?;
    Ok(value)
}

fn deserialize_depth<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = u32::deserialize(d)?;
    validate_impact_depth(value).map_err(serde::de::Error::custom)?;
    Ok(value)
}

pub(crate) fn validate_context_limit(
    field: &str,
    value: u32,
) -> Result<(), rmcp::model::ErrorData> {
    if (1..=MAX_CONTEXT_LIMIT).contains(&value) {
        return Ok(());
    }
    Err(rmcp::model::ErrorData::invalid_params(
        format!("{field} must be an integer between 1 and {MAX_CONTEXT_LIMIT}; received {value}"),
        None,
    ))
}

fn deserialize_context_limit<'de, D: serde::Deserializer<'de>>(
    d: D,
    field: &'static str,
) -> Result<u32, D::Error> {
    let value = u32::deserialize(d).map_err(|error| {
        <D::Error as serde::de::Error>::custom(format!(
            "{field} must be an integer between 1 and {MAX_CONTEXT_LIMIT}: {error}"
        ))
    })?;
    validate_context_limit(field, value).map_err(serde::de::Error::custom)?;
    Ok(value)
}

fn deserialize_code_limit<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    deserialize_context_limit(d, "code_limit")
}

fn deserialize_document_limit<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    deserialize_context_limit(d, "document_limit")
}

fn deserialize_conversation_limit<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    deserialize_context_limit(d, "conversation_limit")
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindSymbolRequest {
    /// Name of the symbol to find
    pub name: String,
    /// Filter by programming language (e.g., "rust", "python", "typescript", "php")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Maximum matching symbols to return (default: 100, maximum: 1000).
    #[serde(
        default = "default_symbol_limit",
        deserialize_with = "deserialize_limit"
    )]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: u32,
    /// Number of matching symbols to skip after language and owner filtering.
    #[serde(default)]
    pub offset: u32,
}

fn default_symbol_limit() -> u32 {
    100
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCallsRequest {
    /// Name of the function to analyze (use symbol_id for unambiguous lookup)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    /// Symbol ID for direct lookup (recommended to avoid ambiguity)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindCallersRequest {
    /// Name of the function to find callers for (use symbol_id for unambiguous lookup)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    /// Symbol ID for direct lookup (recommended to avoid ambiguity)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeImpactRequest {
    /// Name of the symbol to analyze impact for (use symbol_id for unambiguous lookup)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_name: Option<String>,
    /// Symbol ID for direct lookup (recommended to avoid ambiguity)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<u32>,
    /// Maximum depth to search (default: 3)
    #[serde(
        default = "default_depth",
        alias = "depth",
        deserialize_with = "deserialize_depth"
    )]
    #[schemars(range(min = 1, max = 10))]
    pub max_depth: u32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchSymbolsRequest {
    /// Search query (supports fuzzy matching)
    pub query: String,
    /// Maximum number of results (default: 10)
    #[serde(default = "default_limit", deserialize_with = "deserialize_limit")]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: u32,
    /// Filter by symbol kind (e.g., "Function", "Struct", "Trait")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Filter by module path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Filter by programming language (e.g., "rust", "python", "typescript", "php")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Limit symbol discovery to this workspace-relative path/subtree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticSearchRequest {
    /// Natural language search query
    pub query: String,
    /// Maximum number of results (default: 10)
    #[serde(default = "default_limit", deserialize_with = "deserialize_limit")]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: u32,
    /// Minimum similarity score (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    /// Filter by programming language (e.g., "rust", "python", "typescript", "php")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticSearchWithContextRequest {
    /// Natural language search query
    pub query: String,
    /// Maximum number of results (default: 5, as each includes full context)
    #[serde(
        default = "default_context_limit",
        deserialize_with = "deserialize_limit"
    )]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: u32,
    /// Minimum similarity score (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    /// Filter by programming language (e.g., "rust", "python", "typescript", "php")
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
        // MCP spec recommends `{"type":"object","additionalProperties":false}` for
        // no-parameter tools. We also include an empty `properties` map because
        // OpenAI's strict function-calling validation rejects object schemas that
        // lack `properties` entirely.
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
    /// Natural language search query
    pub query: String,
    /// Filter by collection name (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// Maximum number of results (default: 5)
    #[serde(
        default = "default_context_limit",
        deserialize_with = "deserialize_limit"
    )]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: u32,
}

/// Unified topic lookup across code, indexed documents, and shared conversation recall.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchContextRequest {
    /// Topic or phrase to investigate across all available context sources.
    pub query: String,
    /// Maximum code-symbol matches (default: 5, max: 10).
    #[serde(
        default = "default_context_limit",
        deserialize_with = "deserialize_code_limit"
    )]
    #[schemars(range(min = 1, max = 10))]
    pub code_limit: u32,
    /// Maximum document chunks (default: 5, max: 10).
    #[serde(
        default = "default_context_limit",
        deserialize_with = "deserialize_document_limit"
    )]
    #[schemars(range(min = 1, max = 10))]
    pub document_limit: u32,
    /// Maximum conversation messages (default: 5, max: 10).
    #[serde(
        default = "default_context_limit",
        deserialize_with = "deserialize_conversation_limit"
    )]
    #[schemars(range(min = 1, max = 10))]
    pub conversation_limit: u32,
    /// Optional document collection filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// Optional workspace-relative subtree applied to the code-symbol section only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_path_prefix: Option<String>,
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
    fn search_context_defaults_and_limits_are_bounded() {
        let request: SearchContextRequest =
            serde_json::from_value(json!({"query": "status bar"})).unwrap();
        assert_eq!(request.code_limit, 5);
        assert_eq!(request.document_limit, 5);
        assert_eq!(request.conversation_limit, 5);

        for key in ["code_limit", "document_limit", "conversation_limit"] {
            for limit in [0, MAX_CONTEXT_LIMIT + 1, u32::MAX] {
                assert!(
                    serde_json::from_value::<SearchContextRequest>(
                        json!({"query": "q", key: limit})
                    )
                    .is_err()
                );
            }
        }
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
        assert!(
            msg.contains("bogus") && msg.contains("function_name"),
            "rejection must name the offending field and the accepted set: {msg}"
        );
    }
}

#[cfg(test)]
mod review_limit_tests {
    use super::*;
    #[test]
    fn hardening_review_search_limits_reject_zero_and_excessive_values() {
        for limit in [0, MAX_SEARCH_LIMIT + 1, u32::MAX] {
            let value = serde_json::json!({"query": "q", "limit": limit});
            assert!(serde_json::from_value::<SearchSymbolsRequest>(value.clone()).is_err());
            assert!(serde_json::from_value::<SemanticSearchRequest>(value.clone()).is_err());
            assert!(
                serde_json::from_value::<SemanticSearchWithContextRequest>(value.clone()).is_err()
            );
            assert!(serde_json::from_value::<SearchDocumentsRequest>(value).is_err());
            assert!(validate_search_limit(limit).is_err());
        }
        for limit in [1, MAX_SEARCH_LIMIT] {
            let value = serde_json::json!({"query": "q", "limit": limit});
            assert_eq!(
                serde_json::from_value::<SearchSymbolsRequest>(value)
                    .unwrap()
                    .limit,
                limit
            );
        }
        for key in ["depth", "max_depth"] {
            for depth in [0, MAX_IMPACT_DEPTH + 1, u32::MAX] {
                let value = serde_json::json!({key: depth});
                assert!(serde_json::from_value::<AnalyzeImpactRequest>(value).is_err());
            }
        }
        assert_eq!(
            serde_json::from_value::<SearchSymbolsRequest>(serde_json::json!({"query":"q"}))
                .unwrap()
                .limit,
            10
        );
    }
}
