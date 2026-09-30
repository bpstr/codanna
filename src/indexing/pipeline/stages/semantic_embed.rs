//! Semantic embedding stage - parallel embedding generation
//!
//! COLLECT -> EMBED (this stage) -> SimpleSemanticSearch
//!         -> INDEX -> Tantivy
//!
//! Receives EmbeddingBatch from COLLECT and runs in parallel with INDEX.

use crate::indexing::pipeline::types::{EmbeddingBatch, PipelineError, PipelineResult};
use crate::semantic::{EmbeddingBackend, SimpleSemanticSearch};
use crossbeam_channel::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[path = "body_embed.rs"]
mod body_embed;

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

/// Semantic embedding stage dispatching to local fastembed or a remote backend.
pub struct SemanticEmbedStage {
    pool: Arc<EmbeddingBackend>,
    semantic: Arc<Mutex<SimpleSemanticSearch>>,
    progress_callback: Option<EmbedProgressCallback>,
}

impl SemanticEmbedStage {
    /// Create a new semantic embed stage.
    pub fn new(pool: Arc<EmbeddingBackend>, semantic: Arc<Mutex<SimpleSemanticSearch>>) -> Self {
        Self { pool, semantic, progress_callback: None }
    }

    /// Add a progress callback receiving the count processed per collector batch.
    pub fn with_progress(mut self, callback: EmbedProgressCallback) -> Self {
        self.progress_callback = Some(callback);
        self
    }

    /// Generate and store embeddings until the input channel is closed.
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
                    tracing::debug!(target: "semantic", "EMBED batch {}: {} candidates", batches_received, candidate_count);
                    if !batch.is_empty() {
                        let count = self.process_batch(&batch)?;
                        stats.embedded += count;
                        stats.skipped += candidate_count - count;
                        if let Some(ref callback) = self.progress_callback {
                            callback(count as u64);
                        }
                        if last_stats_log.elapsed() >= STATS_LOG_INTERVAL {
                            self.pool.log_usage_stats();
                            last_stats_log = Instant::now();
                        }
                    }
                }
                Err(_) => break,
            }
        }
        self.pool.log_usage_stats();
        stats.elapsed = start.elapsed();
        tracing::info!(target: "semantic", "EMBED complete: {}/{} embedded in {:?} ({} batches)", stats.embedded, stats.received, stats.elapsed, batches_received);
        Ok(stats)
    }

    /// Process a batch of embedding candidates.
    pub(crate) fn process_batch(&self, batch: &EmbeddingBatch) -> PipelineResult<usize> {
        crate::semantic::validate_code_embedding_dimension(self.pool.dimensions()).map_err(
            |error| PipelineError::Parse {
                path: Default::default(),
                reason: error.to_string(),
            },
        )?;
        let accelerated = crate::memory::accelerated_embeddings_requested();
        // Prepare source partitions once and pin ALL existing body hits before
        // any comment/body admission can evict them. Text is materialized later
        // in byte- and segment-bounded cross-symbol windows.
        let body_plan = body_embed::BodyPlan::prepare(batch, &self.pool, &self.semantic)?;

        // Probe the whole comment collector batch before admitting new vectors:
        // the production batch is larger than the default cache.
        let items: Vec<_> = batch.candidates.iter()
            .map(|(id, doc, lang, _)| (*id, doc.as_ref(), lang.as_ref()))
            .collect();
        let missing = {
            let mut semantic = self.semantic.lock().map_err(|_| PipelineError::Parse {
                path: Default::default(),
                reason: "Failed to lock semantic search".into(),
            })?;
            semantic.reuse_cached_embeddings(&items)
        };
        let mut stored = items.len() - missing.len();
        // Preserve the existing comment path's exact-input deduplication.
        let mut group_indexes: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
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
        let unique_missing: Vec<_> = groups.iter()
            .map(|(text, targets)| (targets[0].0, *text, targets[0].1))
            .collect();
        let group_by_representative: std::collections::HashMap<_, _> = unique_missing.iter()
            .enumerate().map(|(index, (id, _, _))| (*id, index)).collect();
        let mut offset = 0;
        while offset < unique_missing.len() {
            let memory = crate::memory::MemoryBudget::current();
            if memory.under_pressure() {
                return Err(PipelineError::Parse {
                    path: Default::default(),
                    reason: format!(
                        "semantic embedding stopped before swap pressure (available={} MiB, rss={} MiB)",
                        memory.available / (1024 * 1024), memory.process_rss / (1024 * 1024),
                    ),
                });
            }
            let batch_size = memory.embedding_batch_size(64, accelerated);
            let end = (offset + batch_size).min(unique_missing.len());
            let embeddings = self.pool.embed_parallel(&unique_missing[offset..end])
                .map_err(|error| PipelineError::Parse {
                    path: Default::default(),
                    reason: format!("Embedding generation failed: {error}"),
                })?;
            if !embeddings.is_empty() {
                let mut semantic = self.semantic.lock().map_err(|_| PipelineError::Parse {
                    path: Default::default(),
                    reason: "Failed to lock semantic search".into(),
                })?;
                for (id, embedding, _) in embeddings {
                    let Some(&group_index) = group_by_representative.get(&id) else { continue; };
                    let (input, targets) = &groups[group_index];
                    stored += semantic.store_shared_embedding(input, embedding, targets);
                }
            }
            offset = end;
        }

        stored += body_plan.process(&self.pool, &self.semantic)?;
        let mut semantic = self.semantic.lock().map_err(|_| PipelineError::Parse {
            path: Default::default(),
            reason: "Failed to lock semantic search".into(),
        })?;
        for (id, _, _, source_sha256) in &batch.candidates {
            semantic.record_source_provenance(*id, source_sha256.clone());
        }
        for (id, _, _, source_sha256) in &batch.body_candidates {
            semantic.record_source_provenance(*id, source_sha256.clone());
        }
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_embed_stats_is_complete() {
        let mut stats = SemanticEmbedStats::default();
        assert!(stats.is_complete());
        stats.received = 10;
        stats.embedded = 8;
        stats.skipped = 2;
        assert!(stats.is_complete());
        stats.skipped = 1;
        assert!(!stats.is_complete());
    }

    #[tokio::test]
    async fn cached_batch_records_source_provenance_without_provider_requests() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let id = crate::SymbolId::new(1).unwrap();
        let input = "prepared documentation";
        let mut search = SimpleSemanticSearch::new_empty(2, "fixture");
        search.store_embeddings_with_inputs(
            vec![(id, vec![1.0, 0.0], "go".into())],
            &[(id, input, "go")],
        );
        assert!(search.cached_symbol_input(input).is_some());
        let search = Arc::new(Mutex::new(search));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mock = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") { break; }
            }
            assert!(!String::from_utf8_lossy(&request).to_lowercase().contains("authorization:"));
            let body = r#"{"data":[{"index":0,"embedding":[1.0,0.0]}]}"#;
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
            ).as_bytes()).await.unwrap();
        });
        // This fixture serves only the constructor's dimension probe. The
        // candidate itself must be satisfied without a provider request.
        let remote = crate::semantic::RemoteEmbedder::new(
            &format!("http://{address}"), "fixture", Some(2), None,
        ).await.unwrap();
        mock.await.unwrap();
        let stage = SemanticEmbedStage::new(
            Arc::new(EmbeddingBackend::Remote(Arc::new(remote))), Arc::clone(&search),
        );
        let source_sha256 = crate::indexing::calculate_hash("package fixture\nfunc f() {}\n");
        let mut batch = EmbeddingBatch::new();
        batch.candidates.push((id, input.into(), "go".into(), source_sha256.clone()));
        assert_eq!(stage.process_batch(&batch).unwrap(), 1);
        let mut search = search.lock().unwrap();
        assert!(search.symbol_provenance(id).is_none());
        search.bind_code_generation(7);
        let provenance = search.symbol_provenance(id).unwrap();
        assert_eq!(provenance.source_sha256, source_sha256);
        assert_eq!(provenance.code_generation, 7);
    }
}
