//! Shared configured backend for document indexing and every query surface.
use crate::SymbolId;
use crate::config::SemanticSearchConfig;
use crate::indexing::facade::build_embedding_backend;
use crate::semantic::EmbeddingBackend;
use crate::vector::{EmbeddingGenerator, VectorDimension, VectorError};

pub(super) struct ConfiguredGenerator {
    backend: EmbeddingBackend,
    dimension: VectorDimension,
    identity: String,
}

impl ConfiguredGenerator {
    pub(super) fn new(config: &SemanticSearchConfig) -> Result<Self, VectorError> {
        let backend = build_embedding_backend(config)
            .map_err(|error| VectorError::EmbeddingFailed(error.to_string()))?;
        let dimension = VectorDimension::new(backend.dimensions())?;
        let identity = backend.identity(config.model_revision.as_deref());
        Ok(Self {
            backend,
            dimension,
            identity,
        })
    }
}

impl EmbeddingGenerator for ConfiguredGenerator {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let embeddings = match &self.backend {
            EmbeddingBackend::Remote(remote) => {
                let remote = remote.clone();
                let texts: Vec<String> = texts.iter().map(|text| (*text).to_string()).collect();
                crate::semantic::remote::run_async(async move { remote.embed(&texts).await })
                    .map_err(|error| VectorError::EmbeddingFailed(error.to_string()))?
            }
            EmbeddingBackend::Local(pool) => {
                let items: Vec<_> = texts
                    .iter()
                    .enumerate()
                    .map(|(index, text)| {
                        let raw = u32::try_from(index + 1).map_err(|_| {
                            VectorError::EmbeddingFailed(
                                "Document embedding batch exceeds ID capacity".into(),
                            )
                        })?;
                        let id = SymbolId::new(raw).ok_or_else(|| {
                            VectorError::EmbeddingFailed(
                                "Invalid document embedding batch ID".into(),
                            )
                        })?;
                        Ok((id, *text, "document"))
                    })
                    .collect::<Result<_, VectorError>>()?;
                let mut results = pool
                    .embed_parallel(&items)
                    .map_err(|error| VectorError::EmbeddingFailed(error.to_string()))?;
                results.sort_by_key(|(id, _, _)| id.to_u32());
                if results.len() != items.len()
                    || results
                        .iter()
                        .zip(&items)
                        .any(|(actual, expected)| actual.0 != expected.0)
                {
                    return Err(VectorError::EmbeddingFailed(
                        "Local document embedding backend returned an incomplete batch".into(),
                    ));
                }
                results
                    .into_iter()
                    .map(|(_, embedding, _)| embedding)
                    .collect()
            }
        };
        if embeddings.len() != texts.len() {
            return Err(VectorError::EmbeddingFailed(format!(
                "Document embedding backend returned {} vectors for {} inputs",
                embeddings.len(),
                texts.len()
            )));
        }
        for vector in &embeddings {
            self.dimension.validate_vector(vector)?;
        }
        Ok(embeddings)
    }

    fn dimension(&self) -> VectorDimension {
        self.dimension
    }
    fn cache_identity(&self) -> String {
        self.identity.clone()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn identity_tracks_endpoint_and_model_without_persisting_url_credentials() {
        let budget = crate::embedding_input::InputBudget::remote(None, None).unwrap();
        let identity = |url: &str, model: &str, revision: &str| {
            let endpoint = crate::indexing::file_info::calculate_hash(url);
            crate::embedding_input::backend_identity(
                "remote",
                model,
                Some(&endpoint),
                Some(revision),
                &budget,
            )
        };
        let first = identity(
            "http://username:secret@127.0.0.1:9999",
            "fixture",
            "revision-1",
        );
        assert!(!first.contains("secret"));
        assert!(!first.contains("127.0.0.1"));
        assert_ne!(
            first,
            identity("http://127.0.0.1:9998", "fixture", "revision-1")
        );
        assert_ne!(
            first,
            identity(
                "http://username:secret@127.0.0.1:9999",
                "other",
                "revision-1"
            )
        );
        assert_ne!(
            first,
            identity(
                "http://username:secret@127.0.0.1:9999",
                "fixture",
                "revision-2"
            )
        );
        assert!(first.contains(crate::embedding_input::INPUT_POLICY_VERSION));
    }
}
