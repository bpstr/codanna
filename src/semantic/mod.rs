//! Semantic search functionality for documentation comments
//!
//! This module provides a simple API for semantic search on documentation,
//! designed to integrate with the existing indexing system.

mod code_dimension;
mod journal;
mod metadata;
mod pool;
pub(crate) mod remote;
mod simple;
mod storage;

pub(crate) use code_dimension::configured_code_dimension;
pub use code_dimension::{
    MAX_CODE_EMBEDDING_DIMENSION, build_code_embedding_backend, validate_code_embedding_dimension,
};
pub use metadata::{EmbeddingBackendKind, SemanticMetadata};
pub use pool::{EmbeddingBackend, EmbeddingPool};
pub use remote::RemoteEmbedder;
pub(crate) use simple::SymbolSegment;
pub use simple::{SemanticSearchError, SimpleSemanticSearch};
pub use storage::SemanticVectorStorage;

// Re-export key types
pub use fastembed::{EmbeddingModel, TextEmbedding};

/// Similarity threshold recommendations based on testing
pub mod thresholds {
    /// Threshold for very similar documents (e.g., same concept, different wording)
    pub const VERY_SIMILAR: f32 = 0.75;

    /// Threshold for similar documents (e.g., related concepts)
    pub const SIMILAR: f32 = 0.60;

    /// Threshold for somewhat related documents
    pub const RELATED: f32 = 0.40;

    /// Default threshold for semantic search
    pub const DEFAULT: f32 = SIMILAR;
}

#[cfg(test)]
mod symbol_representation_tests;
