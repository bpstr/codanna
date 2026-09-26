//! Simple semantic search implementation for documentation comments

#[path = "rebuild_cache.rs"]
mod rebuild_cache;

use crate::SymbolId;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Error type for semantic search operations
#[derive(Debug, thiserror::Error)]
pub enum SemanticSearchError {
    #[error("Failed to initialize embedding model: {0}")]
    ModelInitError(String),

    #[error("Failed to generate embedding: {0}")]
    EmbeddingError(String),

    #[error("No embeddings available for search")]
    NoEmbeddings,

    #[error("Storage error: {message}\nSuggestion: {suggestion}")]
    StorageError { message: String, suggestion: String },

    #[error("Dimension mismatch: expected {expected}, got {actual}\nSuggestion: {suggestion}")]
    DimensionMismatch {
        expected: usize,
        actual: usize,
        suggestion: String,
    },

    #[error("Invalid ID: {id}\nSuggestion: {suggestion}")]
    InvalidId { id: u32, suggestion: String },

    #[error(
        "Embedding pool exhausted: no instance available after {waited:?} (pool size {pool_size})\nSuggestion: An embedder is wedged or leaked. Sample the process to capture stacks, then restart the run."
    )]
    PoolExhausted {
        pool_size: usize,
        waited: std::time::Duration,
    },
}

/// A bounded embedding input belonging to a real parent symbol, never a fabricated graph node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SymbolSegment {
    pub source_range: Option<std::ops::Range<usize>>,
    pub vector: Arc<[f32]>,
    #[serde(skip)]
    magnitude: f32,
}

impl SymbolSegment {
    pub(crate) fn new(source_range: Option<std::ops::Range<usize>>, vector: Arc<[f32]>) -> Self {
        let magnitude = vector_magnitude(&vector);
        Self {
            source_range,
            vector,
            magnitude,
        }
    }

    pub(super) fn validate(&mut self, dimension: usize) -> Result<(), SemanticSearchError> {
        if self.vector.len() != dimension
            || self.vector.iter().any(|value| !value.is_finite())
            || self
                .source_range
                .as_ref()
                .is_some_and(|range| range.start >= range.end)
        {
            return Err(super::journal::error(
                "invalid symbol segment vector or source range",
            ));
        }
        self.magnitude = vector_magnitude(&self.vector);
        Ok(())
    }
}

pub(super) type SymbolSegments = HashMap<SymbolId, Arc<[SymbolSegment]>>;

/// Advanced semantic search engine for documentation analysis
///
/// This implementation uses state-of-the-art embeddings to find
/// semantically similar documentation across the entire codebase,
/// enabling natural language queries for code discovery.
/// Queries can use immutable snapshots while indexing updates a later generation.
pub struct SimpleSemanticSearch {
    /// Embeddings indexed by symbol ID
    embeddings: Arc<HashMap<SymbolId, Arc<[f32]>>>,
    symbol_segments: Arc<SymbolSegments>,

    /// Precomputed vector magnitudes, maintained with `embeddings`.
    embedding_magnitudes: Arc<HashMap<SymbolId, f32>>,

    /// Language mapping for each symbol (for language-filtered search)
    symbol_languages: Arc<HashMap<SymbolId, String>>,

    /// Candidate IDs grouped by language so filtered queries skip other vectors.
    language_symbols: Arc<HashMap<String, Arc<HashSet<SymbolId>>>>,

    /// Content-addressed accelerator for unchanged embedding inputs.
    embedding_cache: crate::embedding_cache::EmbeddingCache,

    /// The embedding model for query-time embedding (None in remote mode — caller
    /// must use `search_with_embedding` and provide the query vector externally).
    model: Option<Arc<Mutex<TextEmbedding>>>,
    input_budget: Option<crate::embedding_input::InputBudget>,

    /// Model dimensions for validation
    dimensions: usize,

    /// Metadata for tracking model info and timestamps
    metadata: Option<crate::semantic::SemanticMetadata>,

    persistence: Arc<Mutex<super::journal::Persistence>>,
    persist_io: Arc<Mutex<()>>,
}

impl std::fmt::Debug for SimpleSemanticSearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleSemanticSearch")
            .field("embeddings_count", &self.embeddings.len())
            .field("dimensions", &self.dimensions)
            .field("model", &"<TextEmbedding>")
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl SimpleSemanticSearch {
    /// Create a new semantic search instance using default model (AllMiniLML6V2).
    ///
    /// For multilingual support, use `from_model_name` with "MultilingualE5Small".
    pub fn new() -> Result<Self, SemanticSearchError> {
        Self::with_model(EmbeddingModel::AllMiniLML6V2)
    }

    /// Create a semantic search instance from a model name string.
    ///
    /// # Arguments
    /// * `model_name` - Name of the model (e.g., "MultilingualE5Small", "AllMiniLML6V2")
    ///
    /// # Supported Models
    /// - `AllMiniLML6V2` - English-only, 384 dimensions (default)
    /// - `MultilingualE5Small` - 94 languages, 384 dimensions (recommended for multilingual)
    /// - `MultilingualE5Base` - 94 languages, 768 dimensions
    /// - `MultilingualE5Large` - 94 languages, 1024 dimensions
    /// - And many more (see `parse_embedding_model` documentation)
    ///
    /// # Example
    /// ```ignore
    /// let search = SimpleSemanticSearch::from_model_name("MultilingualE5Small")?;
    /// ```
    pub fn from_model_name(model_name: &str) -> Result<Self, SemanticSearchError> {
        let model = crate::vector::parse_embedding_model(model_name)
            .map_err(|e| SemanticSearchError::ModelInitError(format!("Invalid model name: {e}")))?;
        Self::with_model(model)
    }

    /// Create with a specific model enum.
    pub fn with_model(model: EmbeddingModel) -> Result<Self, SemanticSearchError> {
        let cache_dir = crate::init::models_dir();
        let model_name = crate::vector::model_to_string(&model);

        // Check if models directory has any content (indicating cached models)
        let has_cached_models = cache_dir.exists()
            && cache_dir
                .read_dir()
                .is_ok_and(|mut entries| entries.any(|_| true));

        // Inform user what's happening
        if has_cached_models {
            eprintln!("Loading embedding model '{model_name}' from cache...");
        } else {
            eprintln!("Downloading embedding model '{model_name}' (first time only)...");
        }

        let mut text_model = TextEmbedding::try_new(
            InitOptions::new(model)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(true), // Always show progress, but with context from message above
        )
        .map_err(|e| {
            SemanticSearchError::ModelInitError(format!(
                "Failed to initialize model '{model_name}': {e}"
            ))
        })?;

        // Get dimensions by generating a test embedding
        let test_embedding = text_model
            .embed(vec!["test"], None)
            .map_err(|e| SemanticSearchError::EmbeddingError(e.to_string()))?;
        let dimensions = test_embedding
            .into_iter()
            .next()
            .ok_or_else(|| SemanticSearchError::EmbeddingError("empty model response".into()))?
            .len();

        // Create initial metadata
        let mut metadata = crate::semantic::SemanticMetadata::new(
            model_name.clone(),
            dimensions,
            0, // No embeddings yet
        );
        let input_budget = crate::embedding_input::InputBudget::local(&text_model.tokenizer, None)
            .map_err(SemanticSearchError::ModelInitError)?;
        let identity = crate::embedding_input::backend_identity(
            "local",
            &model_name,
            None,
            None,
            &input_budget,
        );
        metadata.embedding_identity = Some(identity.clone());

        Ok(Self {
            embeddings: Arc::new(HashMap::new()),
            symbol_segments: Arc::new(HashMap::new()),
            embedding_magnitudes: Arc::new(HashMap::new()),
            symbol_languages: Arc::new(HashMap::new()),
            language_symbols: Arc::new(HashMap::new()),
            embedding_cache: crate::embedding_cache::EmbeddingCache::empty(identity, dimensions),
            model: Some(Arc::new(Mutex::new(text_model))),
            input_budget: Some(input_budget),
            dimensions,
            metadata: Some(metadata),
            persistence: Arc::new(Mutex::new(super::journal::Persistence::default())),
            persist_io: Arc::new(Mutex::new(())),
        })
    }

    /// Index a documentation comment for a symbol
    pub fn index_doc_comment(
        &mut self,
        symbol_id: SymbolId,
        doc: &str,
    ) -> Result<(), SemanticSearchError> {
        // Skip empty docs
        if doc.trim().is_empty() {
            return Ok(());
        }

        if let Some(budget) = &self.input_budget {
            budget
                .validate([doc])
                .map_err(SemanticSearchError::EmbeddingError)?;
        }

        if let Some(embedding) = self.embedding_cache.get(doc) {
            self.insert_embedding(symbol_id, embedding, None);
            return Ok(());
        }

        // Generate embedding — only available in local-model mode
        let model = self.model.as_ref().ok_or_else(|| {
            SemanticSearchError::ModelInitError(
                "No local model — use EmbeddingBackend to generate embeddings in remote mode"
                    .to_string(),
            )
        })?;
        let embeddings = model
            .lock()
            .map_err(|_| SemanticSearchError::EmbeddingError("query model lock poisoned".into()))?
            .embed(vec![doc], None)
            .map_err(|e| SemanticSearchError::EmbeddingError(e.to_string()))?;

        let embedding = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| SemanticSearchError::EmbeddingError("empty model response".into()))?;

        // Validate dimensions
        if embedding.len() != self.dimensions {
            return Err(SemanticSearchError::EmbeddingError(format!(
                "Embedding dimension mismatch: expected {}, got {}",
                self.dimensions,
                embedding.len()
            )));
        }

        let embedding: Arc<[f32]> = Arc::from(embedding);
        self.embedding_cache.insert(doc, Arc::clone(&embedding));
        self.insert_embedding(symbol_id, embedding, None);
        Ok(())
    }

    /// Index a documentation comment for a symbol with language information
    pub fn index_doc_comment_with_language(
        &mut self,
        symbol_id: SymbolId,
        doc: &str,
        language: &str,
    ) -> Result<(), SemanticSearchError> {
        // First index the doc comment normally
        self.index_doc_comment(symbol_id, doc)?;

        // Then store the language mapping
        if self.embeddings.contains_key(&symbol_id) {
            self.set_language(symbol_id, language.to_string());
            self.mark_dirty(symbol_id);
        }

        Ok(())
    }

    /// Store pre-generated embeddings produced by an `EmbeddingBackend`.
    pub fn store_embeddings(&mut self, items: Vec<(SymbolId, Vec<f32>, String)>) -> usize {
        let mut count = 0;
        let mut dropped = 0usize;
        for (symbol_id, embedding, language) in items {
            if embedding.len() == self.dimensions && embedding.iter().all(|x| x.is_finite()) {
                self.insert_embedding(symbol_id, Arc::from(embedding), Some(language));
                count += 1;
            } else {
                dropped += 1;
            }
        }
        if dropped > 0 {
            // This typically means the backend dimension changed without a --force re-index.
            tracing::warn!(
                target: "semantic",
                "store_embeddings dropped {dropped} embeddings due to dimension mismatch \
                 (index={}, received=?). Re-index with --force to fix.",
                self.dimensions
            );
        }
        count
    }

    /// Restore exact inputs from the bounded cache and return the uncached subset.
    pub(crate) fn reuse_cached_embeddings<'a>(
        &mut self,
        items: &[(SymbolId, &'a str, &'a str)],
    ) -> Vec<(SymbolId, &'a str, &'a str)> {
        let mut missing = Vec::new();
        for &(id, text, language) in items {
            if let Some(embedding) = self.embedding_cache.get(text) {
                self.insert_embedding(id, embedding, Some(language.to_string()));
            } else {
                missing.push((id, text, language));
            }
        }
        missing
    }

    /// Store generated vectors and remember the exact inputs that produced them.
    #[cfg(test)]
    pub(crate) fn store_embeddings_with_inputs(
        &mut self,
        items: Vec<(SymbolId, Vec<f32>, String)>,
        inputs: &[(SymbolId, &str, &str)],
    ) -> usize {
        let by_id: HashMap<SymbolId, &str> =
            inputs.iter().map(|(id, text, _)| (*id, *text)).collect();
        let mut count = 0;
        let mut dropped = 0usize;
        for (id, embedding, language) in items {
            if embedding.len() == self.dimensions && embedding.iter().all(|value| value.is_finite())
            {
                let embedding: Arc<[f32]> = Arc::from(embedding);
                if let Some(input) = by_id.get(&id) {
                    self.embedding_cache.insert(input, Arc::clone(&embedding));
                }
                self.insert_embedding(id, embedding, Some(language));
                count += 1;
            } else {
                dropped += 1;
            }
        }
        if dropped > 0 {
            tracing::warn!(
                target: "semantic",
                "store_embeddings dropped {dropped} embeddings due to dimension mismatch \
                 (index={}, received=?). Re-index with --force to fix.",
                self.dimensions
            );
        }
        count
    }

    /// Store one generated vector for every symbol with the same exact input.
    ///
    /// The embedding backend should see a content input once per collector batch;
    /// symbol/language fan-out happens here using shared Arc storage.
    pub(crate) fn store_shared_embedding(
        &mut self,
        input: &str,
        embedding: Vec<f32>,
        targets: &[(SymbolId, &str)],
    ) -> usize {
        if embedding.len() != self.dimensions || !embedding.iter().all(|value| value.is_finite()) {
            tracing::warn!(
                target: "semantic",
                "shared embedding dropped due to dimension or finite-value mismatch \
                 (index={}, received={})",
                self.dimensions,
                embedding.len()
            );
            return 0;
        }

        let embedding: Arc<[f32]> = Arc::from(embedding);
        self.embedding_cache.insert(input, Arc::clone(&embedding));
        for &(id, language) in targets {
            self.insert_embedding(id, Arc::clone(&embedding), Some(language.to_string()));
        }
        targets.len()
    }

    pub(crate) fn cached_symbol_input(&mut self, input: &str) -> Option<Arc<[f32]>> {
        self.embedding_cache.get(input)
    }

    /// Validate the entire group before mutating the parent. A failed group cannot
    /// leave only its first few segments indexed. Cache identity is already bound.
    pub(crate) fn store_symbol_segments(
        &mut self,
        id: SymbolId,
        mut segments: Vec<SymbolSegment>,
        inputs: &[crate::symbol_representation::SymbolInput],
        language: &str,
    ) -> Result<(), SemanticSearchError> {
        if segments.is_empty()
            || segments.len() > crate::symbol_representation::MAX_SYMBOL_SEGMENTS
            || segments.len() != inputs.len()
        {
            return Err(super::journal::error("invalid symbol segment group size"));
        }
        for segment in &mut segments {
            segment.validate(self.dimensions)?;
        }
        self.insert_embedding(id, Arc::clone(&segments[0].vector), Some(language.into()));
        for (segment, input) in segments.iter().zip(inputs) {
            self.embedding_cache
                .insert(&input.text, Arc::clone(&segment.vector));
        }
        Arc::make_mut(&mut self.symbol_segments).insert(id, Arc::from(segments));
        Ok(())
    }

    /// Physical vector count, distinct from the number of represented parents.
    pub fn vector_count(&self) -> usize {
        self.embeddings.len()
            + self
                .symbol_segments
                .values()
                .map(|parts| parts.len().saturating_sub(1))
                .sum::<usize>()
    }

    /// Segment maxima are ranking evidence, not probabilities. Aggregate before
    /// top-k so a large implementation cannot occupy several result slots.
    fn score_symbol(&self, id: SymbolId, query: &[f32], query_magnitude: f32) -> Option<f32> {
        if let Some(segments) = self.symbol_segments.get(&id) {
            return segments
                .iter()
                .map(|segment| {
                    cosine_similarity_with_magnitudes(
                        query,
                        query_magnitude,
                        &segment.vector,
                        segment.magnitude,
                    )
                })
                .max_by(f32::total_cmp);
        }
        let vector = self.embeddings.get(&id)?;
        let magnitude = self.embedding_magnitudes.get(&id).copied()?;
        Some(cosine_similarity_with_magnitudes(
            query,
            query_magnitude,
            vector,
            magnitude,
        ))
    }

    fn insert_embedding(
        &mut self,
        symbol_id: SymbolId,
        embedding: Arc<[f32]>,
        language: Option<String>,
    ) {
        let magnitude = vector_magnitude(&embedding);
        Arc::make_mut(&mut self.symbol_segments).remove(&symbol_id);
        Arc::make_mut(&mut self.embeddings).insert(symbol_id, embedding);
        Arc::make_mut(&mut self.embedding_magnitudes).insert(symbol_id, magnitude);
        if let Some(language) = language {
            self.set_language(symbol_id, language);
        }
        self.mark_dirty(symbol_id);
    }

    fn set_language(&mut self, symbol_id: SymbolId, language: String) {
        let old_language =
            Arc::make_mut(&mut self.symbol_languages).insert(symbol_id, language.clone());
        let language_symbols = Arc::make_mut(&mut self.language_symbols);
        if let Some(old_language) = old_language.filter(|old| old != &language) {
            if let Some(ids) = language_symbols.get_mut(&old_language) {
                Arc::make_mut(ids).remove(&symbol_id);
                if ids.is_empty() {
                    language_symbols.remove(&old_language);
                }
            }
        }
        Arc::make_mut(
            language_symbols
                .entry(language)
                .or_insert_with(|| Arc::new(HashSet::new())),
        )
        .insert(symbol_id);
    }

    /// Search using a pre-computed query embedding vector.
    ///
    /// Use this in remote-embedding mode where the caller obtains the query
    /// vector via `EmbeddingBackend::embed_one` before calling this method.
    /// Returns symbol IDs with their similarity scores, sorted by score descending.
    pub fn search_with_embedding(
        &self,
        query_embedding: &[f32],
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        if self.embeddings.is_empty() {
            return Err(SemanticSearchError::NoEmbeddings);
        }
        if query_embedding.len() != self.dimensions {
            return Err(SemanticSearchError::EmbeddingError(format!(
                "Query embedding dimension {} does not match index dimension {}",
                query_embedding.len(),
                self.dimensions
            )));
        }
        let query_magnitude = vector_magnitude(query_embedding);
        let mut similarities: Vec<(SymbolId, f32)> = self
            .embeddings
            .keys()
            .filter_map(|id| {
                let sim = self.score_symbol(*id, query_embedding, query_magnitude)?;
                if sim >= threshold {
                    Some((*id, sim))
                } else {
                    None
                }
            })
            .collect();
        retain_top_k(&mut similarities, limit);
        Ok(similarities)
    }

    /// Restrict a pinned query snapshot before scoring and parent-level top-K.
    pub fn search_with_embedding_and_symbols(
        &self,
        query: &[f32],
        limit: usize,
        allowed: &std::collections::HashSet<SymbolId>,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        if query.len() != self.dimensions {
            return Err(SemanticSearchError::EmbeddingError(
                "query dimension mismatch".into(),
            ));
        }
        let magnitude = vector_magnitude(query);
        let mut hits = allowed
            .iter()
            .filter_map(|id| {
                self.score_symbol(*id, query, magnitude)
                    .map(|score| (*id, score))
            })
            .collect();
        retain_top_k(&mut hits, limit);
        Ok(hits)
    }

    /// Search using a pre-computed query vector with optional language pre-filtering.
    ///
    /// Language filtering is applied before similarity ranking so the result slice
    /// respects `limit` after filtering, matching the behaviour of `search_with_language`.
    /// Convenience wrapper: delegates to `search_with_embedding` with the given threshold.
    pub fn search_with_embedding_threshold(
        &self,
        query_embedding: &[f32],
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        self.search_with_embedding(query_embedding, limit, threshold)
    }

    pub fn search_with_embedding_and_language(
        &self,
        query_embedding: &[f32],
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        if self.embeddings.is_empty() {
            return Err(SemanticSearchError::NoEmbeddings);
        }
        if query_embedding.len() != self.dimensions {
            return Err(SemanticSearchError::EmbeddingError(format!(
                "Query embedding dimension {} does not match index dimension {}",
                query_embedding.len(),
                self.dimensions
            )));
        }
        let query_magnitude = vector_magnitude(query_embedding);
        let candidate_ids: Box<dyn Iterator<Item = &SymbolId> + '_> = match language {
            Some(language) => Box::new(
                self.language_symbols
                    .get(language)
                    .into_iter()
                    .flat_map(|ids| ids.iter()),
            ),
            None => Box::new(self.embeddings.keys()),
        };
        let mut similarities: Vec<(SymbolId, f32)> = candidate_ids
            .filter_map(|id| {
                self.score_symbol(*id, query_embedding, query_magnitude)
                    .map(|score| (*id, score))
            })
            .collect();
        retain_top_k(&mut similarities, limit);
        Ok(similarities)
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        if self.embeddings.is_empty() {
            return Err(SemanticSearchError::NoEmbeddings);
        }

        let model = self.model.as_ref().ok_or_else(|| {
            SemanticSearchError::ModelInitError(
                "No local model available — use search_with_embedding() in remote mode".to_string(),
            )
        })?;

        if let Some(budget) = &self.input_budget {
            budget
                .validate([query])
                .map_err(SemanticSearchError::EmbeddingError)?;
        }

        // Generate query embedding
        let query_embeddings = model
            .lock()
            .map_err(|_| SemanticSearchError::EmbeddingError("query model lock poisoned".into()))?
            .embed(vec![query], None)
            .map_err(|e| SemanticSearchError::EmbeddingError(e.to_string()))?;
        let query_embedding = query_embeddings
            .into_iter()
            .next()
            .ok_or_else(|| SemanticSearchError::EmbeddingError("empty model response".into()))?;

        // Calculate similarities
        let query_magnitude = vector_magnitude(&query_embedding);
        let mut similarities: Vec<(SymbolId, f32)> = self
            .embeddings
            .keys()
            .filter_map(|id| {
                self.score_symbol(*id, &query_embedding, query_magnitude)
                    .map(|score| (*id, score))
            })
            .collect();

        retain_top_k(&mut similarities, limit);
        Ok(similarities)
    }

    /// Search for similar documentation with language filtering
    ///
    /// This filters BEFORE computing similarity, ensuring we only compute
    /// similarity for symbols in the requested language.
    pub fn search_with_language(
        &self,
        query: &str,
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        if self.embeddings.is_empty() {
            return Err(SemanticSearchError::NoEmbeddings);
        }

        let model = self.model.as_ref().ok_or_else(|| {
            SemanticSearchError::ModelInitError(
                "No local model available — use search_with_embedding() in remote mode".to_string(),
            )
        })?;

        if let Some(budget) = &self.input_budget {
            budget
                .validate([query])
                .map_err(SemanticSearchError::EmbeddingError)?;
        }

        // Generate query embedding
        let query_embeddings = model
            .lock()
            .map_err(|_| SemanticSearchError::EmbeddingError("query model lock poisoned".into()))?
            .embed(vec![query], None)
            .map_err(|e| SemanticSearchError::EmbeddingError(e.to_string()))?;
        let query_embedding = query_embeddings
            .into_iter()
            .next()
            .ok_or_else(|| SemanticSearchError::EmbeddingError("empty model response".into()))?;

        // Filter and score in one pass to avoid a corpus-sized intermediate.
        let query_magnitude = vector_magnitude(&query_embedding);
        let candidate_ids: Box<dyn Iterator<Item = &SymbolId> + '_> = match language {
            Some(language) => Box::new(
                self.language_symbols
                    .get(language)
                    .into_iter()
                    .flat_map(|ids| ids.iter()),
            ),
            None => Box::new(self.embeddings.keys()),
        };
        let mut similarities: Vec<(SymbolId, f32)> = candidate_ids
            .filter_map(|id| {
                self.score_symbol(*id, &query_embedding, query_magnitude)
                    .map(|score| (*id, score))
            })
            .collect();

        retain_top_k(&mut similarities, limit);
        Ok(similarities)
    }

    /// Search with a similarity threshold.
    ///
    /// Requires a local embedding model. In remote-embedding mode, use the
    /// facade's `semantic_search_docs_with_threshold` which handles backend
    /// dispatch, or call `search_with_embedding` with a pre-computed vector.
    pub fn search_with_threshold(
        &self,
        query: &str,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        let results = self.search(query, limit)?;
        Ok(results
            .into_iter()
            .filter(|(_, score)| *score >= threshold)
            .collect())
    }

    /// Returns true when a local fastembed model is available for query embedding.
    /// Returns false for remote-mode instances that require an external backend.
    pub fn has_local_model(&self) -> bool {
        self.model.is_some()
    }

    /// Output dimension of embeddings stored in this index.
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Returns true when this index was built with a remote embedding backend.
    pub fn is_remote_index(&self) -> bool {
        self.metadata.as_ref().is_some_and(|m| m.is_remote())
    }

    /// Get the number of indexed embeddings.
    pub fn embedding_count(&self) -> usize {
        self.embeddings.len()
    }

    /// Snapshot the symbol IDs that currently have semantic vectors.
    ///
    /// This is diagnostic metadata only; callers must not treat vector presence
    /// as relevance or freshness evidence.
    pub(crate) fn embedding_ids(&self) -> Vec<SymbolId> {
        self.embeddings.keys().copied().collect()
    }

    /// Clear all embeddings
    pub fn clear(&mut self) {
        for id in self.embeddings.keys().copied().collect::<Vec<_>>() {
            self.mark_dirty(id);
        }
        Arc::make_mut(&mut self.embeddings).clear();
        Arc::make_mut(&mut self.symbol_segments).clear();
        Arc::make_mut(&mut self.embedding_magnitudes).clear();
        Arc::make_mut(&mut self.symbol_languages).clear();
        Arc::make_mut(&mut self.language_symbols).clear();
    }

    /// Remove embeddings for specific symbols
    ///
    /// This is used when re-indexing files to remove embeddings for symbols
    /// that no longer exist.
    pub fn remove_embeddings(&mut self, symbol_ids: &[SymbolId]) {
        for id in symbol_ids {
            if Arc::make_mut(&mut self.embeddings).remove(id).is_some() {
                self.mark_dirty(*id);
            }
            Arc::make_mut(&mut self.embedding_magnitudes).remove(id);
            Arc::make_mut(&mut self.symbol_segments).remove(id);
            if let Some(language) = Arc::make_mut(&mut self.symbol_languages).remove(id) {
                let language_symbols = Arc::make_mut(&mut self.language_symbols);
                if let Some(ids) = language_symbols.get_mut(&language) {
                    Arc::make_mut(ids).remove(id);
                    if ids.is_empty() {
                        language_symbols.remove(&language);
                    }
                }
            }
        }
    }

    /// Get the metadata if available
    pub fn metadata(&self) -> Option<&crate::semantic::SemanticMetadata> {
        self.metadata.as_ref()
    }

    /// Bind an empty index to the actual backend before producing any vectors.
    /// Existing vectors can only retain their recorded identity.
    pub(crate) fn set_embedding_identity(
        &mut self,
        identity: String,
    ) -> Result<(), SemanticSearchError> {
        if !self.embeddings.is_empty() {
            return self.validate_embedding_identity(&identity);
        }
        // An empty vector generation can still hold compatible cached inputs
        // (fresh rebuild or the last indexed file was removed). Revalidation
        // must not discard those vectors before the first new file batch.
        if self
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.embedding_identity.as_deref())
            == Some(identity.as_str())
        {
            return Ok(());
        }
        if let Some(metadata) = &mut self.metadata {
            metadata.embedding_identity = Some(identity.clone());
        }
        self.embedding_cache =
            crate::embedding_cache::EmbeddingCache::empty(identity, self.dimensions);
        Ok(())
    }

    pub(crate) fn validate_embedding_identity(
        &self,
        identity: &str,
    ) -> Result<(), SemanticSearchError> {
        if self.embeddings.is_empty()
            || self
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.embedding_identity.as_deref())
                == Some(identity)
        {
            return Ok(());
        }
        Err(SemanticSearchError::StorageError {
            message: "Semantic embedding backend, model revision or input policy differs from the indexed vectors (or the index has no recorded identity)".into(),
            suggestion: "Re-index with codanna index <path> --force before semantic querying; equal vector dimensions do not establish model compatibility".into(),
        })
    }

    fn mark_dirty(&mut self, id: SymbolId) {
        // This lock protects only dirty bookkeeping; storage I/O uses a separate lock.
        let mut state = self
            .persistence
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.dirty.insert(id);
        state.revision = state.revision.wrapping_add(1);
    }

    /// Read-only, copy-on-write snapshot. Creating it never generates an embedding,
    /// scans vectors or reads files. Vector payloads are shared, not cloned.
    pub(crate) fn query_snapshot(&self) -> SemanticQuery {
        SemanticQuery(Self {
            embeddings: Arc::clone(&self.embeddings),
            symbol_segments: Arc::clone(&self.symbol_segments),
            embedding_magnitudes: Arc::clone(&self.embedding_magnitudes),
            symbol_languages: Arc::clone(&self.symbol_languages),
            language_symbols: Arc::clone(&self.language_symbols),
            embedding_cache: crate::embedding_cache::EmbeddingCache::empty(
                self.metadata
                    .as_ref()
                    .map_or("unknown", |metadata| metadata.model_name.as_str()),
                self.dimensions,
            ),
            model: self.model.clone(),
            input_budget: self.input_budget.clone(),
            dimensions: self.dimensions,
            metadata: self.metadata.clone(),
            persistence: Arc::new(Mutex::new(super::journal::Persistence::default())),
            persist_io: Arc::new(Mutex::new(())),
        })
    }

    /// Prepare under the owner's short lock, then execute outside that lock.
    pub(crate) fn save_snapshot(&self) -> Result<SemanticSave, SemanticSearchError> {
        let state = self
            .persistence
            .lock()
            .map_err(|_| super::journal::error("semantic persistence tracker poisoned"))?
            .clone();
        let mut metadata = self.metadata.clone().unwrap_or_else(|| {
            crate::semantic::SemanticMetadata::new_remote("unknown".into(), self.dimensions, 0)
        });
        metadata.update(self.embeddings.len());
        metadata.segment_embedding_count =
            self.vector_count().saturating_sub(self.embeddings.len());
        Ok(SemanticSave {
            embeddings: Arc::clone(&self.embeddings),
            symbol_segments: Arc::clone(&self.symbol_segments),
            languages: Arc::clone(&self.symbol_languages),
            metadata,
            state,
            tracker: Arc::clone(&self.persistence),
            io: Arc::clone(&self.persist_io),
            embedding_cache: self.embedding_cache.clone(),
        })
    }

    /// Publish dirty IDs using one atomic manifest; unchanged saves are no-ops.
    /// Every 64 delta commits a checkpoint compacts the journal. Format 1 loads;
    /// the first changed save upgrades to format 2, which older binaries reject.
    pub fn save(&self, path: &Path) -> Result<(), SemanticSearchError> {
        self.save_snapshot()?.save(path)
    }

    /// Create an empty semantic search instance for remote-embedding mode.
    ///
    /// `model_name` should identify the remote model (e.g. "bge-large-en-v1.5")
    /// so it is preserved in saved metadata and visible in status output.
    /// No local fastembed model is loaded. Queries must use `search_with_embedding`.
    pub fn new_empty(dimensions: usize, model_name: &str) -> Self {
        let metadata =
            crate::semantic::SemanticMetadata::new_remote(model_name.to_string(), dimensions, 0);
        Self {
            embeddings: Arc::new(HashMap::new()),
            symbol_segments: Arc::new(HashMap::new()),
            embedding_magnitudes: Arc::new(HashMap::new()),
            symbol_languages: Arc::new(HashMap::new()),
            language_symbols: Arc::new(HashMap::new()),
            embedding_cache: crate::embedding_cache::EmbeddingCache::empty(model_name, dimensions),
            model: None,
            input_budget: None,
            dimensions,
            metadata: Some(metadata),
            persistence: Arc::new(Mutex::new(super::journal::Persistence::default())),
            persist_io: Arc::new(Mutex::new(())),
        }
    }

    /// Create an empty local semantic index without owning an embedding model.
    /// The facade's shared embedding backend performs both indexing and query
    /// inference, avoiding a duplicate native model session.
    pub fn new_empty_local(dimensions: usize, model_name: &str) -> Self {
        let metadata =
            crate::semantic::SemanticMetadata::new(model_name.to_string(), dimensions, 0);
        Self {
            embeddings: Arc::new(HashMap::new()),
            symbol_segments: Arc::new(HashMap::new()),
            embedding_magnitudes: Arc::new(HashMap::new()),
            symbol_languages: Arc::new(HashMap::new()),
            language_symbols: Arc::new(HashMap::new()),
            embedding_cache: crate::embedding_cache::EmbeddingCache::empty(model_name, dimensions),
            model: None,
            input_budget: None,
            dimensions,
            metadata: Some(metadata),
            persistence: Arc::new(Mutex::new(super::journal::Persistence::default())),
            persist_io: Arc::new(Mutex::new(())),
        }
    }

    /// Load symbol-to-language mappings from `languages.json`.
    pub(super) fn load_symbol_languages(
        path: &Path,
    ) -> Result<HashMap<SymbolId, String>, SemanticSearchError> {
        let languages_path = path.join("languages.json");
        if !languages_path.exists() {
            return Ok(HashMap::new());
        }
        let languages_json = std::fs::read_to_string(&languages_path).map_err(|e| {
            SemanticSearchError::StorageError {
                message: format!("Failed to read language mappings: {e}"),
                suggestion: "Language mappings file may be corrupted".to_string(),
            }
        })?;
        let languages_map: HashMap<u32, String> =
            serde_json::from_str(&languages_json).map_err(|e| {
                SemanticSearchError::StorageError {
                    message: format!("Failed to parse language mappings: {e}"),
                    suggestion: "Try rebuilding the semantic index".to_string(),
                }
            })?;
        Ok(languages_map
            .into_iter()
            .filter_map(|(id, lang)| SymbolId::new(id).map(|sid| (sid, lang)))
            .collect())
    }

    /// Load an existing semantic index without initialising a local embedding model.
    ///
    /// Used in remote-embedding mode: stored vectors are loaded for similarity
    /// search but query embedding is handled externally via `search_with_embedding`.
    pub fn load_remote(path: &Path) -> Result<Self, SemanticSearchError> {
        let snapshot = super::journal::load(path)?;
        let embeddings: HashMap<SymbolId, Arc<[f32]>> = snapshot
            .embeddings
            .into_iter()
            .map(|(id, vector)| (id, Arc::from(vector)))
            .collect();
        let embedding_magnitudes = embeddings
            .iter()
            .map(|(id, vector)| (*id, vector_magnitude(vector)))
            .collect();
        let language_symbols = build_language_symbols(&snapshot.languages);
        let embedding_cache = crate::embedding_cache::EmbeddingCache::load(
            &path.join("embedding-cache.json"),
            snapshot
                .metadata
                .embedding_identity
                .as_deref()
                .unwrap_or(&snapshot.metadata.model_name),
            snapshot.metadata.dimension,
        );
        Ok(Self {
            embeddings: Arc::new(embeddings),
            symbol_segments: Arc::new(snapshot.symbol_segments),
            embedding_magnitudes: Arc::new(embedding_magnitudes),
            symbol_languages: Arc::new(snapshot.languages),
            language_symbols: Arc::new(language_symbols),
            embedding_cache,
            dimensions: snapshot.metadata.dimension,
            metadata: Some(snapshot.metadata),
            model: None,
            input_budget: None,
            persistence: Arc::new(Mutex::new(snapshot.persistence)),
            persist_io: Arc::new(Mutex::new(())),
        })
    }

    /// Load stored vectors without constructing a query-time model. The
    /// facade's shared embedding backend supplies query embeddings.
    pub fn load_without_model(path: &Path) -> Result<Self, SemanticSearchError> {
        Self::load_remote(path)
    }

    pub fn load(path: &Path) -> Result<Self, SemanticSearchError> {
        let mut search = Self::load_remote(path)?;
        let metadata = search
            .metadata
            .as_ref()
            .expect("loaded snapshot has metadata");
        if !metadata.is_remote() {
            let model = crate::vector::parse_embedding_model(&metadata.model_name)
                .map_err(|e| SemanticSearchError::ModelInitError(e.to_string()))?;
            let text_model = TextEmbedding::try_new(
                InitOptions::new(model)
                    .with_cache_dir(crate::init::models_dir())
                    .with_show_download_progress(false),
            )
            .map_err(|e| SemanticSearchError::ModelInitError(e.to_string()))?;
            let input_budget =
                crate::embedding_input::InputBudget::local(&text_model.tokenizer, None)
                    .map_err(SemanticSearchError::ModelInitError)?;
            let identity = crate::embedding_input::backend_identity(
                "local",
                &metadata.model_name,
                None,
                None,
                &input_budget,
            );
            let identity = [
                crate::symbol_representation::CodeEmbeddingPolicy::SymbolBodyV1,
                crate::symbol_representation::CodeEmbeddingPolicy::SymbolBodyV2,
            ]
            .into_iter()
            .find(|policy| policy.accepts_recorded_identity(metadata.embedding_identity.as_deref()))
            .map_or_else(
                || identity.clone(),
                |policy| policy.bind_identity(identity.clone()),
            );
            search.validate_embedding_identity(&identity)?;
            search.input_budget = Some(input_budget);
            search.model = Some(Arc::new(Mutex::new(text_model)));
        }
        Ok(search)
    }
}

/// Restricted query surface: snapshots cannot mutate or publish a generation.
pub(crate) struct SemanticQuery(SimpleSemanticSearch);
impl SemanticQuery {
    pub(crate) fn retain_symbols(&mut self, allowed: &std::collections::HashSet<SymbolId>) {
        Arc::make_mut(&mut self.0.embeddings).retain(|id, _| allowed.contains(id));
        Arc::make_mut(&mut self.0.language_symbols)
            .values_mut()
            .for_each(|ids| Arc::make_mut(ids).retain(|id| allowed.contains(id)));
    }
    pub(crate) fn embedding_ids(&self) -> Vec<SymbolId> {
        self.0.embedding_ids()
    }
    pub(crate) fn has_local_model(&self) -> bool {
        self.0.has_local_model()
    }
    pub(crate) fn search_with_language(
        &self,
        query: &str,
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        self.0.search_with_language(query, limit, language)
    }
    pub(crate) fn search_with_embedding_and_language(
        &self,
        query: &[f32],
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<(SymbolId, f32)>, SemanticSearchError> {
        self.0
            .search_with_embedding_and_language(query, limit, language)
    }
}

pub(crate) struct SemanticSave {
    embeddings: Arc<HashMap<SymbolId, Arc<[f32]>>>,
    symbol_segments: Arc<SymbolSegments>,
    languages: Arc<HashMap<SymbolId, String>>,
    metadata: super::SemanticMetadata,
    state: super::journal::Persistence,
    tracker: Arc<Mutex<super::journal::Persistence>>,
    io: Arc<Mutex<()>>,
    embedding_cache: crate::embedding_cache::EmbeddingCache,
}
impl SemanticSave {
    pub(crate) fn save(mut self, path: &Path) -> Result<(), SemanticSearchError> {
        let _io = self
            .io
            .lock()
            .map_err(|_| super::journal::error("semantic save lane poisoned"))?;
        let revision = self.state.revision;
        {
            let current = self
                .tracker
                .lock()
                .map_err(|_| super::journal::error("semantic persistence tracker poisoned"))?;
            if current
                .committed_revision
                .is_some_and(|committed| committed > revision)
            {
                return Err(super::journal::error(
                    "stale prepared semantic save; a newer snapshot is already committed",
                ));
            }
            // Another prepared snapshot may have committed while this one waited
            // on the I/O lane. Compare against that owner-known manifest, not an
            // obsolete one captured before waiting. External writers still fail
            // the on-disk compare-and-swap inside journal::save.
            self.state.saved = current.saved.clone();
            if current.committed_revision == Some(revision) {
                self.state.dirty.clear();
            }
        }
        super::journal::save(
            path,
            &mut self.state,
            &self.embeddings,
            &self.symbol_segments,
            &self.languages,
            self.metadata,
        )?;
        if let Err(error) = self
            .embedding_cache
            .save(&path.join("embedding-cache.json"))
        {
            tracing::warn!(target: "embedding_cache", %error, "failed to persist semantic embedding cache");
        }
        let mut current = self
            .tracker
            .lock()
            .map_err(|_| super::journal::error("semantic persistence tracker poisoned"))?;
        self.state.committed_revision = Some(revision);
        if current.revision == revision {
            *current = self.state;
        } else {
            // Preserve every later dirty ID, even when the same ID changed twice.
            current.saved = self.state.saved;
            current.committed_revision = Some(revision);
        }
        Ok(())
    }
}

/// Retains only the highest-scoring results without fully sorting the entire corpus.
///
/// Semantic search commonly asks for a small `limit` from a large embedding set.
/// Partitioning first keeps selection O(n), then only the retained prefix is sorted.
fn retain_top_k(similarities: &mut Vec<(SymbolId, f32)>, limit: usize) {
    if limit == 0 {
        similarities.clear();
        return;
    }

    if similarities.len() > limit {
        similarities.select_nth_unstable_by(limit, |a, b| b.1.total_cmp(&a.1));
        similarities.truncate(limit);
    }

    similarities.sort_by(|a, b| b.1.total_cmp(&a.1));
}

fn build_language_symbols(
    languages: &HashMap<SymbolId, String>,
) -> HashMap<String, Arc<HashSet<SymbolId>>> {
    let mut grouped: HashMap<String, HashSet<SymbolId>> = HashMap::new();
    for (id, language) in languages {
        grouped.entry(language.clone()).or_default().insert(*id);
    }
    grouped
        .into_iter()
        .map(|(language, ids)| (language, Arc::new(ids)))
        .collect()
}

/// Calculate cosine similarity between two vectors.
///
/// Semantic ranking must never emit a non-orderable score. Malformed direct
/// callers or legacy in-memory vectors containing NaN/±infinity therefore
/// degrade to zero similarity instead of propagating NaN into sorting.
#[cfg(test)]
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    cosine_similarity_with_magnitudes(a, vector_magnitude(a), b, vector_magnitude(b))
}

fn vector_magnitude(vector: &[f32]) -> f32 {
    vector.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn cosine_similarity_with_magnitudes(
    a: &[f32],
    magnitude_a: f32,
    b: &[f32],
    magnitude_b: f32,
) -> f32 {
    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();

    if !dot_product.is_finite()
        || !magnitude_a.is_finite()
        || !magnitude_b.is_finite()
        || magnitude_a == 0.0
        || magnitude_b == 0.0
    {
        return 0.0;
    }

    let score = dot_product / (magnitude_a * magnitude_b);
    if score.is_finite() { score } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_identity_survives_reopen_and_rejects_equal_dimension_changes() {
        let temp = tempfile::tempdir().unwrap();
        let mut semantic = SimpleSemanticSearch::new_empty(2, "unchanged-alias");
        semantic
            .set_embedding_identity("revision-1:complete-input-v2".into())
            .unwrap();
        semantic.store_embeddings(vec![(
            SymbolId::new(1).unwrap(),
            vec![1.0, 0.0],
            "rust".into(),
        )]);
        semantic.save(temp.path()).unwrap();
        let reopened = SimpleSemanticSearch::load_without_model(temp.path()).unwrap();
        reopened
            .validate_embedding_identity("revision-1:complete-input-v2")
            .unwrap();
        for changed in ["revision-2:complete-input-v2", "revision-1:old-input"] {
            let error = reopened
                .validate_embedding_identity(changed)
                .unwrap_err()
                .to_string();
            assert!(error.contains("Re-index"), "{error}");
        }
        assert_eq!(reopened.dimensions(), 2);
    }

    #[test]
    fn legacy_semantic_vectors_require_identity_before_inference() {
        let mut semantic = SimpleSemanticSearch::new_empty(2, "fixture");
        semantic.store_embeddings(vec![(
            SymbolId::new(1).unwrap(),
            vec![1.0, 0.0],
            "rust".into(),
        )]);
        assert!(
            semantic
                .validate_embedding_identity("current-policy")
                .is_err()
        );
        assert!(
            semantic
                .set_embedding_identity("current-policy".into())
                .is_err()
        );
    }

    /// Two savers on one semantic directory (the serve co-run shape:
    /// two --watch processes on one workspace) must not destroy each
    /// other's in-flight staging: every save succeeds and the promoted
    /// generation is a complete one.
    #[test]
    fn concurrent_saves_do_not_destroy_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

        let mut handles = Vec::new();
        for seed in 1..=2u32 {
            let path = path.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut search = SimpleSemanticSearch::new_empty(8, "test-remote");
                let items: Vec<(SymbolId, Vec<f32>, String)> = (1..=16u32)
                    .map(|i| {
                        let id = SymbolId::new(seed * 100 + i).unwrap();
                        let v: Vec<f32> = (0..8).map(|d| (seed * 100 + i + d) as f32).collect();
                        (id, v, "rust".to_string())
                    })
                    .collect();
                search.store_embeddings(items);

                barrier.wait();
                let mut errors = Vec::new();
                for _ in 0..50 {
                    if let Err(e) = search.save(&path) {
                        errors.push(e.to_string());
                    }
                }
                errors
            }));
        }

        let mut all_errors = Vec::new();
        for h in handles {
            all_errors.extend(h.join().expect("saver thread panicked"));
        }
        assert!(
            all_errors.is_empty(),
            "concurrent saves must not destroy each other's staging:\n{all_errors:#?}"
        );

        let loaded = SimpleSemanticSearch::load(&path).unwrap();
        assert_eq!(
            loaded.embedding_count(),
            16,
            "the promoted generation is complete"
        );
    }

    #[test]
    #[ignore = "Downloads 86MB model - run with --ignored for semantic tests"]
    fn test_save_survives_stale_staging_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut search = SimpleSemanticSearch::new().unwrap();
        search
            .index_doc_comment(SymbolId::new(1).unwrap(), "Parse JSON data")
            .unwrap();
        search.save(dir.path()).unwrap();

        // Simulated crash: staging dir left behind by an interrupted save
        let staging = dir.path().join(".staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("segment_0.vec"), b"garbage from a dead save").unwrap();

        // Old generation still loads
        let loaded = SimpleSemanticSearch::load(dir.path()).unwrap();
        assert_eq!(loaded.embedding_count(), 1);

        // Next save ignores legacy staging and commits a new generation
        search
            .index_doc_comment(SymbolId::new(2).unwrap(), "Connect to database")
            .unwrap();
        search.save(dir.path()).unwrap();
        assert!(
            staging.exists(),
            "legacy staging is not owned by the new journal"
        );
        let reloaded = SimpleSemanticSearch::load(dir.path()).unwrap();
        assert_eq!(reloaded.embedding_count(), 2);
    }

    #[test]
    #[ignore = "Downloads 86MB model - run with --ignored for semantic tests"]
    fn test_remove_embeddings() {
        let mut search = SimpleSemanticSearch::new().unwrap();

        // Add some embeddings with distinct content
        let id1 = SymbolId::new(1).unwrap();
        let id2 = SymbolId::new(2).unwrap();
        let id3 = SymbolId::new(3).unwrap();

        search
            .index_doc_comment(id1, "Parse JSON data from file")
            .unwrap();
        search
            .index_doc_comment(id2, "Connect to database server")
            .unwrap();
        search
            .index_doc_comment(id3, "Calculate hash of string")
            .unwrap();

        assert_eq!(search.embedding_count(), 3);

        // Remove specific embeddings
        search.remove_embeddings(&[id1, id3]);

        assert_eq!(search.embedding_count(), 1);

        // Verify correct embedding was kept - search for database content
        let results = search.search("database connection", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id2);

        // Verify we can't find removed content with good similarity
        let json_results = search.search_with_threshold("parse JSON", 10, 0.6).unwrap();
        assert!(
            json_results.is_empty(),
            "Should not find removed JSON parsing doc"
        );

        let hash_results = search
            .search_with_threshold("calculate hash", 10, 0.6)
            .unwrap();
        assert!(
            hash_results.is_empty(),
            "Should not find removed hash calculation doc"
        );
    }

    #[test]
    #[ignore = "Downloads 86MB model - run with --ignored for semantic tests"]
    fn test_save_and_load() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();

        // Create and populate search instance
        // Skip test if model is not available
        let mut search = match SimpleSemanticSearch::new() {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Skipping test: FastEmbed model not available");
                return;
            }
        };

        // Index some test data
        search
            .index_doc_comment(SymbolId::new(1).unwrap(), "This function parses JSON data")
            .unwrap();

        search
            .index_doc_comment(
                SymbolId::new(2).unwrap(),
                "Authenticates a user with credentials",
            )
            .unwrap();

        let original_count = search.embedding_count();

        // Save to disk
        search.save(temp_dir.path()).unwrap();

        // Load from disk
        let loaded = SimpleSemanticSearch::load(temp_dir.path()).unwrap();

        // Verify same number of embeddings
        assert_eq!(loaded.embedding_count(), original_count);

        // Verify search still works
        let results = loaded.search("parse JSON", 10).unwrap();
        assert!(!results.is_empty());

        // The first result should be our JSON parsing function
        assert_eq!(results[0].0, SymbolId::new(1).unwrap());
    }

    #[test]
    fn test_load_missing_file() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();

        // Try to load from non-existent path
        let result = SimpleSemanticSearch::load(temp_dir.path());

        assert!(result.is_err());
        match result.unwrap_err() {
            SemanticSearchError::StorageError { .. } => {}
            _ => panic!("Expected StorageError"),
        }
    }

    #[test]
    #[ignore = "Downloads 86MB model - run with --ignored for semantic tests"]
    fn test_semantic_search_basic() {
        // Skip test if model is not available
        let mut search = match SimpleSemanticSearch::new() {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Skipping test: FastEmbed model not available");
                return;
            }
        };

        // Index some doc comments
        let id1 = SymbolId::new(1).unwrap();
        let id2 = SymbolId::new(2).unwrap();
        let id3 = SymbolId::new(3).unwrap();

        search
            .index_doc_comment(id1, "Parse JSON data from a string")
            .unwrap();
        search
            .index_doc_comment(id2, "Serialize data structure to JSON")
            .unwrap();
        search
            .index_doc_comment(id3, "Calculate factorial of a number")
            .unwrap();

        // Search for JSON-related functions
        let results = search.search("parse JSON", 3).unwrap();

        // First two should be JSON-related
        assert!(results[0].1 > 0.7); // High similarity
        assert!(results[1].1 > 0.5); // Moderate similarity
        assert!(results[2].1 < 0.3); // Low similarity (factorial)

        // The parse function should be most similar
        assert_eq!(results[0].0, id1);
    }

    #[test]
    #[ignore = "Downloads 86MB model - run with --ignored for semantic tests"]
    fn test_similarity_threshold() {
        // Skip test if model is not available
        let mut search = match SimpleSemanticSearch::new() {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Skipping test: FastEmbed model not available");
                return;
            }
        };

        // Index test data
        search
            .index_doc_comment(
                SymbolId::new(1).unwrap(),
                "Authentication and authorization",
            )
            .unwrap();
        search
            .index_doc_comment(SymbolId::new(2).unwrap(), "User login and authentication")
            .unwrap();
        search
            .index_doc_comment(SymbolId::new(3).unwrap(), "Matrix multiplication algorithm")
            .unwrap();

        // Search with threshold
        let results = search
            .search_with_threshold("user authentication", 10, 0.5)
            .unwrap();

        // Should only return auth-related results
        assert_eq!(results.len(), 2);
        for (_, score) in &results {
            assert!(*score >= 0.5);
        }
    }

    #[test]
    fn hardening_top_k_selection_matches_full_sort() {
        let mut results: Vec<(SymbolId, f32)> = (1..=200u32)
            .map(|id| {
                let score = ((id * 37) % 101) as f32 / 100.0;
                (SymbolId::new(id).unwrap(), score)
            })
            .collect();
        let mut expected = results.clone();
        expected.sort_by(|a, b| b.1.total_cmp(&a.1));
        expected.truncate(10);

        retain_top_k(&mut results, 10);

        assert_eq!(results.len(), 10);
        let actual_scores: Vec<_> = results.iter().map(|(_, score)| *score).collect();
        let expected_scores: Vec<_> = expected.iter().map(|(_, score)| *score).collect();
        assert_eq!(actual_scores, expected_scores);
    }

    #[test]
    fn hardening_top_k_zero_limit_returns_no_results() {
        let mut results = vec![(SymbolId::new(1).unwrap(), 0.9)];
        retain_top_k(&mut results, 0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_cosine_similarity() {
        // Identical vectors
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v1, &v2) - 1.0).abs() < 0.001);

        // Orthogonal vectors
        let v3 = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&v1, &v3) - 0.0).abs() < 0.001);

        // Opposite vectors
        let v4 = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v1, &v4) - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn hardening_cosine_similarity_never_returns_nonfinite_score() {
        let finite = [1.0, 0.0];
        for malformed in [
            [f32::NAN, 1.0],
            [f32::INFINITY, 1.0],
            [f32::NEG_INFINITY, 1.0],
        ] {
            let score = cosine_similarity(&finite, &malformed);
            assert!(score.is_finite());
            assert_eq!(score, 0.0);
        }
    }

    #[test]
    fn hardening_semantic_search_handles_nonfinite_direct_query_without_panic() {
        let mut search = SimpleSemanticSearch::new_empty(2, "test-remote");
        search.store_embeddings(vec![(
            SymbolId::new(1).unwrap(),
            vec![1.0, 0.0],
            "rust".to_string(),
        )]);

        let results = search
            .search_with_embedding(&[f32::NAN, 1.0], 10, -1.0)
            .expect("malformed direct query must degrade instead of panicking");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, 0.0);
    }
}

#[cfg(test)]
mod review_borrowed_persistence {
    use super::*;
    #[test]
    fn hardening_review_borrowed_semantic_save_round_trips_vectors_and_languages() {
        let dir = tempfile::tempdir().unwrap();
        let mut search = SimpleSemanticSearch::new_empty(2, "prepared-fixture");
        assert_eq!(
            search.store_embeddings(vec![
                (SymbolId::new(1).unwrap(), vec![1.0, 0.0], "rust".to_owned()),
                (
                    SymbolId::new(2).unwrap(),
                    vec![0.0, 1.0],
                    "typescript".to_owned()
                ),
            ]),
            2
        );
        search.save(dir.path()).unwrap();
        let loaded = SimpleSemanticSearch::load_remote(dir.path()).unwrap();
        assert_eq!(loaded.embeddings, search.embeddings);
        assert_eq!(loaded.symbol_languages, search.symbol_languages);
        search.remove_embeddings(&[SymbolId::new(1).unwrap()]);
        search.save(dir.path()).unwrap();
        let loaded = SimpleSemanticSearch::load_remote(dir.path()).unwrap();
        assert_eq!(loaded.embeddings.len(), 1);
        assert_eq!(loaded.symbol_languages.len(), 1);
        assert!(loaded.embeddings.contains_key(&SymbolId::new(2).unwrap()));
    }

    #[test]
    fn content_cache_reuses_vectors_across_ids_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let first = SymbolId::new(1).unwrap();
        let second = SymbolId::new(2).unwrap();
        let inputs = [(first, "unchanged docs", "rust")];
        let mut search = SimpleSemanticSearch::new_empty(2, "fixture@revision");
        assert_eq!(
            search.store_embeddings_with_inputs(
                vec![(first, vec![3.0, 4.0], "rust".into())],
                &inputs,
            ),
            1
        );
        search.save(dir.path()).unwrap();

        let mut loaded = SimpleSemanticSearch::load_remote(dir.path()).unwrap();
        loaded.remove_embeddings(&[first]);
        let missing = loaded.reuse_cached_embeddings(&[(second, "unchanged docs", "rust")]);
        assert!(missing.is_empty());
        assert_eq!(loaded.embeddings[&second].as_ref(), &[3.0, 4.0]);
        assert_eq!(loaded.embedding_magnitudes[&second], 5.0);
        assert!(loaded.language_symbols["rust"].contains(&second));
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn hardening_final_semantic_query_pins_old_payload_without_copying_vectors() {
        let mut search = SimpleSemanticSearch::new_empty(2, "fixture");
        let id = SymbolId::new(1).unwrap();
        search.store_embeddings(vec![(id, vec![1., 0.], "rust".into())]);
        let query = search.query_snapshot();
        assert!(Arc::ptr_eq(&search.embeddings, &query.0.embeddings));
        let payload = Arc::clone(&search.embeddings[&id]);
        search.store_embeddings(vec![(id, vec![0., 1.], "rust".into())]);
        assert!(Arc::ptr_eq(&payload, &query.0.embeddings[&id]));
        assert_eq!(
            query
                .search_with_embedding_and_language(&[1., 0.], 1, None)
                .unwrap()[0]
                .1,
            1.
        );
        assert_eq!(
            search
                .search_with_embedding_and_language(&[1., 0.], 1, None)
                .unwrap()[0]
                .1,
            0.
        );
    }
    #[test]
    fn hardening_final_save_snapshot_preserves_updates_made_after_preparation() {
        let dir = tempfile::tempdir().unwrap();
        let mut search = SimpleSemanticSearch::new_empty(2, "fixture");
        let id = SymbolId::new(1).unwrap();
        search.store_embeddings(vec![(id, vec![1., 0.], "rust".into())]);
        let pending = search.save_snapshot().unwrap();
        search.store_embeddings(vec![(id, vec![0., 1.], "rust".into())]);
        pending.save(dir.path()).unwrap();
        search.save(dir.path()).unwrap();
        let loaded = SimpleSemanticSearch::load_remote(dir.path()).unwrap();
        assert_eq!(
            loaded
                .search_with_embedding_and_language(&[0., 1.], 1, None)
                .unwrap()[0]
                .1,
            1.
        );
        assert!(search.persistence.lock().unwrap().dirty.is_empty());
    }
    #[test]
    fn hardening_final_older_prepared_save_cannot_erase_newer_commit() {
        let dir = tempfile::tempdir().unwrap();
        let mut search = SimpleSemanticSearch::new_empty(2, "fixture");
        let id = SymbolId::new(1).unwrap();
        search.store_embeddings(vec![(id, vec![1., 0.], "rust".into())]);
        let older = search.save_snapshot().unwrap();
        search.store_embeddings(vec![(id, vec![0., 1.], "rust".into())]);
        let newer = search.save_snapshot().unwrap();
        newer.save(dir.path()).unwrap();
        let committed = std::fs::read(dir.path().join("metadata.json")).unwrap();
        assert!(older.save(dir.path()).is_err());
        search.save(dir.path()).unwrap();
        assert_eq!(
            committed,
            std::fs::read(dir.path().join("metadata.json")).unwrap()
        );
        let loaded = SimpleSemanticSearch::load_remote(dir.path()).unwrap();
        assert_eq!(loaded.embeddings[&id].as_ref(), &[0., 1.]);
    }

    #[test]
    fn hardening_final_prepared_saves_in_order_rebase_without_losing_updates() {
        let dir = tempfile::tempdir().unwrap();
        let mut search = SimpleSemanticSearch::new_empty(2, "fixture");
        let a = SymbolId::new(1).unwrap();
        let b = SymbolId::new(2).unwrap();
        search.store_embeddings(vec![(a, vec![1., 0.], "rust".into())]);
        search.save(dir.path()).unwrap();
        search.store_embeddings(vec![(a, vec![0., 1.], "rust".into())]);
        let first = search.save_snapshot().unwrap();
        search.store_embeddings(vec![(b, vec![1., 0.], "typescript".into())]);
        let second = search.save_snapshot().unwrap();
        first.save(dir.path()).unwrap();
        second.save(dir.path()).unwrap();
        let loaded = SimpleSemanticSearch::load_remote(dir.path()).unwrap();
        assert_eq!(loaded.embeddings.len(), 2);
        assert_eq!(loaded.embeddings[&a].as_ref(), &[0., 1.]);
        assert_eq!(loaded.symbol_languages[&b], "typescript");
        assert!(search.persistence.lock().unwrap().dirty.is_empty());
    }
}
