//! Document chunking and embedding for RAG use cases.
//!
//! This module provides:
//! - Document chunking with configurable strategies
//! - Vector embeddings for document chunks
//! - Collection-based organization and filtering
//! - Semantic search within document collections

pub mod chunker;
pub mod config;
mod embedding;
mod generation;
mod ranking;
pub mod schema;
pub mod status;
pub mod store;
pub mod types;

pub use chunker::{Chunker, HybridChunker, RawChunk};
pub use config::{
    ChunkingConfig, ChunkingStrategy, CollectionConfig, DocumentsConfig, PreviewMode, SearchConfig,
    ValidatedChunkingConfig,
};
pub use schema::DocumentSchema;
pub use store::{
    CollectionStats, DocumentStore, EmbeddingDiagnostics, IndexProgress, SearchQuery, SearchResult,
};
pub use types::{ChunkId, CollectionId, DocumentChunk, FileState};

use crate::config::Settings;
use crate::vector::{EmbeddingGenerator, VectorDimension};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Open a store with the same enabled flag, model, dimensions and backend for
/// CLI indexing, CLI search and MCP. Disabled embeddings never load a model.
pub fn open_from_settings(settings: &Settings) -> store::StoreResult<DocumentStore> {
    let path = settings.index_path.join("documents");
    if !settings.semantic_search.enabled {
        return DocumentStore::new(path, VectorDimension::dimension_384())
            .map(|store| store.with_source_exclusion(&settings.index_path));
    }
    let generator = embedding::ConfiguredGenerator::new(&settings.semantic_search)
        .map_err(|error| store::DocumentStoreError::Embedding(error.to_string()))?;
    DocumentStore::new(path, generator.dimension())?
        .with_source_exclusion(&settings.index_path)
        .with_embeddings(Box::new(generator))
}

/// Load document store from settings if enabled and indexed.
///
/// Returns None if documents are disabled, index doesn't exist, or loading fails.
/// The returned Arc can be shared between MCP server and file watcher.
pub fn load_from_settings(settings: &Settings) -> Option<Arc<RwLock<DocumentStore>>> {
    if !settings.documents.enabled {
        tracing::debug!(target: "documents", "document store disabled in settings");
        return None;
    }

    let doc_path = settings.index_path.join("documents");
    if !doc_path.exists() {
        tracing::debug!(target: "documents", "document index not found at {}", crate::parsing::paths::render_absolute_path(&doc_path).display());
        return None;
    }

    let store = match open_from_settings(settings) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "documents", "failed to open document store: {e}");
            return None;
        }
    };

    tracing::info!(target: "documents", "loaded document store from {}", crate::parsing::paths::render_absolute_path(&doc_path).display());
    Some(Arc::new(RwLock::new(store)))
}
