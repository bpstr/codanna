//! Document storage with tantivy metadata and vector embeddings.
//!
//! This module provides the main storage interface for document chunks,
//! combining tantivy for metadata/filtering with mmap vectors for semantic search.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};

/// Progress updates during document indexing.
#[derive(Debug, Clone)]
pub enum IndexProgress<'a> {
    /// A phase without a measurable completion count.
    Phase { name: &'static str },
    /// Processing a file (chunking, metadata extraction)
    ProcessingFile {
        current: usize,
        total: usize,
        path: &'a Path,
    },
    /// Generating embeddings for chunks
    GeneratingEmbeddings { current: usize, total: usize },
}

/// Default batch size for embedding generation.
/// Smaller batches reduce memory pressure and provide smoother progress.
const EMBEDDING_BATCH_SIZE: usize = 64;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tantivy::collector::{DocSetCollector, TopDocs};
use tantivy::directory::MmapDirectory;
use tantivy::directory::error::OpenDirectoryError;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, Query, TermQuery, TermSetQuery};
use tantivy::schema::Value;
use tantivy::{
    Index, IndexReader, IndexSettings, IndexWriter, ReloadPolicy, TantivyDocument as Document, Term,
};
use thiserror::Error;

use super::chunker::{Chunker, HybridChunker, RawChunk};
use super::config::{ChunkingConfig, CollectionConfig, ValidatedChunkingConfig};
use super::schema::DocumentSchema;
use super::types::{ChunkId, CollectionId, FileState};
use crate::indexing::file_info::{calculate_hash, get_utc_timestamp};
use crate::vector::{
    ClusterId, EmbeddingGenerator, MmapVectorStorage, SegmentOrdinal, VectorDimension, VectorId,
    VectorStorageError,
};

/// Errors from document storage operations.
#[derive(Error, Debug)]
pub enum DocumentStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("Directory error: {0}")]
    Directory(#[from] OpenDirectoryError),

    #[error("Vector storage error: {0}")]
    VectorStorage(#[from] VectorStorageError),

    #[error("Collection not found: {0}")]
    CollectionNotFound(String),

    #[error("Index error: {0}")]
    Index(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Invalid chunking configuration: {0}")]
    InvalidChunkingConfig(String),

    #[error("Lock poisoned")]
    LockPoisoned,
}

/// Result type for document store operations.
pub type StoreResult<T> = Result<T, DocumentStoreError>;

/// Statistics from an indexing operation.
#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    /// Number of files processed.
    pub files_processed: usize,
    /// Number of files skipped (unchanged).
    pub files_skipped: usize,
    /// Number of chunks created.
    pub chunks_created: usize,
    /// Number of chunks removed (from changed/deleted files).
    pub chunks_removed: usize,
}

/// Query parameters for document search.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Search text to embed and match.
    pub text: String,
    /// Filter by collection name.
    pub collection: Option<String>,
    /// Filter by source document path.
    pub document: Option<PathBuf>,
    /// Maximum results to return.
    pub limit: usize,
    /// Preview configuration (KWIC, highlighting, etc.).
    pub preview_config: Option<super::config::SearchConfig>,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            collection: None,
            document: None,
            limit: 10,
            preview_config: None,
        }
    }
}

/// Find case-insensitive keywords and map matches back to original UTF-8 spans.
/// Lowercasing can expand or shrink a character, so lowercase byte positions
/// must never be used directly to slice the source (for example, İ -> i + ◌̇).
fn keyword_match_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    let mut lowercase = String::with_capacity(text.len());
    let mut positions = Vec::new();
    for (source_start, character) in text.char_indices() {
        positions.push((
            lowercase.len(),
            source_start,
            source_start + character.len_utf8(),
        ));
        lowercase.extend(character.to_lowercase());
    }

    let mut matches = Vec::new();
    for word in query.split_whitespace() {
        if word.len() < 2 {
            continue;
        }
        let word: String = word.chars().flat_map(char::to_lowercase).collect();
        for (start, matched) in lowercase.match_indices(&word) {
            let end = start + matched.len();
            let first = positions.partition_point(|(position, _, _)| *position <= start) - 1;
            let last = positions.partition_point(|(position, _, _)| *position < end) - 1;
            matches.push((positions[first].1, positions[last].2));
        }
    }
    matches
}

/// Extract a KWIC (Keyword In Context) preview centered on the first keyword match.
/// Expands boundaries to word edges to avoid cutting words mid-character.
fn extract_kwic_preview(content: &str, query: &str, window_chars: usize) -> String {
    let match_pos = keyword_match_ranges(content, query)
        .into_iter()
        .map(|(start, _)| start)
        .min()
        .unwrap_or(0);

    // Calculate window boundaries (character-based)
    let half_window = window_chars / 2;
    let chars: Vec<char> = content.chars().collect();
    let total_chars = chars.len();

    // Find char position from byte position
    let char_pos = content[..match_pos].chars().count();

    let mut start_char = char_pos.saturating_sub(half_window);
    let mut end_char = char_pos.saturating_add(half_window).min(total_chars);

    // Expand start to word boundary (find previous whitespace)
    if start_char > 0 {
        while start_char > 0 && !chars[start_char - 1].is_whitespace() {
            start_char -= 1;
        }
    }

    // Expand end to word boundary (find next whitespace)
    if end_char < total_chars {
        while end_char < total_chars && !chars[end_char].is_whitespace() {
            end_char += 1;
        }
    }

    // Build preview
    let mut preview = String::new();

    if start_char > 0 {
        preview.push_str("...");
    }

    preview.extend(chars[start_char..end_char].iter());

    if end_char < total_chars {
        preview.push_str("...");
    }

    preview
}

/// Dual highlighting markers for both humans and LLMs.
/// - ANSI bold cyan: renders as color in terminals
/// - Text markers >>..<<: parseable pattern for LLMs
const HIGHLIGHT_START: &str = "\x1b[1;36m>>";
const HIGHLIGHT_END: &str = "<<\x1b[0m";

/// Highlight keywords with dual markers (ANSI + text).
/// Merges adjacent keywords: ">>word1<< >>word2<<" becomes ">>word1 word2<<"
fn highlight_keywords(text: &str, query: &str) -> String {
    let mut matches = keyword_match_ranges(text, query);

    if matches.is_empty() {
        return text.to_string();
    }

    // Sort by start position
    matches.sort_by_key(|m| m.0);

    // Merge overlapping or adjacent ranges (adjacent = spaces/tabs only, not newlines)
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in matches {
        if let Some(last) = merged.last_mut() {
            // Check overlap first — slice is only safe when start > last.1
            let is_adjacent = if start <= last.1 {
                true // overlapping, merge unconditionally
            } else {
                // Adjacent: only spaces/tabs between ranges (no newlines)
                text[last.1..start].chars().all(|c| c == ' ' || c == '\t')
            };
            if is_adjacent {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    // Build result with highlights
    let mut result = String::new();
    let mut offset = 0;

    for (start, end) in merged {
        result.push_str(&text[offset..start]);
        result.push_str(HIGHLIGHT_START);
        result.push_str(&text[start..end]);
        result.push_str(HIGHLIGHT_END);
        offset = end;
    }

    result.push_str(&text[offset..]);
    result
}

/// Generate preview from full content based on config.
fn generate_preview(content: &str, query: &str, config: &super::config::SearchConfig) -> String {
    use super::config::PreviewMode;

    let preview = match config.preview_mode {
        PreviewMode::Full => content.to_string(),
        PreviewMode::Kwic => extract_kwic_preview(content, query, config.preview_chars),
    };

    if config.highlight {
        highlight_keywords(&preview, query)
    } else {
        preview
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn unicode_expansion_before_match_keeps_kwic_and_highlight_source_positions() {
        assert_eq!(
            extract_kwic_preview("zero İé needle tail", "é", 2),
            "...İé..."
        );
        assert_eq!(
            highlight_keywords("İé needle", "é"),
            format!("İ{HIGHLIGHT_START}é{HIGHLIGHT_END} needle")
        );
    }

    #[test]
    fn expanded_or_shrunk_keywords_highlight_whole_original_characters() {
        assert_eq!(
            highlight_keywords("İé", "i\u{307}"),
            format!("{HIGHLIGHT_START}İ{HIGHLIGHT_END}é")
        );
        assert_eq!(
            highlight_keywords("Kİẞ", "ki\u{307}ß"),
            format!("{HIGHLIGHT_START}Kİẞ{HIGHLIGHT_END}")
        );
    }

    #[test]
    fn unicode_adjacent_highlights_merge_without_crossing_newlines() {
        assert_eq!(
            highlight_keywords("İstanbul Straße\nécole", "İSTANBUL Straße ÉCOLE"),
            format!(
                "{HIGHLIGHT_START}İstanbul Straße{HIGHLIGHT_END}\n{HIGHLIGHT_START}école{HIGHLIGHT_END}"
            )
        );
        assert_eq!(
            highlight_keywords("needle", "need needle"),
            format!("{HIGHLIGHT_START}needle{HIGHLIGHT_END}")
        );
        assert_eq!(highlight_keywords("é", ""), "é");
        assert_eq!(highlight_keywords("", "needle"), "");
    }
}

/// A search result with chunk metadata and backend-specific relevance score.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    /// Chunk identifier.
    pub chunk_id: ChunkId,
    /// Collection this chunk belongs to.
    pub collection: String,
    /// Source file path.
    pub source_path: PathBuf,
    /// Heading hierarchy for context.
    pub heading_context: Vec<String>,
    /// Content preview (first ~200 chars).
    pub content_preview: String,
    /// Byte range in source file.
    pub byte_range: (usize, usize),
    /// Cosine similarity for semantic search, or BM25 relevance for lexical search.
    pub similarity: f32,
}

/// Document store combining tantivy metadata with vector embeddings.
pub struct DocumentStore {
    /// Base path for all storage files.
    base_path: PathBuf,

    /// Tantivy index for chunk metadata.
    index: Index,

    /// Index reader for queries.
    reader: IndexReader,

    /// Schema fields for documents.
    schema: DocumentSchema,

    /// Index writer (lazily created).
    writer: Mutex<Option<IndexWriter<Document>>>,

    /// Vector storage for chunk embeddings.
    vector_storage: Option<MmapVectorStorage>,
    vector_staging: Option<tempfile::TempDir>,
    original_vectors: Option<MmapVectorStorage>,

    /// Cluster assignments for IVFFlat search.
    cluster_assignments: HashMap<VectorId, ClusterId>,

    /// Cluster centroids.
    centroids: Vec<Vec<f32>>,

    /// File states for change detection.
    file_states: HashMap<PathBuf, FileState>,

    /// Successful chunking policy and embedding generation for each source.
    chunking_fingerprints: HashMap<PathBuf, String>,
    embedded_files: HashMap<PathBuf, String>,
    embedding_identity: Option<String>,
    forced_collections: HashSet<String>,

    /// Collection name to ID mapping.
    collection_ids: HashMap<String, CollectionId>,

    /// Next chunk ID counter.
    next_chunk_id: u64,

    /// Chunker implementation.
    chunker: Box<dyn Chunker>,

    /// Embedding generator (optional).
    embedding_generator: Option<Arc<dyn EmbeddingGenerator>>,

    /// Optional accelerator keyed by exact chunk content, independent of chunk IDs.
    embedding_cache: Option<crate::embedding_cache::EmbeddingCache>,

    /// Vector dimension.
    dimension: VectorDimension,

    /// Tantivy heap size in bytes.
    heap_size: usize,

    /// Optional immutable local source boundary, shared by query snapshots.
    workspace_root: Option<Arc<Path>>,
}

impl std::fmt::Debug for DocumentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentStore")
            .field("base_path", &self.base_path)
            .field("has_vector_storage", &self.vector_storage.is_some())
            .field(
                "has_embedding_generator",
                &self.embedding_generator.is_some(),
            )
            .field("file_states_count", &self.file_states.len())
            .field("collection_count", &self.collection_ids.len())
            .field("next_chunk_id", &self.next_chunk_id)
            .finish()
    }
}

impl DocumentStore {
    /// Create or open a document store.
    ///
    /// # Arguments
    /// * `base_path` - Directory for all storage files (tantivy index, vectors, state)
    /// * `dimension` - Vector dimension for embeddings
    pub fn new(base_path: impl AsRef<Path>, dimension: VectorDimension) -> StoreResult<Self> {
        let base_path = base_path.as_ref().to_path_buf();
        std::fs::create_dir_all(&base_path)?;

        let index_path = base_path.join("tantivy");
        std::fs::create_dir_all(&index_path)?;

        let (tantivy_schema, document_schema) = DocumentSchema::build();

        // Create or open tantivy index
        let index = if index_path.join("meta.json").exists() {
            Index::open_in_dir(&index_path)?
        } else {
            let dir = MmapDirectory::open(&index_path)?;
            Index::create(dir, tantivy_schema, IndexSettings::default())?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        // If opening existing index, reload to get latest segments
        if index_path.join("meta.json").exists() {
            reader.reload()?;
        }

        // Load persisted state if available
        let state_path = base_path.join("state.json");
        let state = if state_path.exists() {
            Self::load_state(&state_path)?
        } else {
            PersistedState::default()
        };

        Ok(Self {
            base_path,
            index,
            reader,
            schema: document_schema,
            writer: Mutex::new(None),
            vector_storage: None,
            vector_staging: None,
            original_vectors: None,
            cluster_assignments: HashMap::new(),
            centroids: Vec::new(),
            file_states: state
                .file_states
                .into_iter()
                .map(|(path, state)| (PathBuf::from(path), state))
                .collect(),
            chunking_fingerprints: state
                .chunking_fingerprints
                .into_iter()
                .map(|(path, fingerprint)| (PathBuf::from(path), fingerprint))
                .collect(),
            embedded_files: state
                .embedded_files
                .into_iter()
                .map(|(path, identity)| (PathBuf::from(path), identity))
                .collect(),
            embedding_identity: state.embedding_identity,
            forced_collections: HashSet::new(),
            collection_ids: state
                .collection_ids
                .into_iter()
                .filter_map(|(name, id)| CollectionId::from_u32(id).map(|id| (name, id)))
                .collect(),
            next_chunk_id: state.next_chunk_id.max(1),
            chunker: Box::new(HybridChunker::new()),
            embedding_generator: None,
            embedding_cache: None,
            dimension,
            heap_size: 50_000_000, // 50MB default
            workspace_root: None,
        })
    }

    /// Bind an already opened store to one canonical workspace. Validate existing
    /// provenance once; returned hits are checked again after external commits.
    pub(crate) fn restrict_workspace(&mut self, root: &Path) -> StoreResult<()> {
        let root = root.canonicalize()?;
        for path in self.file_states.keys() {
            crate::indexing::facade::IndexFacade::contained_source(&root, path)
                .map_err(|error| DocumentStoreError::Index(error.to_string()))?;
        }
        self.workspace_root = Some(Arc::from(root));
        Ok(())
    }

    /// Enable embedding generation for semantic search.
    pub fn with_embeddings(mut self, generator: Box<dyn EmbeddingGenerator>) -> StoreResult<Self> {
        if generator.dimension() != self.dimension {
            return Err(DocumentStoreError::Embedding(format!(
                "Embedding generator dimension {} does not match document store dimension {}",
                generator.dimension().get(),
                self.dimension.get()
            )));
        }
        let identity = format!("{};document-input=2", generator.cache_identity());
        if let Some(previous) = &self.embedding_identity {
            if previous != &identity {
                return Err(DocumentStoreError::Embedding(format!(
                    "Document embedding model or preprocessing changed ({previous} -> {identity}). Select the original model or rebuild documents in a new index directory; existing vectors were preserved."
                )));
            }
        }
        // Initialize vector storage
        let vector_path = self.base_path.join("vectors");
        std::fs::create_dir_all(&vector_path)?;

        let vector_storage = MmapVectorStorage::open_or_create(
            &vector_path,
            SegmentOrdinal::new(0),
            self.dimension,
        )?;

        if self.embedding_identity.is_none() && vector_storage.vector_count() > 0 {
            return Err(DocumentStoreError::Embedding(
                "Legacy document vectors have no model identity. Rebuild documents in a new index directory before semantic search; the existing index is still available for lexical search.".into()
            ));
        }
        let expected_vectors: usize = self
            .file_states
            .iter()
            .filter(|(path, _)| self.embedded_files.contains_key(*path))
            .map(|(_, state)| state.chunk_ids.len())
            .sum();
        if vector_storage.vector_count() < expected_vectors {
            self.embedded_files.clear();
        }
        self.embedding_identity = Some(identity.clone());

        let generator: Arc<dyn EmbeddingGenerator> = Arc::from(generator);
        self.embedding_cache = Some(crate::embedding_cache::EmbeddingCache::load(
            &self.base_path.join("embedding-cache.json"),
            &identity,
            self.dimension.get(),
        ));
        self.vector_storage = Some(vector_storage);
        self.embedding_generator = Some(generator);

        // Load cluster data if available
        self.load_cluster_data()?;

        Ok(self)
    }

    /// Count the number of files that would be indexed for a collection.
    ///
    /// Useful for progress bar setup before indexing.
    pub fn count_collection_files(&self, config: &CollectionConfig) -> StoreResult<usize> {
        let files = self.collect_files(config)?;
        Ok(files.len())
    }

    /// Index documents from a collection configuration.
    ///
    /// Only processes files that have changed since last index.
    pub fn index_collection(
        &mut self,
        name: &str,
        config: &CollectionConfig,
        chunking_config: &ChunkingConfig,
    ) -> StoreResult<IndexStats> {
        self.index_collection_with_progress(name, config, chunking_config, |_| {})
    }

    /// Index documents from a collection with progress callback.
    ///
    /// Progress is reported in two phases:
    /// 1. `ProcessingFile` - for each file being chunked and indexed
    /// 2. `GeneratingEmbeddings` - for each batch of embeddings generated
    ///
    /// Embeddings are generated in batches to reduce memory pressure.
    pub fn index_collection_with_progress<F>(
        &mut self,
        name: &str,
        config: &CollectionConfig,
        chunking_config: &ChunkingConfig,
        mut on_progress: F,
    ) -> StoreResult<IndexStats>
    where
        F: FnMut(IndexProgress<'_>),
    {
        let chunking_config = ValidatedChunkingConfig::try_from(chunking_config.clone())
            .map_err(DocumentStoreError::InvalidChunkingConfig)?;
        let fingerprint = chunking_fingerprint(&chunking_config)?;
        self.transaction(|store| {
            store.index_collection_inner(
                name,
                config,
                &chunking_config,
                &fingerprint,
                &mut on_progress,
            )
        })
    }

    fn index_collection_inner<F>(
        &mut self,
        name: &str,
        config: &CollectionConfig,
        chunking_config: &ValidatedChunkingConfig,
        fingerprint: &str,
        mut on_progress: F,
    ) -> StoreResult<IndexStats>
    where
        F: FnMut(IndexProgress<'_>),
    {
        let mut stats = IndexStats::default();

        // Ensure collection has an ID
        let _collection_id = self.get_or_create_collection_id(name);

        // Collect files to process
        on_progress(IndexProgress::Phase {
            name: "discovering files",
        });
        let files = self.collect_files(config)?;

        for path in &files {
            if let Some(state) = self.file_states.get(path) {
                if state.collection != name {
                    return Err(DocumentStoreError::Index(format!(
                        "Document {} already belongs to collection '{}'; overlapping collection ownership is not supported",
                        path.display(),
                        state.collection
                    )));
                }
            }
        }

        // Detect changes
        let (changed, unchanged, removed) = self.detect_changes(&files, name, fingerprint)?;

        tracing::info!(
            target: "rag",
            "collection '{}': {} to index, {} unchanged, {} removed",
            name,
            changed.len(),
            unchanged.len(),
            removed.len()
        );

        stats.files_skipped = unchanged.len();

        // Remove chunks from deleted/changed files
        for path in removed.iter().chain(changed.iter()) {
            if let Some(state) = self.file_states.get(path) {
                let chunk_count = state.chunk_ids.len();
                stats.chunks_removed += chunk_count;
                tracing::info!(
                    target: "rag",
                    "deleted {} chunks from {}",
                    chunk_count,
                    path.display()
                );
            }
            // A previous interrupted metadata commit may not have a state entry.
            // Replacement is keyed by source, not by the presence of that cache.
            self.delete_chunks_by_file(path, name)?;
        }

        // Phase 1: Process files (chunking and metadata)
        // Chunk text can be much larger than its final vector. Spool it to disk
        // instead of retaining an entire collection generation in RAM.
        let mut embedding_spool = self
            .embedding_generator
            .as_ref()
            .map(|_| tempfile::NamedTempFile::new_in(&self.base_path))
            .transpose()?;
        let mut pending_embedding_count = 0usize;
        let total_files = changed.len();

        for (idx, path) in changed.iter().enumerate() {
            // Report file processing progress
            on_progress(IndexProgress::ProcessingFile {
                current: idx + 1,
                total: total_files,
                path,
            });

            let content = std::fs::read_to_string(path)?;
            let raw_chunks = self.chunker.chunk(&content, chunking_config);

            let mut chunk_ids = Vec::new();

            for raw_chunk in raw_chunks {
                let chunk_id = self.allocate_chunk_id();
                chunk_ids.push(chunk_id);

                // Store chunk metadata in tantivy
                self.store_chunk(chunk_id, name, path, &raw_chunk, &content)?;

                if let Some(spool) = embedding_spool.as_mut() {
                    serde_json::to_writer(
                        &mut *spool,
                        &(chunk_id.get(), embedding_input(&raw_chunk)),
                    )
                    .map_err(|e| {
                        DocumentStoreError::Index(format!(
                            "Failed to spool document embedding input: {e}"
                        ))
                    })?;
                    spool.write_all(b"\n")?;
                    pending_embedding_count += 1;
                }

                stats.chunks_created += 1;
            }

            // Update file state
            let file_state = FileState {
                path: path.clone(),
                collection: name.to_string(),
                content_hash: calculate_hash(&content),
                chunk_ids,
                last_indexed: get_utc_timestamp(),
                mtime: crate::indexing::file_info::get_file_mtime(path).unwrap_or(0),
            };
            self.file_states.insert(path.clone(), file_state);
            self.chunking_fingerprints
                .insert(path.clone(), fingerprint.to_string());
            self.embedded_files.remove(path);

            stats.files_processed += 1;
        }

        // Phase 2: Generate embeddings in batches
        if let Some(mut spool) = embedding_spool {
            let embed_count = pending_embedding_count;
            self.process_embedding_spool(&mut spool, embed_count, &mut on_progress)?;
            tracing::info!(
                target: "rag",
                "generated embeddings for {} chunks",
                embed_count
            );
        }

        // Remove file states for deleted files
        for path in &removed {
            self.file_states.remove(path);
            self.chunking_fingerprints.remove(path);
            self.embedded_files.remove(path);
        }
        if self.embedding_generator.is_some() {
            if let Some(identity) = &self.embedding_identity {
                for path in &changed {
                    self.embedded_files.insert(path.clone(), identity.clone());
                }
            }
        }
        self.forced_collections.remove(name);

        // Persist state
        on_progress(IndexProgress::Phase {
            name: "committing metadata and state",
        });

        Ok(stats)
    }

    /// Re-index a single file.
    ///
    /// Used by the file watcher when a document changes. Looks up the file's
    /// collection from stored state and re-indexes with the provided config.
    ///
    /// Returns the number of chunks created, or None if file wasn't indexed.
    pub fn reindex_file(
        &mut self,
        path: &Path,
        chunking_config: &ChunkingConfig,
    ) -> StoreResult<Option<usize>> {
        let chunking_config = ValidatedChunkingConfig::try_from(chunking_config.clone())
            .map_err(DocumentStoreError::InvalidChunkingConfig)?;
        let path = normalize_source_path(path);
        self.transaction(|store| store.reindex_file_inner(&path, &chunking_config))
    }

    fn reindex_file_inner(
        &mut self,
        path: &Path,
        chunking_config: &ValidatedChunkingConfig,
    ) -> StoreResult<Option<usize>> {
        // Look up collection from file state
        let (collection, old_chunk_count) = match self.file_states.get(path) {
            Some(state) => (state.collection.clone(), state.chunk_ids.len()),
            None => return Ok(None), // File not in index
        };

        // Read file content
        let content = std::fs::read_to_string(path)?;

        // Chunk the content
        let raw_chunks = self.chunker.chunk(&content, chunking_config);

        // Delete existing chunks
        self.delete_chunks_by_file(path, &collection)?;
        tracing::info!(
            target: "rag",
            "deleted {} chunks from {}",
            old_chunk_count,
            path.display()
        );

        let mut chunk_ids = Vec::new();
        let mut pending_embeddings: Vec<(ChunkId, String)> = Vec::new();

        for raw_chunk in raw_chunks {
            let chunk_id = self.allocate_chunk_id();
            chunk_ids.push(chunk_id);

            // Store chunk metadata in tantivy
            self.store_chunk(chunk_id, &collection, path, &raw_chunk, &content)?;

            // Queue for embedding
            pending_embeddings.push((chunk_id, embedding_input(&raw_chunk)));
        }

        // Generate embeddings
        let chunks_created = pending_embeddings.len();
        if !pending_embeddings.is_empty() {
            self.process_embeddings_batched(&pending_embeddings, &mut |_| {})?;
            tracing::info!(
                target: "rag",
                "generated embeddings for {} chunks",
                chunks_created
            );
        }

        // Update file state
        let file_state = FileState {
            path: path.to_path_buf(),
            collection,
            content_hash: calculate_hash(&content),
            chunk_ids,
            last_indexed: get_utc_timestamp(),
            mtime: crate::indexing::file_info::get_file_mtime(path).unwrap_or(0),
        };
        self.file_states.insert(path.to_path_buf(), file_state);
        self.chunking_fingerprints
            .insert(path.to_path_buf(), chunking_fingerprint(chunking_config)?);
        self.embedded_files.remove(path);
        if self.embedding_generator.is_some() {
            if let Some(identity) = &self.embedding_identity {
                self.embedded_files
                    .insert(path.to_path_buf(), identity.clone());
            }
        }

        Ok(Some(chunks_created))
    }

    /// Remove a file from the index.
    ///
    /// Used by the file watcher when a document is deleted.
    /// Returns true if the file was in the index.
    pub fn remove_file(&mut self, path: &Path) -> StoreResult<bool> {
        let path = normalize_source_path(path);
        self.transaction(|store| store.remove_file_inner(&path))
    }

    fn remove_file_inner(&mut self, path: &Path) -> StoreResult<bool> {
        let Some(state) = self.file_states.remove(path) else {
            return Ok(false);
        };

        let chunk_count = state.chunk_ids.len();

        // Delete chunks from tantivy
        self.delete_chunks_by_file(path, &state.collection)?;
        self.chunking_fingerprints.remove(path);
        self.embedded_files.remove(path);

        tracing::info!(
            target: "rag",
            "removed {} chunks for deleted file {}",
            chunk_count,
            path.display()
        );

        Ok(true)
    }

    /// Get the collection name for a file, if indexed.
    pub fn get_file_collection(&self, path: &Path) -> Option<&str> {
        self.file_states
            .get(&normalize_source_path(path))
            .map(|s| s.collection.as_str())
    }

    /// Get all indexed file paths.
    pub fn get_indexed_paths(&self) -> Vec<PathBuf> {
        self.file_states.keys().cloned().collect()
    }

    /// Mark indexed collections for replacement without discarding their source
    /// ownership. A selected force operation must preserve other collections.
    pub fn clear_file_states(&mut self) {
        self.forced_collections
            .extend(self.collection_ids.keys().cloned());
    }

    /// Search for chunks matching a query.
    pub(crate) fn query_snapshot(&self) -> DocumentQuery {
        // Only clone reader/model handles under the store guard. No files are opened
        // and no vector payloads or file-state corpus are copied here.
        DocumentQuery(Self {
            base_path: self.base_path.clone(),
            index: self.index.clone(),
            reader: self.reader.clone(),
            schema: self.schema,
            writer: Mutex::new(None),
            vector_storage: None,
            vector_staging: None,
            original_vectors: None,
            cluster_assignments: HashMap::new(),
            centroids: Vec::new(),
            file_states: HashMap::new(),
            chunking_fingerprints: HashMap::new(),
            embedded_files: HashMap::new(),
            embedding_identity: self.embedding_identity.clone(),
            forced_collections: HashSet::new(),
            collection_ids: HashMap::new(),
            next_chunk_id: self.next_chunk_id,
            chunker: Box::new(HybridChunker::new()),
            embedding_generator: self.embedding_generator.clone(),
            embedding_cache: None,
            dimension: self.dimension,
            heap_size: self.heap_size,
            workspace_root: self.workspace_root.clone(),
        })
    }

    pub fn search(&mut self, query: SearchQuery) -> StoreResult<Vec<SearchResult>> {
        if query.text.trim().is_empty() || query.limit == 0 {
            return Ok(Vec::new());
        }

        if self.embedding_generator.is_none() {
            return self.search_lexical(&query);
        }

        // Get candidate chunks based on filters
        let candidates = self.get_filtered_candidates(&query)?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let generator = self.embedding_generator.as_ref().expect("checked above");

        // Generate query embedding
        let query_embeddings = generator
            .generate_embeddings(&[query.text.as_str()])
            .map_err(|e| DocumentStoreError::Embedding(e.to_string()))?;

        let query_vec = query_embeddings
            .into_iter()
            .next()
            .ok_or_else(|| DocumentStoreError::Embedding("No embedding generated".to_string()))?;

        // Score candidates by vector similarity
        let mut scored_candidates = self.score_by_similarity(&candidates, &query_vec)?;

        retain_top_chunks(&mut scored_candidates, query.limit);

        // Enrich with full metadata and KWIC preview
        self.build_search_results(scored_candidates, &query)
    }

    fn search_lexical(&self, query: &SearchQuery) -> StoreResult<Vec<SearchResult>> {
        // Natural-language input is analyzed as literal terms, never parsed as
        // Tantivy query syntax. Headings contribute without excluding body hits.
        let mut analyzer = self.index.tokenizers().get("default").ok_or_else(|| {
            DocumentStoreError::Index("Document text tokenizer is unavailable".into())
        })?;
        let mut stream = analyzer.token_stream(&query.text);
        let mut terms = HashSet::new();
        while stream.advance() {
            terms.insert(stream.token().text.clone());
        }
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut text_queries: Vec<Box<dyn Query>> = Vec::new();
        for text in terms {
            for (field, boost) in [
                (self.schema.content, 1.0),
                (self.schema.heading_context, 2.0),
            ] {
                let term = Term::from_field_text(field, &text);
                text_queries.push(Box::new(BoostQuery::new(
                    Box::new(TermQuery::new(
                        term,
                        tantivy::schema::IndexRecordOption::WithFreqs,
                    )),
                    boost,
                )));
            }
        }
        let mut clauses = self.filter_clauses(query);
        clauses.push((Occur::Must, Box::new(BooleanQuery::union(text_queries))));
        let searcher = self.reader.searcher();
        let hits = searcher.search(
            &BooleanQuery::new(clauses),
            &TopDocs::with_limit(query.limit).order_by_score(),
        )?;
        let mut scored = Vec::with_capacity(hits.len());
        for (score, address) in hits {
            let doc: Document = searcher.doc(address)?;
            if let Some(id) = doc
                .get_first(self.schema.chunk_id)
                .and_then(|value| value.as_u64())
                .and_then(|id| u32::try_from(id).ok())
                .and_then(ChunkId::from_u32)
            {
                scored.push((id, score));
            }
        }
        self.build_search_results(scored, query)
    }

    /// Delete all chunks from a collection.
    pub fn delete_collection(&mut self, name: &str) -> StoreResult<usize> {
        let searcher = self.reader.searcher();

        // Find all chunks in collection
        let term = Term::from_field_text(self.schema.collection_name, name);
        let query = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);

        let count = searcher.search(&query, &tantivy::collector::Count)?;

        // Delete from tantivy
        {
            let mut writer_guard = self
                .writer
                .lock()
                .map_err(|_| DocumentStoreError::LockPoisoned)?;
            let writer = self.ensure_writer(&mut writer_guard)?;

            let term = Term::from_field_text(self.schema.collection_name, name);
            writer.delete_term(term);
            writer.commit()?;
        }

        self.reader.reload()?;

        // Each state records its collection; preserve every other collection,
        // including empty files and legacy states with an unknown collection.
        self.file_states.retain(|_, state| state.collection != name);
        self.chunking_fingerprints
            .retain(|path, _| self.file_states.contains_key(path));
        self.embedded_files
            .retain(|path, _| self.file_states.contains_key(path));
        self.forced_collections.remove(name);

        // Remove collection ID
        self.collection_ids.remove(name);

        self.save_state()?;

        Ok(count)
    }

    /// Get statistics about a collection.
    pub fn collection_stats(&self, name: &str) -> StoreResult<CollectionStats> {
        let searcher = self.reader.searcher();

        let term = Term::from_field_text(self.schema.collection_name, name);
        let query = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);

        let count = searcher.search(&query, &tantivy::collector::Count)?;

        let file_count = self
            .file_states
            .values()
            .filter(|s| s.collection == name && !s.chunk_ids.is_empty())
            .count();

        Ok(CollectionStats {
            name: name.to_string(),
            chunk_count: count,
            file_count,
        })
    }

    /// List all collections.
    pub fn list_collections(&self) -> Vec<String> {
        self.collection_ids.keys().cloned().collect()
    }

    // Private helper methods

    fn allocate_chunk_id(&mut self) -> ChunkId {
        let id = self.next_chunk_id;
        self.next_chunk_id += 1;
        ChunkId::from_u32(id as u32).unwrap_or_else(|| {
            // Wrap around if we hit zero
            self.next_chunk_id = 2;
            ChunkId::from_u32(1).expect("1 is not zero")
        })
    }

    fn get_or_create_collection_id(&mut self, name: &str) -> CollectionId {
        if let Some(&id) = self.collection_ids.get(name) {
            return id;
        }

        let id = CollectionId::from_u32((self.collection_ids.len() + 1) as u32)
            .expect("collection ID should be valid (non-zero)");
        self.collection_ids.insert(name.to_string(), id);
        id
    }

    fn collect_files(&self, config: &CollectionConfig) -> StoreResult<Vec<PathBuf>> {
        Self::discover_files(config)
    }

    /// Shared collection discovery for indexing and watcher reconciliation.
    pub(crate) fn discover_files(config: &CollectionConfig) -> StoreResult<Vec<PathBuf>> {
        let mut files = Vec::new();
        let patterns = config
            .effective_patterns()
            .into_iter()
            .map(|pattern| {
                glob::Pattern::new(&pattern).map_err(|e| {
                    DocumentStoreError::Index(format!("Invalid glob pattern '{pattern}': {e}"))
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;

        for base_path in &config.paths {
            if !base_path.exists() {
                continue;
            }

            if base_path.is_file() {
                files.push(base_path.clone());
                continue;
            }

            // Document collections are governed by Codanna's indexing policy,
            // not Git's tracking policy. A user may intentionally index local
            // knowledge that is excluded from version control, while dependency
            // and generated trees belong in `.codannaignore`.
            let mut walker = ignore::WalkBuilder::new(base_path);
            walker
                .hidden(false)
                .ignore(false)
                .git_ignore(false)
                .git_global(false)
                .git_exclude(false)
                .follow_links(false);
            walker.add_custom_ignore_filename(".codannaignore");

            for entry in walker.build() {
                let entry = entry.map_err(|error| {
                    DocumentStoreError::Index(format!(
                        "Document discovery failed beneath {}: {error}",
                        base_path.display()
                    ))
                })?;
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                let relative = path.strip_prefix(base_path).unwrap_or(path);
                if patterns
                    .iter()
                    .any(|pattern| pattern.matches_path(relative))
                {
                    files.push(path.to_path_buf());
                }
            }
        }

        let mut files = files
            .into_iter()
            .map(|path| path.canonicalize())
            .collect::<Result<Vec<_>, _>>()?;
        files.sort();
        files.dedup();
        Ok(files)
    }

    fn detect_changes(
        &self,
        files: &[PathBuf],
        collection: &str,
        fingerprint: &str,
    ) -> StoreResult<(Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>)> {
        let mut changed = Vec::new();
        let mut unchanged = Vec::new();
        let mut removed: Vec<PathBuf> = Vec::new();

        let current_files: std::collections::HashSet<_> = files.iter().collect();

        // Find changed and unchanged files
        for path in files {
            if let Some(state) = self.file_states.get(path) {
                // mtime is not content identity: multiple saves can share one
                // second, and sync tools can preserve timestamps deliberately.
                let content = std::fs::read_to_string(path)?;
                let needs_vectors = self.embedding_generator.is_some()
                    && self.embedded_files.get(path) != self.embedding_identity.as_ref();
                if !self.forced_collections.contains(collection)
                    && self
                        .chunking_fingerprints
                        .get(path)
                        .is_some_and(|value| value == fingerprint)
                    && !needs_vectors
                    && calculate_hash(&content) == state.content_hash
                {
                    unchanged.push(path.clone());
                } else {
                    changed.push(path.clone());
                }
            } else {
                // New file
                changed.push(path.clone());
            }
        }

        // Find removed files. Scope to the target collection: file_states is a
        // single map across all collections, so without this filter, indexing
        // collection B would classify every collection-A file as "removed" and
        // wipe its chunks (issue #100).
        for (path, state) in self.file_states.iter() {
            if state.collection == collection && !current_files.contains(path) {
                removed.push(path.clone());
            }
        }

        tracing::debug!(
            target: "rag",
            "detect_changes: collection={}, changed={}, unchanged={}, removed={}",
            collection,
            changed.len(),
            unchanged.len(),
            removed.len()
        );

        Ok((changed, unchanged, removed))
    }

    fn store_chunk(
        &mut self,
        chunk_id: ChunkId,
        collection: &str,
        source_path: &Path,
        raw_chunk: &RawChunk,
        _full_content: &str,
    ) -> StoreResult<()> {
        let mut writer_guard = self
            .writer
            .lock()
            .map_err(|_| DocumentStoreError::LockPoisoned)?;
        let writer = self.ensure_writer(&mut writer_guard)?;

        let mut doc = Document::new();

        // Document type discriminator
        doc.add_text(self.schema.doc_type, "chunk");

        // Chunk ID
        doc.add_u64(self.schema.chunk_id, chunk_id.get() as u64);

        // Collection name
        doc.add_text(self.schema.collection_name, collection);

        // Source path
        doc.add_text(
            self.schema.source_path,
            source_path.to_string_lossy().as_ref(),
        );

        // Heading context as JSON array
        let heading_json =
            serde_json::to_string(&raw_chunk.heading_context).unwrap_or_else(|_| "[]".to_string());
        doc.add_text(self.schema.heading_context, &heading_json);

        // Full content
        doc.add_text(self.schema.content, &raw_chunk.content);

        // Content preview (first ~200 chars)
        let preview: String = raw_chunk.content.chars().take(200).collect();
        doc.add_text(self.schema.content_preview, &preview);

        // Byte offsets
        doc.add_u64(self.schema.byte_start, raw_chunk.byte_range.0 as u64);
        doc.add_u64(self.schema.byte_end, raw_chunk.byte_range.1 as u64);

        // Character count
        doc.add_u64(self.schema.char_count, raw_chunk.char_count() as u64);

        // Indexed timestamp
        doc.add_u64(self.schema.indexed_at, get_utc_timestamp());

        writer.add_document(doc)?;

        Ok(())
    }

    fn delete_chunks_by_file(&mut self, path: &Path, _collection: &str) -> StoreResult<()> {
        let mut writer_guard = self
            .writer
            .lock()
            .map_err(|_| DocumentStoreError::LockPoisoned)?;
        let writer = self.ensure_writer(&mut writer_guard)?;

        let term = Term::from_field_text(self.schema.source_path, path.to_string_lossy().as_ref());
        writer.delete_term(term);

        Ok(())
    }

    fn ensure_writer<'a>(
        &self,
        writer_guard: &'a mut Option<IndexWriter<Document>>,
    ) -> StoreResult<&'a mut IndexWriter<Document>> {
        if writer_guard.is_none() {
            *writer_guard = Some(self.index.writer(self.heap_size)?);
        }
        Ok(writer_guard.as_mut().unwrap())
    }

    fn commit(&mut self) -> StoreResult<()> {
        let mut writer_guard = self
            .writer
            .lock()
            .map_err(|_| DocumentStoreError::LockPoisoned)?;

        if let Some(ref mut writer) = *writer_guard {
            writer.commit()?;
        }

        self.reader.reload()?;

        Ok(())
    }

    /// Keep metadata mutations private until every embedding batch succeeds.
    /// Vectors are copied lazily to a sibling staging inode, so failed batches
    /// and active mmap readers cannot observe partially appended records.
    fn transaction<T>(
        &mut self,
        action: impl FnOnce(&mut Self) -> StoreResult<T>,
    ) -> StoreResult<T> {
        let file_states = self.file_states.clone();
        let chunking_fingerprints = self.chunking_fingerprints.clone();
        let embedded_files = self.embedded_files.clone();
        let collection_ids = self.collection_ids.clone();
        let forced_collections = self.forced_collections.clone();
        let result = action(self).and_then(|value| {
            self.publish_staged_vectors()?;
            self.commit()?;
            Ok(value)
        });
        match result {
            Ok(value) => {
                // State publication is atomic. If this write fails after the
                // Tantivy commit, retain the committed in-memory generation so
                // a retry can persist it without duplicating document chunks.
                self.save_state()?;
                Ok(value)
            }
            Err(error) => {
                if let Ok(mut guard) = self.writer.lock() {
                    if let Some(writer) = guard.as_mut() {
                        if let Err(rollback_error) = writer.rollback() {
                            tracing::error!(target: "documents", %rollback_error, "document metadata rollback failed");
                        }
                    }
                }
                if let Some(original) = self.original_vectors.take() {
                    self.vector_storage = Some(original);
                }
                self.vector_staging = None;
                self.file_states = file_states;
                self.chunking_fingerprints = chunking_fingerprints;
                self.embedded_files = embedded_files;
                self.collection_ids = collection_ids;
                self.forced_collections = forced_collections;
                // IDs are deliberately not reused after a failed attempt.
                if let Err(state_error) = self.save_state() {
                    tracing::warn!(target: "documents", %state_error, "failed to persist document retry state");
                }
                Err(error)
            }
        }
    }

    fn stage_vector_writes(&mut self) -> StoreResult<()> {
        if self.vector_staging.is_some() {
            return Ok(());
        }
        let Some(original) = self.vector_storage.take() else {
            return Ok(());
        };
        self.original_vectors = Some(original);
        let staging = tempfile::Builder::new()
            .prefix(".document-vectors-")
            .tempdir_in(&self.base_path)?;
        let active = self.base_path.join("vectors/segment_0.vec");
        if active.exists() {
            std::fs::copy(active, staging.path().join("segment_0.vec"))?;
        }
        self.vector_storage = Some(MmapVectorStorage::open_or_create(
            staging.path(),
            SegmentOrdinal::new(0),
            self.dimension,
        )?);
        self.vector_staging = Some(staging);
        Ok(())
    }

    fn publish_staged_vectors(&mut self) -> StoreResult<()> {
        let Some(staging) = self.vector_staging.take() else {
            return Ok(());
        };
        let parent = self.base_path.join("vectors");
        let temporary = tempfile::NamedTempFile::new_in(&parent)?.into_temp_path();
        self.vector_storage = None;
        std::fs::rename(staging.path().join("segment_0.vec"), &temporary)?;
        std::fs::File::open(&temporary)?.sync_all()?;
        temporary
            .persist(parent.join("segment_0.vec"))
            .map_err(|error| error.error)?;
        self.vector_storage = Some(MmapVectorStorage::open(&parent, SegmentOrdinal::new(0))?);
        self.original_vectors = None;
        Ok(())
    }

    /// Process embeddings in batches with progress reporting.
    ///
    /// Batching reduces memory pressure and provides smoother progress updates.
    fn process_embeddings_batched<F>(
        &mut self,
        chunks: &[(ChunkId, String)],
        on_progress: &mut F,
    ) -> StoreResult<()>
    where
        F: FnMut(IndexProgress<'_>),
    {
        let total_chunks = chunks.len();
        let mut processed = 0;
        let accelerated = crate::memory::accelerated_embeddings_requested();

        on_progress(IndexProgress::GeneratingEmbeddings {
            current: 0,
            total: total_chunks,
        });

        while processed < chunks.len() {
            let memory = crate::memory::MemoryBudget::current();
            if memory.under_pressure() {
                return Err(DocumentStoreError::Embedding(format!(
                    "embedding stopped before swap pressure (available={} MiB, rss={} MiB); \
                     metadata is safe and the run can be resumed",
                    memory.available / (1024 * 1024),
                    memory.process_rss / (1024 * 1024),
                )));
            }
            let batch_size = memory.embedding_batch_size(EMBEDDING_BATCH_SIZE, accelerated);
            let end = (processed + batch_size).min(chunks.len());
            self.process_embedding_batch(&chunks[processed..end])?;
            processed = end;

            // Report progress
            on_progress(IndexProgress::GeneratingEmbeddings {
                current: processed,
                total: total_chunks,
            });
        }

        on_progress(IndexProgress::Phase {
            name: "finalizing embeddings",
        });

        Ok(())
    }

    fn process_embedding_batch(&mut self, batch: &[(ChunkId, String)]) -> StoreResult<()> {
        let Some(generator) = self.embedding_generator.clone() else {
            return Ok(());
        };
        if self.vector_storage.is_none() {
            return Ok(());
        }

        let mut vectors: Vec<(ChunkId, Arc<[f32]>)> = Vec::with_capacity(batch.len());
        let mut missing: Vec<(&str, Vec<ChunkId>)> = Vec::new();
        let mut missing_by_text: HashMap<&str, usize> = HashMap::new();
        for (chunk_id, text) in batch {
            if let Some(hit) = self
                .embedding_cache
                .as_ref()
                .and_then(|cache| cache.get(text))
            {
                vectors.push((*chunk_id, hit));
            } else if let Some(index) = missing_by_text.get(text.as_str()).copied() {
                missing[index].1.push(*chunk_id);
            } else {
                missing_by_text.insert(text, missing.len());
                missing.push((text, vec![*chunk_id]));
            }
        }

        if !missing.is_empty() {
            let texts: Vec<&str> = missing.iter().map(|(text, _)| *text).collect();
            let generated = generator
                .generate_embeddings(&texts)
                .map_err(|e| DocumentStoreError::Embedding(e.to_string()))?;
            if generated.len() != missing.len() {
                return Err(DocumentStoreError::Embedding(format!(
                    "embedding backend returned {} vectors for {} inputs",
                    generated.len(),
                    missing.len()
                )));
            }
            for ((text, chunk_ids), embedding) in missing.into_iter().zip(generated) {
                if embedding.len() != self.dimension.get()
                    || !embedding.iter().all(|value| value.is_finite())
                {
                    return Err(DocumentStoreError::Embedding(format!(
                        "embedding backend returned an invalid {}-element vector; expected {} finite elements",
                        embedding.len(),
                        self.dimension.get()
                    )));
                }
                let embedding: Arc<[f32]> = Arc::from(embedding);
                if let Some(cache) = self.embedding_cache.as_mut() {
                    cache.insert(text, Arc::clone(&embedding));
                }
                vectors.extend(
                    chunk_ids
                        .into_iter()
                        .map(|chunk_id| (chunk_id, Arc::clone(&embedding))),
                );
            }
        }

        let vector_pairs: Vec<(VectorId, &[f32])> = vectors
            .iter()
            .filter_map(|(chunk_id, embedding)| {
                VectorId::new(chunk_id.get()).map(|id| (id, embedding.as_ref()))
            })
            .collect();
        self.stage_vector_writes()?;
        self.vector_storage
            .as_mut()
            .expect("checked above")
            .write_batch(&vector_pairs)?;

        Ok(())
    }

    fn process_embedding_spool<F>(
        &mut self,
        spool: &mut tempfile::NamedTempFile,
        total_chunks: usize,
        on_progress: &mut F,
    ) -> StoreResult<()>
    where
        F: FnMut(IndexProgress<'_>),
    {
        spool.as_file_mut().flush()?;
        spool.as_file_mut().seek(SeekFrom::Start(0))?;
        let reader = BufReader::new(spool.reopen()?);
        let accelerated = crate::memory::accelerated_embeddings_requested();
        let mut processed = 0usize;
        let mut batch = Vec::new();
        let mut memory_sampler = crate::memory::MemorySampler::new();
        let mut memory = memory_sampler.sample();
        if memory.under_pressure() {
            return Err(DocumentStoreError::Embedding(format!(
                "embedding stopped before swap pressure (available={} MiB, rss={} MiB); \
                 metadata is safe and the run can be resumed",
                memory.available / (1024 * 1024),
                memory.process_rss / (1024 * 1024),
            )));
        }
        let mut target = memory.embedding_batch_size(EMBEDDING_BATCH_SIZE, accelerated);

        on_progress(IndexProgress::GeneratingEmbeddings {
            current: 0,
            total: total_chunks,
        });

        for line in reader.lines() {
            let line = line?;
            let (raw_id, text): (u32, String) = serde_json::from_str(&line).map_err(|e| {
                DocumentStoreError::Index(format!("Failed to read embedding spool: {e}"))
            })?;
            let id = ChunkId::from_u32(raw_id).ok_or_else(|| {
                DocumentStoreError::Index(format!("Invalid chunk ID in embedding spool: {raw_id}"))
            })?;
            batch.push((id, text));

            if batch.len() >= target {
                memory = memory_sampler.sample();
                if memory.under_pressure() {
                    return Err(DocumentStoreError::Embedding(format!(
                        "embedding stopped before swap pressure (available={} MiB, rss={} MiB); \
                         metadata is safe and the run can be resumed",
                        memory.available / (1024 * 1024),
                        memory.process_rss / (1024 * 1024),
                    )));
                }
                target = memory.embedding_batch_size(EMBEDDING_BATCH_SIZE, accelerated);
                let take = target.min(batch.len());
                self.process_embedding_batch(&batch[..take])?;
                batch.drain(..take);
                processed += take;
                on_progress(IndexProgress::GeneratingEmbeddings {
                    current: processed,
                    total: total_chunks,
                });
            }
        }
        while !batch.is_empty() {
            memory = memory_sampler.sample();
            if memory.under_pressure() {
                return Err(DocumentStoreError::Embedding(format!(
                    "embedding stopped before swap pressure (available={} MiB, rss={} MiB); \
                     metadata is safe and the run can be resumed",
                    memory.available / (1024 * 1024),
                    memory.process_rss / (1024 * 1024),
                )));
            }
            let take = memory
                .embedding_batch_size(EMBEDDING_BATCH_SIZE, accelerated)
                .min(batch.len());
            self.process_embedding_batch(&batch[..take])?;
            batch.drain(..take);
            processed += take;
            on_progress(IndexProgress::GeneratingEmbeddings {
                current: processed,
                total: total_chunks,
            });
        }
        on_progress(IndexProgress::Phase {
            name: "finalizing embeddings",
        });
        Ok(())
    }

    fn filter_clauses(&self, query: &SearchQuery) -> Vec<(Occur, Box<dyn Query>)> {
        let mut subqueries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        // Always filter for chunks (not metadata)
        let doc_type_term = Term::from_field_text(self.schema.doc_type, "chunk");
        subqueries.push((
            Occur::Must,
            Box::new(TermQuery::new(
                doc_type_term,
                tantivy::schema::IndexRecordOption::Basic,
            )),
        ));

        // Collection filter
        if let Some(ref collection) = query.collection {
            let term = Term::from_field_text(self.schema.collection_name, collection);
            subqueries.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    term,
                    tantivy::schema::IndexRecordOption::Basic,
                )),
            ));
        }

        // Document filter
        if let Some(ref doc_path) = query.document {
            let term =
                Term::from_field_text(self.schema.source_path, doc_path.to_string_lossy().as_ref());
            subqueries.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    term,
                    tantivy::schema::IndexRecordOption::Basic,
                )),
            ));
        }

        subqueries
    }

    fn get_filtered_candidates(&self, query: &SearchQuery) -> StoreResult<Vec<ChunkId>> {
        let searcher = self.reader.searcher();
        let filter_query = BooleanQuery::new(self.filter_clauses(query));

        // Enumerate the complete filtered set. A TopDocs limit here used to
        // silently exclude matching chunks from semantic ranking once a
        // collection crossed 10,000 chunks.
        let mut doc_addresses: Vec<_> = searcher
            .search(&filter_query, &DocSetCollector)?
            .into_iter()
            .collect();
        doc_addresses.sort_unstable();

        // Extract chunk IDs
        let mut chunk_ids = Vec::with_capacity(doc_addresses.len());
        for doc_address in doc_addresses {
            let chunk_ids_column = searcher
                .segment_reader(doc_address.segment_ord)
                .fast_fields()
                .u64("chunk_id")?;
            if let Some(chunk_id) = chunk_ids_column
                .first(doc_address.doc_id)
                .and_then(|id| ChunkId::from_u32(id as u32))
            {
                chunk_ids.push(chunk_id);
            }
        }

        Ok(chunk_ids)
    }

    fn score_by_similarity(
        &mut self,
        candidates: &[ChunkId],
        query_vec: &[f32],
    ) -> StoreResult<Vec<(ChunkId, f32)>> {
        let Some(ref mut vector_storage) = self.vector_storage else {
            // No vectors, return with zero scores
            return Ok(candidates.iter().map(|&id| (id, 0.0)).collect());
        };

        let ids: std::collections::HashSet<_> = candidates
            .iter()
            .filter_map(|chunk_id| VectorId::new(chunk_id.get()))
            .collect();
        let scored = vector_storage.score_vectors(&ids, query_vec)?;
        Ok(scored
            .into_iter()
            .filter_map(|(id, score)| ChunkId::from_u32(id.get()).map(|chunk| (chunk, score)))
            .collect())
    }

    fn build_search_results(
        &self,
        scored: Vec<(ChunkId, f32)>,
        query: &SearchQuery,
    ) -> StoreResult<Vec<SearchResult>> {
        let searcher = self.reader.searcher();
        let mut results = Vec::new();

        if scored.is_empty() {
            return Ok(results);
        }

        // Hydrate all winners with one term-set query against this pinned
        // reader generation instead of issuing one Tantivy query per result.
        let chunk_query = TermSetQuery::new(scored.iter().map(|(chunk_id, _)| {
            Term::from_field_u64(self.schema.chunk_id, chunk_id.get() as u64)
        }));
        let mut doc_addresses: Vec<_> = searcher
            .search(&chunk_query, &DocSetCollector)?
            .into_iter()
            .collect();
        doc_addresses.sort_unstable();
        let mut documents = HashMap::with_capacity(doc_addresses.len());
        for doc_address in doc_addresses {
            let doc: Document = searcher.doc(doc_address)?;
            if let Some(chunk_id) = doc
                .get_first(self.schema.chunk_id)
                .and_then(|value| value.as_u64())
                .and_then(|id| ChunkId::from_u32(id as u32))
            {
                documents.insert(chunk_id, doc);
            }
        }

        // Get preview config (use defaults if not provided)
        let default_config = super::config::SearchConfig::default();
        let preview_config = query.preview_config.as_ref().unwrap_or(&default_config);

        for (chunk_id, similarity) in scored {
            if let Some(doc) = documents.get(&chunk_id) {
                let collection = doc
                    .get_first(self.schema.collection_name)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let source_path = doc
                    .get_first(self.schema.source_path)
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_default();

                if let Some(root) = &self.workspace_root {
                    if source_path.as_os_str().is_empty() {
                        return Err(DocumentStoreError::Index(
                            "Document result has no source provenance".into(),
                        ));
                    }
                    crate::indexing::facade::IndexFacade::contained_source(root, &source_path)
                        .map_err(|error| DocumentStoreError::Index(error.to_string()))?;
                }

                let heading_json = doc
                    .get_first(self.schema.heading_context)
                    .and_then(|v| v.as_str())
                    .unwrap_or("[]");

                let heading_context: Vec<String> =
                    serde_json::from_str(heading_json).unwrap_or_default();

                // Get full content for KWIC extraction
                let full_content = doc
                    .get_first(self.schema.content)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Generate preview with KWIC and highlighting
                let content_preview = generate_preview(full_content, &query.text, preview_config);

                let byte_start = doc
                    .get_first(self.schema.byte_start)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;

                let byte_end = doc
                    .get_first(self.schema.byte_end)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;

                results.push(SearchResult {
                    chunk_id,
                    collection,
                    source_path,
                    heading_context,
                    content_preview,
                    byte_range: (byte_start, byte_end),
                    similarity,
                });
            }
        }

        Ok(results)
    }

    fn load_state(path: &Path) -> StoreResult<PersistedState> {
        let content = std::fs::read_to_string(path)?;
        let state: PersistedState = serde_json::from_str(&content)
            .map_err(|e| DocumentStoreError::Index(format!("Failed to parse state: {e}")))?;

        Ok(state)
    }

    fn save_state(&self) -> StoreResult<()> {
        let state = PersistedState {
            file_states: self
                .file_states
                .iter()
                .map(|(k, v)| (k.to_string_lossy().to_string(), v.clone()))
                .collect(),
            collection_ids: self
                .collection_ids
                .iter()
                .map(|(name, id)| (name.clone(), id.get()))
                .collect(),
            next_chunk_id: self.next_chunk_id,
            chunking_fingerprints: self
                .chunking_fingerprints
                .iter()
                .map(|(path, value)| (path.to_string_lossy().to_string(), value.clone()))
                .collect(),
            embedded_files: self
                .embedded_files
                .iter()
                .map(|(path, value)| (path.to_string_lossy().to_string(), value.clone()))
                .collect(),
            embedding_identity: self.embedding_identity.clone(),
        };

        let content = serde_json::to_string_pretty(&state)
            .map_err(|e| DocumentStoreError::Index(format!("Failed to serialize state: {e}")))?;

        let state_path = self.base_path.join("state.json");
        let mut staging = tempfile::NamedTempFile::new_in(&self.base_path)?;
        staging.write_all(content.as_bytes())?;
        staging.flush()?;
        staging.as_file().sync_all()?;
        staging.persist(state_path).map_err(|error| error.error)?;

        if let Some(cache) = &self.embedding_cache {
            if let Err(error) = cache.save(&self.base_path.join("embedding-cache.json")) {
                tracing::warn!(target: "embedding_cache", %error, "failed to persist document embedding cache");
            }
        }

        Ok(())
    }

    fn load_cluster_data(&mut self) -> StoreResult<()> {
        let cluster_path = self.base_path.join("clusters.json");

        if !cluster_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(cluster_path)?;
        let data: ClusterData = serde_json::from_str(&content)
            .map_err(|e| DocumentStoreError::Index(format!("Failed to parse clusters: {e}")))?;

        self.centroids = data.centroids;
        self.cluster_assignments = data
            .assignments
            .into_iter()
            .filter_map(|(id, cluster)| {
                let vid = VectorId::new(id)?;
                let cid = ClusterId::new(cluster)?;
                Some((vid, cid))
            })
            .collect();

        Ok(())
    }
}

/// Normalize existing sources and deleted children beneath a canonical parent.
/// This preserves ownership across platform aliases such as /var and /private/var.
pub(crate) fn normalize_source_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    for ancestor in path.ancestors().skip(1) {
        if let Ok(canonical) = ancestor.canonicalize() {
            if let Ok(suffix) = path.strip_prefix(ancestor) {
                return canonical.join(suffix);
            }
        }
    }
    path.to_path_buf()
}

fn chunking_fingerprint(config: &ValidatedChunkingConfig) -> StoreResult<String> {
    let encoded = serde_json::to_string(&**config)
        .map_err(|error| DocumentStoreError::InvalidChunkingConfig(error.to_string()))?;
    Ok(calculate_hash(&format!(
        "hybrid-source-spans-v2\n{encoded}"
    )))
}

fn embedding_input(chunk: &RawChunk) -> String {
    if chunk.heading_context.is_empty() {
        chunk.content.clone()
    } else {
        format!("{}\n\n{}", chunk.heading_context.join(" > "), chunk.content)
    }
}

fn retain_top_chunks(scored: &mut Vec<(ChunkId, f32)>, limit: usize) {
    if limit == 0 {
        scored.clear();
        return;
    }
    if scored.len() > limit {
        scored.select_nth_unstable_by(limit, |a, b| b.1.total_cmp(&a.1));
        scored.truncate(limit);
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
}

/// Statistics about a collection.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CollectionStats {
    /// Collection name.
    pub name: String,
    /// Number of chunks indexed.
    pub chunk_count: usize,
    /// Number of files indexed.
    pub file_count: usize,
}

/// Persisted state for the document store.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct PersistedState {
    file_states: HashMap<String, FileState>,
    collection_ids: HashMap<String, u32>,
    next_chunk_id: u64,
    #[serde(default)]
    chunking_fingerprints: HashMap<String, String>,
    #[serde(default)]
    embedded_files: HashMap<String, String>,
    #[serde(default)]
    embedding_identity: Option<String>,
}

/// Persisted cluster data.
#[derive(serde::Serialize, serde::Deserialize)]
struct ClusterData {
    centroids: Vec<Vec<f32>>,
    assignments: HashMap<u32, u32>,
}

/// Read-only query ownership, independent of the mutable document writer.
pub(crate) struct DocumentQuery(DocumentStore);
impl DocumentQuery {
    pub(crate) fn search(&mut self, query: SearchQuery) -> StoreResult<Vec<SearchResult>> {
        if self.0.embedding_generator.is_some() {
            self.0.vector_storage = Some(MmapVectorStorage::open(
                self.0.base_path.join("vectors"),
                SegmentOrdinal::new(0),
            )?);
        }
        self.0.search(query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct CountingGenerator {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        dimension: VectorDimension,
    }

    impl EmbeddingGenerator for CountingGenerator {
        fn generate_embeddings(
            &self,
            texts: &[&str],
        ) -> Result<Vec<Vec<f32>>, crate::vector::VectorError> {
            self.calls
                .fetch_add(texts.len(), std::sync::atomic::Ordering::Relaxed);
            Ok(texts
                .iter()
                .map(|_| vec![0.5; self.dimension.get()])
                .collect())
        }

        fn dimension(&self) -> VectorDimension {
            self.dimension
        }

        fn cache_identity(&self) -> String {
            "counting-fixture@1".to_string()
        }
    }

    fn test_dimension() -> VectorDimension {
        VectorDimension::new(4).unwrap()
    }

    #[test]
    fn top_chunk_selection_partitions_before_sorting() {
        let mut scored: Vec<_> = (1..=100)
            .map(|id| (ChunkId::from_u32(id).unwrap(), id as f32))
            .collect();
        retain_top_chunks(&mut scored, 3);

        assert_eq!(scored.len(), 3);
        assert_eq!(scored[0].0.get(), 100);
        assert_eq!(scored[1].0.get(), 99);
        assert_eq!(scored[2].0.get(), 98);
    }

    #[test]
    fn filtered_candidates_are_not_capped_at_ten_thousand() {
        let temp = TempDir::new().unwrap();
        let mut store = DocumentStore::new(temp.path(), test_dimension()).unwrap();
        let chunk = RawChunk::new((0, 4), "body".to_string(), Vec::new());
        let path = Path::new("large.md");

        for raw_id in 1..=10_001 {
            store
                .store_chunk(
                    ChunkId::from_u32(raw_id).unwrap(),
                    "large",
                    path,
                    &chunk,
                    "body",
                )
                .unwrap();
        }
        store.commit().unwrap();

        let candidates = store
            .get_filtered_candidates(&SearchQuery {
                text: "body".to_string(),
                collection: Some("large".to_string()),
                ..SearchQuery::default()
            })
            .unwrap();
        assert_eq!(candidates.len(), 10_001);
    }

    #[test]
    fn lexical_search_finds_relevant_chunk_after_ten_thousand_nonmatches() {
        let temp = TempDir::new().unwrap();
        let mut store = DocumentStore::new(temp.path(), test_dimension()).unwrap();
        let ordinary = RawChunk::new((0, 4), "body".to_string(), Vec::new());
        let relevant = RawChunk::new((0, 12), "rarechecksum".to_string(), Vec::new());
        for raw_id in 1..=10_001 {
            let chunk = if raw_id == 10_001 {
                &relevant
            } else {
                &ordinary
            };
            store
                .store_chunk(
                    ChunkId::from_u32(raw_id).unwrap(),
                    "large",
                    Path::new("large.md"),
                    chunk,
                    &chunk.content,
                )
                .unwrap();
        }
        store.commit().unwrap();
        let hits = store
            .search(SearchQuery {
                text: "rarechecksum".into(),
                collection: Some("large".into()),
                limit: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk_id.get(), 10_001);
    }

    #[test]
    fn test_document_store_creation() {
        let temp_dir = TempDir::new().unwrap();
        let store = DocumentStore::new(temp_dir.path(), test_dimension());
        assert!(store.is_ok());
    }

    #[test]
    fn document_embedding_cache_reuses_content_after_reopen() {
        let temp = TempDir::new().unwrap();
        let dimension = test_dimension();
        let first_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let generator = CountingGenerator {
            calls: Arc::clone(&first_calls),
            dimension,
        };
        let mut store = DocumentStore::new(temp.path(), dimension)
            .unwrap()
            .with_embeddings(Box::new(generator))
            .unwrap();
        store
            .process_embedding_batch(&[(
                ChunkId::from_u32(1).unwrap(),
                "unchanged chunk".to_string(),
            )])
            .unwrap();
        store.save_state().unwrap();
        assert_eq!(first_calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        drop(store);

        let reopened_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let generator = CountingGenerator {
            calls: Arc::clone(&reopened_calls),
            dimension,
        };
        let mut reopened = DocumentStore::new(temp.path(), dimension)
            .unwrap()
            .with_embeddings(Box::new(generator))
            .unwrap();
        reopened
            .process_embedding_batch(&[(
                ChunkId::from_u32(2).unwrap(),
                "unchanged chunk".to_string(),
            )])
            .unwrap();
        assert_eq!(reopened_calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn test_embedding_progress_includes_initial_and_final_counts() {
        let temp = TempDir::new().unwrap();
        let dimension = VectorDimension::new(8).unwrap();
        let generator = crate::vector::MockEmbeddingGenerator::with_dimension(dimension);
        let mut store = DocumentStore::new(temp.path(), dimension)
            .unwrap()
            .with_embeddings(Box::new(generator))
            .unwrap();
        let chunks: Vec<_> = (1..=65)
            .map(|id| {
                let text = ["parse", "json", "error", "async"]
                    .iter()
                    .enumerate()
                    .filter(|(bit, _)| id & (1 << bit) != 0)
                    .map(|(_, word)| *word)
                    .collect::<Vec<_>>()
                    .join(" ");
                (ChunkId::from_u32(id).unwrap(), text)
            })
            .collect();
        let mut counts = Vec::new();
        let mut phases = Vec::new();
        store
            .process_embeddings_batched(&chunks, &mut |event| match event {
                IndexProgress::GeneratingEmbeddings { current, total } => {
                    counts.push((current, total))
                }
                IndexProgress::Phase { name } => phases.push(name),
                _ => {}
            })
            .unwrap();
        assert_eq!(counts.first(), Some(&(0, 65)));
        assert_eq!(counts.last(), Some(&(65, 65)));
        assert!(counts.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(phases, vec!["finalizing embeddings"]);
    }

    #[test]
    fn collection_embeddings_stream_through_disk_spool() {
        use crate::documents::config::{ChunkingConfig, CollectionConfig};

        let store_dir = TempDir::new().unwrap();
        let source_dir = TempDir::new().unwrap();
        let body = "# Memory-safe indexing\n\n".to_string()
            + &"This prepared fixture is long enough to create a document chunk. ".repeat(8);
        std::fs::write(source_dir.path().join("guide.md"), body).unwrap();

        let dimension = VectorDimension::new(8).unwrap();
        let generator = crate::vector::MockEmbeddingGenerator::with_dimension(dimension);
        let mut store = DocumentStore::new(store_dir.path(), dimension)
            .unwrap()
            .with_embeddings(Box::new(generator))
            .unwrap();
        let config = CollectionConfig {
            paths: vec![source_dir.path().to_path_buf()],
            patterns: vec!["**/*.md".to_string()],
            ..Default::default()
        };

        let stats = store
            .index_collection("guides", &config, &ChunkingConfig::default())
            .unwrap();

        assert!(stats.chunks_created > 0);
        assert_eq!(
            store.vector_storage.as_ref().unwrap().vector_count(),
            stats.chunks_created
        );
    }

    #[test]
    fn test_collect_files_uses_codannaignore_not_gitignore() {
        let store_dir = TempDir::new().unwrap();
        let source_dir = TempDir::new().unwrap();
        let store = DocumentStore::new(store_dir.path(), test_dimension()).unwrap();

        std::fs::write(source_dir.path().join(".gitignore"), "node_modules/\n").unwrap();
        std::fs::write(source_dir.path().join(".codannaignore"), "vendor/\n").unwrap();
        std::fs::write(source_dir.path().join("README.md"), "included").unwrap();
        std::fs::create_dir_all(source_dir.path().join(".agents")).unwrap();
        std::fs::write(
            source_dir.path().join(".agents/guide.md"),
            "included hidden doc",
        )
        .unwrap();
        std::fs::create_dir_all(source_dir.path().join("vendor/pkg")).unwrap();
        std::fs::write(
            source_dir.path().join("vendor/pkg/README.md"),
            "ignored vendored doc",
        )
        .unwrap();
        std::fs::create_dir_all(source_dir.path().join("node_modules/pkg")).unwrap();
        std::fs::write(
            source_dir.path().join("node_modules/pkg/README.md"),
            "ignored dependency doc",
        )
        .unwrap();

        let config = CollectionConfig {
            paths: vec![source_dir.path().to_path_buf()],
            patterns: vec!["**/*.md".to_string()],
            ..Default::default()
        };

        let mut files = store.collect_files(&config).unwrap();
        files.sort();

        assert_eq!(files.len(), 3);
        assert!(files.contains(&source_dir.path().join("README.md")));
        assert!(files.contains(&source_dir.path().join(".agents/guide.md")));
        assert!(files.contains(&source_dir.path().join("node_modules/pkg/README.md")));
        assert!(!files.contains(&source_dir.path().join("vendor/pkg/README.md")));
    }

    #[test]
    fn test_collection_id_allocation() {
        let temp_dir = TempDir::new().unwrap();
        let mut store = DocumentStore::new(temp_dir.path(), test_dimension()).unwrap();

        let id1 = store.get_or_create_collection_id("test-collection");
        let id2 = store.get_or_create_collection_id("test-collection");
        let id3 = store.get_or_create_collection_id("another-collection");

        // Same name should return same ID
        assert_eq!(id1.get(), id2.get());

        // Different name should return different ID
        assert_ne!(id1.get(), id3.get());
    }

    #[test]
    fn test_chunk_id_allocation() {
        let temp_dir = TempDir::new().unwrap();
        let mut store = DocumentStore::new(temp_dir.path(), test_dimension()).unwrap();

        let id1 = store.allocate_chunk_id();
        let id2 = store.allocate_chunk_id();
        let id3 = store.allocate_chunk_id();

        // IDs should be unique and sequential
        assert_ne!(id1.get(), id2.get());
        assert_ne!(id2.get(), id3.get());
        assert_eq!(id2.get(), id1.get() + 1);
        assert_eq!(id3.get(), id2.get() + 1);
    }

    #[test]
    fn test_state_persistence() {
        let temp_dir = TempDir::new().unwrap();

        // Create store and allocate some IDs
        {
            let mut store = DocumentStore::new(temp_dir.path(), test_dimension()).unwrap();
            store.get_or_create_collection_id("persist-test");
            let _id1 = store.allocate_chunk_id();
            let _id2 = store.allocate_chunk_id();
            store.save_state().unwrap();
        }

        // Reopen and verify state
        {
            let store = DocumentStore::new(temp_dir.path(), test_dimension()).unwrap();
            assert!(store.collection_ids.contains_key("persist-test"));
            assert!(store.next_chunk_id > 2);
        }
    }

    #[test]
    fn test_list_collections() {
        let temp_dir = TempDir::new().unwrap();
        let mut store = DocumentStore::new(temp_dir.path(), test_dimension()).unwrap();

        store.get_or_create_collection_id("alpha");
        store.get_or_create_collection_id("beta");
        store.get_or_create_collection_id("gamma");

        let collections = store.list_collections();
        assert_eq!(collections.len(), 3);
        assert!(collections.contains(&"alpha".to_string()));
        assert!(collections.contains(&"beta".to_string()));
        assert!(collections.contains(&"gamma".to_string()));
    }

    /// Regression for issue #100: indexing collection B must not wipe
    /// chunks belonging to collection A.
    #[test]
    fn test_index_collection_does_not_wipe_other_collections() {
        use crate::documents::config::{ChunkingConfig, CollectionConfig};

        let store_dir = TempDir::new().unwrap();
        let alpha_dir = TempDir::new().unwrap();
        let beta_dir = TempDir::new().unwrap();

        // Write source markdown for two disjoint collections. Body length is
        // padded above the 200-char min_chunk_chars default so each file
        // produces at least one chunk.
        let body = "# Heading\n\n".to_string()
            + &"This is a sentence used to pad the chunk above the minimum size threshold. "
                .repeat(8);
        std::fs::write(alpha_dir.path().join("a1.md"), &body).unwrap();
        std::fs::write(alpha_dir.path().join("a2.md"), &body).unwrap();
        std::fs::write(beta_dir.path().join("b1.md"), &body).unwrap();
        std::fs::write(beta_dir.path().join("b2.md"), &body).unwrap();

        let alpha_cfg = CollectionConfig {
            paths: vec![alpha_dir.path().to_path_buf()],
            patterns: vec!["**/*.md".to_string()],
            ..Default::default()
        };
        let beta_cfg = CollectionConfig {
            paths: vec![beta_dir.path().to_path_buf()],
            patterns: vec!["**/*.md".to_string()],
            ..Default::default()
        };
        let chunking = ChunkingConfig::default();

        let mut store = DocumentStore::new(store_dir.path(), test_dimension()).unwrap();

        let alpha_stats = store
            .index_collection("alpha", &alpha_cfg, &chunking)
            .unwrap();
        assert!(alpha_stats.files_processed >= 2);
        assert!(alpha_stats.chunks_created >= 2);
        assert_eq!(alpha_stats.chunks_removed, 0);

        let alpha_count_before = store.collection_stats("alpha").unwrap().chunk_count;
        assert!(alpha_count_before >= 2);

        // Indexing beta on a fresh store (only alpha pre-loaded) must not
        // touch alpha's chunks. Pre-fix: chunks_removed == alpha_count_before.
        let beta_stats = store
            .index_collection("beta", &beta_cfg, &chunking)
            .unwrap();
        assert!(beta_stats.files_processed >= 2);
        assert!(beta_stats.chunks_created >= 2);
        assert_eq!(
            beta_stats.chunks_removed, 0,
            "indexing beta wiped {} alpha chunks (issue #100)",
            beta_stats.chunks_removed
        );

        let alpha_count_after = store.collection_stats("alpha").unwrap().chunk_count;
        let beta_count_after = store.collection_stats("beta").unwrap().chunk_count;
        assert_eq!(alpha_count_after, alpha_count_before, "alpha chunks lost");
        assert!(beta_count_after >= 2, "beta chunks not persisted");
    }

    #[test]
    fn test_collection_stats_file_count_is_scoped() {
        use crate::documents::config::{ChunkingConfig, CollectionConfig};

        let store_dir = TempDir::new().unwrap();
        let alpha_dir = TempDir::new().unwrap();
        let beta_dir = TempDir::new().unwrap();

        let body = "# Heading\n\n".to_string()
            + &"This is a sentence used to pad the chunk above the minimum size threshold. "
                .repeat(8);
        std::fs::write(alpha_dir.path().join("a1.md"), &body).unwrap();
        std::fs::write(alpha_dir.path().join("a2.md"), &body).unwrap();
        std::fs::write(beta_dir.path().join("b1.md"), &body).unwrap();
        std::fs::write(beta_dir.path().join("b2.md"), &body).unwrap();

        let alpha_cfg = CollectionConfig {
            paths: vec![alpha_dir.path().to_path_buf()],
            patterns: vec!["**/*.md".to_string()],
            ..Default::default()
        };
        let beta_cfg = CollectionConfig {
            paths: vec![beta_dir.path().to_path_buf()],
            patterns: vec!["**/*.md".to_string()],
            ..Default::default()
        };
        let chunking = ChunkingConfig::default();

        let mut store = DocumentStore::new(store_dir.path(), test_dimension()).unwrap();
        store
            .index_collection("alpha", &alpha_cfg, &chunking)
            .unwrap();
        store
            .index_collection("beta", &beta_cfg, &chunking)
            .unwrap();

        let alpha = store.collection_stats("alpha").unwrap();
        let beta = store.collection_stats("beta").unwrap();
        assert_eq!(alpha.file_count, 2, "alpha file_count not scoped");
        assert_eq!(beta.file_count, 2, "beta file_count not scoped");
    }
    #[test]
    fn hardening_review_invalid_chunking_never_mutates_collection_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.md");
        std::fs::write(&path, "A document that must remain indexed.").unwrap();
        let mut store = DocumentStore::new(
            temp.path().join("index"),
            VectorDimension::new(384).unwrap(),
        )
        .unwrap();
        let collection = CollectionConfig {
            paths: vec![path.clone()],
            ..Default::default()
        };
        store
            .index_collection("keep", &collection, &ChunkingConfig::default())
            .unwrap();
        let ids = store.file_states.get(&path).unwrap().chunk_ids.clone();
        let bad = ChunkingConfig {
            max_chunk_chars: 0,
            ..Default::default()
        };
        assert!(matches!(
            store.index_collection("new", &collection, &bad),
            Err(DocumentStoreError::InvalidChunkingConfig(_))
        ));
        assert!(matches!(
            store.reindex_file(&path, &bad),
            Err(DocumentStoreError::InvalidChunkingConfig(_))
        ));
        assert_eq!(store.file_states.get(&path).unwrap().chunk_ids, ids);
        assert!(!store.collection_ids.contains_key("new"));
    }

    #[test]
    fn hardening_review_delete_collection_preserves_other_file_states() {
        let dir = TempDir::new().unwrap();
        let docs = TempDir::new().unwrap();
        let alpha = docs.path().join("alpha.md");
        let beta = docs.path().join("beta.md");
        let body = "# Heading\n\n".to_string() + &"Unique document text. ".repeat(30);
        std::fs::write(&alpha, &body).unwrap();
        std::fs::write(&beta, &body).unwrap();
        let mut store = DocumentStore::new(dir.path(), test_dimension()).unwrap();
        let config = |path| CollectionConfig {
            paths: vec![path],
            ..Default::default()
        };
        let a = config(alpha.clone());
        let b = config(beta.clone());
        store
            .index_collection("alpha", &a, &ChunkingConfig::default())
            .unwrap();
        store
            .index_collection("beta", &b, &ChunkingConfig::default())
            .unwrap();
        let before = store.collection_stats("beta").unwrap().chunk_count;
        assert!(before > 0);
        assert!(store.delete_collection("alpha").unwrap() > 0);
        assert!(!store.file_states.contains_key(&alpha));
        assert!(store.file_states.contains_key(&beta));
        drop(store);
        let mut reopened = DocumentStore::new(dir.path(), test_dimension()).unwrap();
        assert!(reopened.file_states.contains_key(&beta));
        let stats = reopened
            .index_collection("beta", &b, &ChunkingConfig::default())
            .unwrap();
        assert_eq!(stats.files_skipped, 1);
        assert_eq!(stats.chunks_created, 0);
        assert_eq!(
            reopened.collection_stats("beta").unwrap().chunk_count,
            before
        );
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;

    #[test]
    fn hardening_workspace_document_snapshots_reject_foreign_hits_after_binding() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("one");
        let foreign = temp.path().join("two");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(root.join("guide.md"), "workspace_topic ".repeat(30)).unwrap();
        std::fs::write(
            foreign.join("guide.md"),
            "PRIVATE_WORKSPACE_TOPIC ".repeat(30),
        )
        .unwrap();
        let config = |path: &Path| CollectionConfig {
            paths: vec![path.to_path_buf()],
            ..CollectionConfig::default()
        };
        let mut writer =
            DocumentStore::new(root.join("index"), VectorDimension::new(4).unwrap()).unwrap();
        writer
            .index_collection("local", &config(&root), &ChunkingConfig::default())
            .unwrap();
        writer.restrict_workspace(&root).unwrap();
        let mut snapshot = writer.query_snapshot();
        // Simulate a later external writer that does not observe this reader's
        // in-memory scope. A shallow query snapshot must retain its boundary.
        writer
            .index_collection("foreign", &config(&foreign), &ChunkingConfig::default())
            .unwrap();
        assert!(
            snapshot
                .search(SearchQuery {
                    text: "PRIVATE_WORKSPACE_TOPIC".into(),
                    ..SearchQuery::default()
                })
                .is_err()
        );
        let mut reopened =
            DocumentStore::new(root.join("index"), VectorDimension::new(4).unwrap()).unwrap();
        assert!(reopened.restrict_workspace(&root).is_err());
    }
}
