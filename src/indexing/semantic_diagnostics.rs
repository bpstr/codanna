use super::IndexFacade;
use crate::Symbol;

/// Membership is observed by numeric ID, not proof of source/vector freshness.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SemanticDefinitionStatus {
    pub symbol_id: u32,
    pub name: String,
    pub file_path: String,
    pub state: &'static str,
    pub eligible: bool,
    pub source_input_policy: &'static str,
    pub recorded_source_policy: Option<String>,
    pub representation_status: &'static str,
    pub embedding_identity_sha256: Option<String>,
    pub vector_presence: &'static str,
    pub vector_presence_basis: &'static str,
    pub vector_count: Option<usize>,
    pub code_generation: u64,
    pub vector_code_generation: Option<u64>,
    pub generation_alignment: &'static str,
    pub freshness: &'static str,
}

impl IndexFacade {
    /// Describe only the selected definitions without loading vectors or a model.
    /// A metadata-only reader cannot infer membership from the aggregate count.
    pub fn semantic_definition_status(&self, symbols: &[Symbol]) -> Vec<SemanticDefinitionStatus> {
        let semantic = self.semantic_search.as_ref().and_then(|s| s.lock().ok());
        let metadata = semantic
            .as_ref()
            .and_then(|s| s.metadata())
            .or(self.semantic_metadata_snapshot.as_ref());
        let identity = metadata.and_then(|m| m.embedding_identity.as_deref());
        let recorded_source_policy = identity
            .and_then(|i| serde_json::from_str::<serde_json::Value>(i).ok())
            .and_then(|v| {
                v.get("source_input_policy")
                    .and_then(|p| p.as_str())
                    .map(str::to_owned)
            });
        let policy = self.settings.semantic_search.code_representation;
        let representation_status = match recorded_source_policy.as_deref() {
            Some(recorded) if recorded == policy.source_policy() => "matched",
            Some(_) => "mismatch",
            None => "unknown_legacy_or_absent",
        };
        let state = if self.semantic_incompatible {
            "incompatible"
        } else if semantic.is_some() {
            "live"
        } else if self.semantic_search.is_some() {
            "unavailable"
        } else if metadata.is_some() {
            "metadata_only"
        } else {
            "disabled"
        };
        let identity_sha256 = identity.map(crate::indexing::calculate_hash);
        let code_generation = self.document_index.generation();
        symbols
            .iter()
            .map(|symbol| {
                let count = semantic.as_ref().map(|s| s.symbol_vector_count(symbol.id));
                SemanticDefinitionStatus {
                    symbol_id: symbol.id.value(),
                    name: symbol.name.to_string(),
                    file_path: symbol.file_path.to_string(),
                    state,
                    eligible: policy.eligible(symbol.kind, symbol.doc_comment.is_some()),
                    source_input_policy: policy.source_policy(),
                    recorded_source_policy: recorded_source_policy.clone(),
                    representation_status,
                    embedding_identity_sha256: identity_sha256.clone(),
                    vector_presence: match count {
                        Some(0) => "missing",
                        Some(_) => "present",
                        None => "unknown",
                    },
                    vector_presence_basis: "numeric_symbol_id_only",
                    vector_count: count,
                    code_generation,
                    // Current vector persistence records neither the producing code
                    // generation nor a per-definition source hash. Do not infer it.
                    vector_code_generation: None,
                    generation_alignment: "unknown_untracked",
                    freshness: "unknown",
                }
            })
            .collect()
    }
}
