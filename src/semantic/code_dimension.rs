//! The code journal's dimension contract, distinct from document backends.
use super::{EmbeddingBackend, SemanticSearchError};
use crate::config::SemanticSearchConfig;
use crate::{IndexError, IndexResult};

/// Maximum readable code-journal dimension. Does not limit document vectors.
pub const MAX_CODE_EMBEDDING_DIMENSION: usize = 4096;

/// Validate before code inference and before publishing any semantic artifact.
pub fn validate_code_embedding_dimension(dimension: usize) -> Result<(), SemanticSearchError> {
    if (1..=MAX_CODE_EMBEDDING_DIMENSION).contains(&dimension) {
        return Ok(());
    }
    Err(SemanticSearchError::StorageError {
        message: format!(
            "Code embedding dimension {dimension} is unsupported; expected 1..={MAX_CODE_EMBEDDING_DIMENSION}"
        ),
        suggestion: "Choose a supported code embedding dimension. Preserve existing index files; changing the cache limit does not change the journal format.".into(),
    })
}

/// Resolve an explicit remote code dimension without initializing a provider.
/// An environment override takes precedence over the file, exactly as at runtime.
pub(crate) fn configured_code_dimension(
    config: &SemanticSearchConfig,
) -> Result<Option<usize>, SemanticSearchError> {
    if std::env::var("CODANNA_EMBED_URL").is_err() && config.remote_url.is_none() {
        return Ok(None);
    }
    let dimension = match std::env::var("CODANNA_EMBED_DIM") {
        Ok(value) => Some(value.parse::<usize>().map_err(|_| {
            SemanticSearchError::ModelInitError(format!(
                "CODANNA_EMBED_DIM must be an integer in 1..={MAX_CODE_EMBEDDING_DIMENSION}"
            ))
        })?),
        Err(std::env::VarError::NotPresent) => config.remote_dim,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(SemanticSearchError::ModelInitError(
                "CODANNA_EMBED_DIM must be a Unicode integer".into(),
            ));
        }
    };
    if let Some(dimension) = dimension {
        validate_code_embedding_dimension(dimension)?;
    }
    Ok(dimension)
}

/// Code-only wrapper. Shared document construction retains its own contract.
/// Unknown remote dimensions require a probe, but never source inference here.
pub fn build_code_embedding_backend(
    config: &SemanticSearchConfig,
) -> IndexResult<EmbeddingBackend> {
    configured_code_dimension(config).map_err(IndexError::SemanticSearch)?;
    let backend = crate::indexing::facade::build_embedding_backend(config)?;
    validate_code_embedding_dimension(backend.dimensions()).map_err(IndexError::SemanticSearch)?;
    Ok(backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_dimension_contract_accepts_only_readable_journal_dimensions() {
        for dimension in [1, 2, 4096] {
            validate_code_embedding_dimension(dimension).unwrap();
        }
        for dimension in [0, 4097, 16384, usize::MAX] {
            let error = validate_code_embedding_dimension(dimension)
                .unwrap_err()
                .to_string();
            assert!(error.contains("1..=4096"));
            assert!(error.contains(&dimension.to_string()));
        }
    }
}
