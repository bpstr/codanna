//! Simple semantic search implementation for documentation comments

use crate::SymbolId;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::collections::HashMap;
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

/// Advanced semantic search engine for documentation analysis
///
/// This implementation uses state-of-the-art embeddings to find
/// semantically similar documentation across the entire codebase,
/// enabling natural language queries for code discovery.
/// Queries can use immutable snapshots while indexing updates a later generation.
pub struct SimpleSemanticSearch {
    /// Embeddings indexed by symbol ID
    embeddings: Arc<HashMap<SymbolId, Arc<[f32]>>>,

    /// Language mapping for each symbol (for language-filtered search)
    symbol_languages: Arc<HashMap<SymbolId, String>>,

    /// The embedding model for query-time embedding (None in remote mode — caller
    /// must use `search_with_embedding` and provide the query vector externally).
    model: Option<Arc<Mutex<TextEmbedding>>>,

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
        let metadata = crate::semantic::SemanticMetadata::new(
            model_name.clone(),
            dimensions,
            0, // No embeddings yet
        );

        Ok(Self {
            embeddings: Arc::new(HashMap::new()),
            symbol_languages: Arc::new(HashMap::new()),
            model: Some(Arc::new(Mutex::new(text_model))),
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

        Arc::make_mut(&mut self.embeddings).insert(symbol_id, Arc::from(embedding));
        self.mark_dirty(symbol_id);
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
            Arc::make_mut(&mut self.symbol_languages).insert(symbol_id, language.to_string());
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
                Arc::make_mut(&mut self.embeddings).insert(symbol_id, Arc::from(embedding));
                Arc::make_mut(&mut self.symbol_languages).insert(symbol_id, language);
                self.mark_dirty(symbol_id);
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
        let mut similarities: Vec<(SymbolId, f32)> = self
            .embeddings
            .iter()
            .filter_map(|(id, emb)| {
                let sim = cosine_similarity(query_embedding, emb);
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
        let candidates: Vec<(&SymbolId, &Arc<[f32]>)> = if let Some(lang) = language {
            self.embeddings
                .iter()
                .filter(|(id, _)| self.symbol_languages.get(id).is_some_and(|l| l == lang))
                .collect()
        } else {
            self.embeddings.iter().collect()
        };
        let mut similarities: Vec<(SymbolId, f32)> = candidates
            .into_iter()
            .map(|(id, emb)| (*id, cosine_similarity(query_embedding, emb)))
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
        let mut similarities: Vec<(SymbolId, f32)> = self
            .embeddings
            .iter()
            .map(|(id, embedding)| {
                let similarity = cosine_similarity(&query_embedding, embedding);
                (*id, similarity)
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

        // Filter embeddings by language BEFORE computing similarity
        let filtered_embeddings: Vec<(&SymbolId, &Arc<[f32]>)> = if let Some(lang) = language {
            self.embeddings
                .iter()
                .filter(|(id, _)| {
                    self.symbol_languages
                        .get(id)
                        .is_some_and(|symbol_lang| symbol_lang == lang)
                })
                .collect()
        } else {
            self.embeddings.iter().collect()
        };

        // Calculate similarities only for filtered embeddings
        let mut similarities: Vec<(SymbolId, f32)> = filtered_embeddings
            .into_iter()
            .map(|(id, embedding)| {
                let similarity = cosine_similarity(&query_embedding, embedding);
                (*id, similarity)
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

    /// Get the number of indexed embeddings
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

    pub fn embedding_count(&self) -> usize {
        self.embeddings.len()
    }

    /// Clear all embeddings
    pub fn clear(&mut self) {
        for id in self.embeddings.keys().copied().collect::<Vec<_>>() {
            self.mark_dirty(id);
        }
        Arc::make_mut(&mut self.embeddings).clear();
        Arc::make_mut(&mut self.symbol_languages).clear();
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
            Arc::make_mut(&mut self.symbol_languages).remove(id);
        }
    }

    /// Get the metadata if available
    pub fn metadata(&self) -> Option<&crate::semantic::SemanticMetadata> {
        self.metadata.as_ref()
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
            symbol_languages: Arc::clone(&self.symbol_languages),
            model: self.model.clone(),
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
        Ok(SemanticSave {
            embeddings: Arc::clone(&self.embeddings),
            languages: Arc::clone(&self.symbol_languages),
            metadata,
            state,
            tracker: Arc::clone(&self.persistence),
            io: Arc::clone(&self.persist_io),
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
            symbol_languages: Arc::new(HashMap::new()),
            model: None,
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
        Ok(Self {
            embeddings: Arc::new(
                snapshot
                    .embeddings
                    .into_iter()
                    .map(|(id, v)| (id, Arc::from(v)))
                    .collect(),
            ),
            symbol_languages: Arc::new(snapshot.languages),
            dimensions: snapshot.metadata.dimension,
            metadata: Some(snapshot.metadata),
            model: None,
            persistence: Arc::new(Mutex::new(snapshot.persistence)),
            persist_io: Arc::new(Mutex::new(())),
        })
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
            search.model = Some(Arc::new(Mutex::new(text_model)));
        }
        Ok(search)
    }
}

/// Restricted query surface: snapshots cannot mutate or publish a generation.
pub(crate) struct SemanticQuery(SimpleSemanticSearch);
impl SemanticQuery {
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
    languages: Arc<HashMap<SymbolId, String>>,
    metadata: super::SemanticMetadata,
    state: super::journal::Persistence,
    tracker: Arc<Mutex<super::journal::Persistence>>,
    io: Arc<Mutex<()>>,
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
            &self.languages,
            self.metadata,
        )?;
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

/// Calculate cosine similarity between two vectors.
///
/// Semantic ranking must never emit a non-orderable score. Malformed direct
/// callers or legacy in-memory vectors containing NaN/±infinity therefore
/// degrade to zero similarity instead of propagating NaN into sorting.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

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
