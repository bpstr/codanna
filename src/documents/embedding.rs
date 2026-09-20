//! Shared configured backend for document indexing and every query surface.
use crate::SymbolId;
use crate::config::SemanticSearchConfig;
use crate::indexing::facade::{build_embedding_backend, resolve_remote_model_name};
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
        let identity = backend_identity(config);
        Ok(Self {
            backend,
            dimension,
            identity,
        })
    }
}

fn backend_identity(config: &SemanticSearchConfig) -> String {
    let remote_url = std::env::var("CODANNA_EMBED_URL")
        .ok()
        .or_else(|| config.remote_url.clone());
    match remote_url {
        Some(url) => {
            // Endpoint identity matters when two self-hosted servers expose the
            // same model alias. Persist its digest, never a URL or API key.
            let endpoint = crate::indexing::file_info::calculate_hash(url.trim_end_matches('/'));
            format!("remote:{}:{endpoint}", resolve_remote_model_name(config))
        }
        None => format!("local:{}", config.model),
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
    use super::*;

    #[test]
    fn identity_tracks_endpoint_and_model_without_persisting_url_credentials() {
        let mut settings = crate::Settings::default().semantic_search;
        settings.remote_url = Some("http://username:secret@127.0.0.1:9999".into());
        settings.remote_model = Some("fixture@1".into());
        let first = backend_identity(&settings);
        assert!(!first.contains("secret"));
        assert!(!first.contains("127.0.0.1"));
        settings.remote_url = Some("http://127.0.0.1:9998".into());
        assert_ne!(first, backend_identity(&settings));
        settings.remote_model = Some("fixture@2".into());
        assert_ne!(first, backend_identity(&settings));
    }
}
