//! Semantic embedding stage - parallel embedding generation
//!
//! COLLECT ─┬─> EMBED (this stage) ─> SimpleSemanticSearch
//!          └─> INDEX ─> Tantivy
//!
//! Receives EmbeddingBatch from COLLECT, generates embeddings using EmbeddingBackend,
//! stores them in SimpleSemanticSearch. Runs in parallel with INDEX stage.

use crate::indexing::pipeline::types::{EmbeddingBatch, PipelineError, PipelineResult};
use crate::semantic::{EmbeddingBackend, SimpleSemanticSearch};
use crossbeam_channel::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Statistics from the EMBED stage.
#[derive(Debug, Clone, Default)]
pub struct SemanticEmbedStats {
    /// Total candidates received
    pub received: usize,
    /// Successfully embedded
    pub embedded: usize,
    /// Skipped (empty doc, dimension mismatch)
    pub skipped: usize,
    /// Time waiting on input channel
    pub input_wait: Duration,
    /// Total processing time
    pub elapsed: Duration,
}

impl SemanticEmbedStats {
    /// Check if all received candidates were processed.
    pub fn is_complete(&self) -> bool {
        self.embedded + self.skipped == self.received
    }
}

/// Progress callback type for EMBED stage.
pub type EmbedProgressCallback = Arc<dyn Fn(u64) + Send + Sync>;

/// Semantic embedding stage — dispatches to local fastembed pool or remote HTTP backend.
///
/// Receives EmbeddingBatch from COLLECT, generates embeddings in parallel,
/// stores them in SimpleSemanticSearch.
pub struct SemanticEmbedStage {
    pool: Arc<EmbeddingBackend>,
    semantic: Arc<Mutex<SimpleSemanticSearch>>,
    progress_callback: Option<EmbedProgressCallback>,
}

impl SemanticEmbedStage {
    /// Create a new semantic embed stage.
    pub fn new(pool: Arc<EmbeddingBackend>, semantic: Arc<Mutex<SimpleSemanticSearch>>) -> Self {
        Self {
            pool,
            semantic,
            progress_callback: None,
        }
    }

    /// Add a progress callback that receives the count of embeddings processed per batch.
    pub fn with_progress(mut self, callback: EmbedProgressCallback) -> Self {
        self.progress_callback = Some(callback);
        self
    }

    /// Run the embed stage.
    ///
    /// Receives EmbeddingBatch from channel, generates embeddings, stores them.
    /// Runs until channel is closed.
    pub fn run(&self, receiver: Receiver<EmbeddingBatch>) -> PipelineResult<SemanticEmbedStats> {
        const STATS_LOG_INTERVAL: Duration = Duration::from_secs(10);

        tracing::info!(target: "semantic", "EMBED stage started, waiting for batches...");

        let start = Instant::now();
        let mut stats = SemanticEmbedStats::default();
        let mut last_stats_log = Instant::now();
        let mut batches_received = 0usize;

        loop {
            let recv_start = Instant::now();
            match receiver.recv() {
                Ok(batch) => {
                    batches_received += 1;
                    stats.input_wait += recv_start.elapsed();

                    let candidate_count = batch.len();
                    stats.received += candidate_count;

                    tracing::debug!(
                        target: "semantic",
                        "EMBED batch {}: {} candidates",
                        batches_received,
                        candidate_count
                    );

                    if !batch.is_empty() {
                        let count = self.process_batch(&batch)?;
                        stats.embedded += count;
                        stats.skipped += candidate_count - count;

                        // Report progress
                        if let Some(ref callback) = self.progress_callback {
                            callback(count as u64);
                        }

                        // Log pool stats periodically
                        if last_stats_log.elapsed() >= STATS_LOG_INTERVAL {
                            self.pool.log_usage_stats();
                            last_stats_log = Instant::now();
                        }
                    }
                }
                Err(_) => break, // Channel closed
            }
        }

        // Log final pool stats
        self.pool.log_usage_stats();

        stats.elapsed = start.elapsed();

        tracing::info!(
            target: "semantic",
            "EMBED complete: {}/{} embedded in {:?} ({} batches)",
            stats.embedded,
            stats.received,
            stats.elapsed,
            batches_received
        );

        Ok(stats)
    }

    /// Process a batch of embedding candidates.
    pub(crate) fn process_batch(&self, batch: &EmbeddingBatch) -> PipelineResult<usize> {
        let accelerated = crate::memory::accelerated_embeddings_requested();
        // Pin only existing body-input hits before any admission can evict them.
        let body_hits = self.body_cache_hits(batch)?;

        // Probe the whole collector batch before admitting any newly generated
        // vector into the bounded cache. Otherwise early misses can evict later
        // compatible hits before those hits are looked up (the production
        // collector batch is 5,000 symbols, larger than the default cache).
        let items: Vec<_> = batch
            .candidates
            .iter()
            .map(|(id, doc, lang)| (*id, doc.as_ref(), lang.as_ref()))
            .collect();
        let missing = {
            let mut semantic = self.semantic.lock().map_err(|_| PipelineError::Parse {
                path: std::path::PathBuf::new(),
                reason: "Failed to lock semantic search".to_string(),
            })?;
            semantic.reuse_cached_embeddings(&items)
        };
        let mut stored = items.len() - missing.len();

        // Collapse exact duplicate misses before provider/local-model inference.
        // Preserve first-seen order so request ordering stays deterministic.
        let mut group_indexes: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        let mut groups: Vec<(&str, Vec<(crate::SymbolId, &str)>)> = Vec::new();
        for &(id, text, language) in &missing {
            if let Some(&index) = group_indexes.get(text) {
                groups[index].1.push((id, language));
            } else {
                let index = groups.len();
                group_indexes.insert(text, index);
                groups.push((text, vec![(id, language)]));
            }
        }
        let unique_missing: Vec<_> = groups
            .iter()
            .map(|(text, targets)| (targets[0].0, *text, targets[0].1))
            .collect();
        let group_by_representative: std::collections::HashMap<_, _> = unique_missing
            .iter()
            .enumerate()
            .map(|(index, (id, _, _))| (*id, index))
            .collect();
        let mut offset = 0;

        // Persist each bounded inference result before producing the next one.
        // This retains the memory bound while cache lookup remains snapshot-like
        // for the entire collector batch.
        while offset < unique_missing.len() {
            let memory = crate::memory::MemoryBudget::current();
            if memory.under_pressure() {
                return Err(PipelineError::Parse {
                    path: std::path::PathBuf::new(),
                    reason: format!(
                        "semantic embedding stopped before swap pressure \
                         (available={} MiB, rss={} MiB)",
                        memory.available / (1024 * 1024),
                        memory.process_rss / (1024 * 1024),
                    ),
                });
            }
            let batch_size = memory.embedding_batch_size(64, accelerated);
            let end = (offset + batch_size).min(unique_missing.len());
            let inference = &unique_missing[offset..end];

            let embeddings =
                self.pool
                    .embed_parallel(inference)
                    .map_err(|e| PipelineError::Parse {
                        path: std::path::PathBuf::new(),
                        reason: format!("Embedding generation failed: {e}"),
                    })?;

            if !embeddings.is_empty() {
                let mut semantic = self.semantic.lock().map_err(|_| PipelineError::Parse {
                    path: std::path::PathBuf::new(),
                    reason: "Failed to lock semantic search".to_string(),
                })?;
                for (id, embedding, _) in embeddings {
                    let Some(&group_index) = group_by_representative.get(&id) else {
                        continue;
                    };
                    let (input, targets) = &groups[group_index];
                    stored += semantic.store_shared_embedding(input, embedding, targets);
                }
            }
            offset = end;
        }

        for (id, source, language) in &batch.body_candidates {
            self.process_symbol_source(*id, source, language, &body_hits)?;
            stored += 1;
        }
        Ok(stored)
    }

    /// Retain Arc references to matching cache entries, not all prepared source
    /// strings. At most the bounded cache's resident vectors are pinned until
    /// this collector batch ends; new vectors still enter the normal live cache.
    fn body_cache_hits(
        &self,
        batch: &EmbeddingBatch,
    ) -> PipelineResult<std::collections::HashMap<String, Arc<[f32]>>> {
        let mut hits = std::collections::HashMap::new();
        for (_, source, _) in &batch.body_candidates {
            if crate::memory::MemoryBudget::current().under_pressure() {
                return Err(PipelineError::Parse {
                    path: Default::default(),
                    reason: "body cache lookup stopped before memory pressure".into(),
                });
            }
            let inputs =
                source
                    .inputs(self.pool.input_budget())
                    .map_err(|reason| PipelineError::Parse {
                        path: Default::default(),
                        reason,
                    })?;
            let mut semantic = self.semantic.lock().map_err(|_| PipelineError::Parse {
                path: Default::default(),
                reason: "Failed to lock semantic search".into(),
            })?;
            for input in inputs {
                if let Some(vector) = semantic.cached_symbol_input(&input.text) {
                    hits.entry(crate::calculate_hash(&input.text))
                        .or_insert(vector);
                }
            }
        }
        Ok(hits)
    }

    fn process_symbol_source(
        &self,
        parent: crate::SymbolId,
        source: &crate::symbol_representation::SymbolSource,
        language: &str,
        body_hits: &std::collections::HashMap<String, Arc<[f32]>>,
    ) -> PipelineResult<()> {
        let failure = |reason: String| PipelineError::Parse {
            path: Default::default(),
            reason,
        };
        let inputs = source.inputs(self.pool.input_budget()).map_err(failure)?;
        let mut vectors: Vec<Option<Arc<[f32]>>> = {
            let mut semantic = self
                .semantic
                .lock()
                .map_err(|_| failure("Failed to lock semantic search".into()))?;
            inputs
                .iter()
                .map(|input| {
                    semantic
                        .cached_symbol_input(&input.text)
                        .or_else(|| body_hits.get(&crate::calculate_hash(&input.text)).cloned())
                })
                .collect()
        };
        // Correlation IDs are request-local ordinals, never persisted symbol IDs.
        let missing: Vec<_> = inputs
            .iter()
            .enumerate()
            .filter(|(index, input)| {
                vectors[*index].is_none()
                    && !inputs[..*index]
                        .iter()
                        .any(|earlier| earlier.text == input.text)
            })
            .map(|(index, input)| {
                (
                    crate::SymbolId::new(index as u32 + 1).expect("bounded segment ordinal"),
                    input.text.as_str(),
                    language,
                )
            })
            .collect();
        for chunk in missing.chunks(
            crate::memory::MemoryBudget::current()
                .embedding_batch_size(64, crate::memory::accelerated_embeddings_requested())
                .max(1),
        ) {
            if crate::memory::MemoryBudget::current().under_pressure() {
                return Err(failure(
                    "symbol representation embedding stopped before memory pressure".into(),
                ));
            }
            let results = self
                .pool
                .embed_parallel(chunk)
                .map_err(|error| failure(error.to_string()))?;
            for (ordinal, vector, _) in results {
                let index = ordinal.value() as usize - 1;
                if !chunk.iter().any(|(id, _, _)| *id == ordinal) || vectors[index].is_some() {
                    return Err(failure(
                        "unexpected or duplicate symbol segment embedding ordinal".into(),
                    ));
                }
                let shared: Arc<[f32]> = Arc::from(vector);
                for (slot, input) in inputs.iter().enumerate() {
                    if input.text == inputs[index].text && vectors[slot].is_none() {
                        vectors[slot] = Some(Arc::clone(&shared));
                    }
                }
            }
        }
        let segments: Result<Vec<_>, _> = vectors
            .into_iter()
            .zip(&inputs)
            .map(|(vector, input)| {
                vector
                    .map(|vector| {
                        crate::semantic::SymbolSegment::new(input.source_range.clone(), vector)
                    })
                    .ok_or_else(|| failure("missing symbol segment embedding".into()))
            })
            .collect();
        self.semantic
            .lock()
            .map_err(|_| failure("Failed to lock semantic search".into()))?
            .store_symbol_segments(parent, segments?, &inputs, language)
            .map_err(|error| failure(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_embed_stats_is_complete() {
        let mut stats = SemanticEmbedStats::default();
        assert!(stats.is_complete()); // 0 received, 0 processed

        stats.received = 10;
        stats.embedded = 8;
        stats.skipped = 2;
        assert!(stats.is_complete());

        stats.skipped = 1;
        assert!(!stats.is_complete()); // 8 + 1 != 10
    }
}
