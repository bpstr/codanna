use super::IndexFacade;
use crate::Symbol;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, io::Read, path::Path};

const MAX_FRESHNESS_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FRESHNESS_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

fn current_source_sha256(path: &Path, remaining: &mut u64) -> (Option<String>, &'static str) {
    let mut file = match crate::documents::drift::open_regular_source(path) {
        Ok(Some(file)) => file,
        Ok(None) => return (None, "unavailable"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (None, "missing"),
        Err(_) => return (None, "unavailable"),
    };
    let Some(length) = file.metadata().ok().map(|metadata| metadata.len()) else {
        return (None, "unavailable");
    };
    if length > MAX_FRESHNESS_SOURCE_BYTES || length > *remaining {
        return (None, "unavailable");
    }
    let mut bytes = Vec::new();
    if file
        .by_ref()
        .take(MAX_FRESHNESS_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 != length
    {
        return (None, "unavailable");
    }
    *remaining -= length;
    (Some(hex::encode(Sha256::digest(&bytes))), "observed")
}

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
    pub indexed_content_hash: Option<String>,
    pub current_content_hash: Option<String>,
    pub vector_source_hash: Option<String>,
    pub source_freshness: &'static str,
    pub reader_generation: u64,
    pub code_generation: Option<u64>,
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
        let reader_generation = self.document_index.generation();
        let code_generation = self.document_index.commit_opstamp().ok();
        let mut remaining_source_bytes = MAX_FRESHNESS_TOTAL_BYTES;
        let mut observed_sources: HashMap<String, (Option<String>, &'static str)> = HashMap::new();
        symbols
            .iter()
            .map(|symbol| {
                let count = semantic.as_ref().map(|s| s.symbol_vector_count(symbol.id));
                // Symbols expose portable paths, while the file registration retains
                // the path that was actually indexed. Use that stored path for both
                // the indexed hash lookup and the bounded source observation.
                let stored_path = self
                    .document_index
                    .get_file_path(symbol.file_id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| symbol.file_path.to_string());
                let indexed_content_hash = self
                    .document_index
                    .get_file_info(&stored_path)
                    .ok()
                    .flatten()
                    .map(|(_, hash, _)| hash);
                let (current_content_hash, observation) = observed_sources
                    .entry(stored_path.clone())
                    .or_insert_with(|| {
                        current_source_sha256(Path::new(&stored_path), &mut remaining_source_bytes)
                    })
                    .clone();
                let source_freshness = match (
                    indexed_content_hash.as_deref(),
                    current_content_hash.as_deref(),
                    observation,
                ) {
                    (Some(indexed), Some(current), _) if indexed == current => {
                        "matching_observed_hashes"
                    }
                    (Some(_), Some(_), _) => "stale",
                    (Some(_), None, "missing") => "missing",
                    (Some(_), None, _) => "unavailable",
                    _ => "unknown_untracked",
                };
                let provenance = semantic
                    .as_ref()
                    .and_then(|search| search.symbol_provenance(symbol.id));
                let vector_source_hash = provenance.map(|value| value.source_sha256.clone());
                let vector_code_generation = provenance.map(|value| value.code_generation);
                let generation_alignment = match (vector_code_generation, code_generation) {
                    (Some(vector_generation), Some(code_generation))
                        if vector_generation == code_generation =>
                    {
                        "matched"
                    }
                    (Some(_), Some(_)) => "stale",
                    _ => "unknown_untracked",
                };
                let freshness = if source_freshness == "stale" {
                    "stale_source"
                } else if source_freshness == "matching_observed_hashes"
                    && generation_alignment == "matched"
                    && vector_source_hash.as_deref() == indexed_content_hash.as_deref()
                {
                    "verified"
                } else if provenance.is_some() {
                    "stale_vector"
                } else {
                    "unknown"
                };
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
                    indexed_content_hash,
                    current_content_hash,
                    vector_source_hash,
                    source_freshness,
                    reader_generation,
                    code_generation,
                    vector_code_generation,
                    generation_alignment,
                    freshness,
                }
            })
            .collect()
    }
}
