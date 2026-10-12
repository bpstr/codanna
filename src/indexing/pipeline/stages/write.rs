//! Write stage - store resolved relationships to Tantivy
//!
//! Final stage of Phase 2: takes ResolvedBatch from RESOLVE stage
//! and writes relationships to DocumentIndex.
//!
//! Data flow:
//! - Receives: ResolvedBatch from RESOLVE stage
//! - Writes: Relationships to Tantivy via DocumentIndex
//! - Outputs: WriteStats with counts

use crate::indexing::pipeline::types::{ResolvedBatch, ResolvedRelationship};
use crate::relationship::Relationship;
use crate::storage::DocumentIndex;
use std::sync::Arc;

/// Write stage for storing resolved relationships.
///
/// Single-threaded: Tantivy IndexWriter is not Send.
/// Batches writes and commits periodically.
pub struct WriteStage {
    index: Arc<DocumentIndex>,
    /// Accumulated relationships before commit
    pending: Vec<ResolvedRelationship>,
    /// Commit every N relationships
    commit_threshold: usize,
    /// Whether a batch is currently active
    batch_started: bool,
    /// Unlike "has committed", this also rejects replay after staged-write failures.
    ever_started: bool,
}

/// Statistics from write operations.
#[derive(Debug, Default, Clone)]
pub struct WriteStats {
    /// Relationships accepted by the writer; durable only after commit/flush succeeds
    pub written: usize,
    /// Number of commits performed
    pub commits: usize,
    /// Retained for API compatibility; failed writes now return an error
    pub failed: usize,
}

impl WriteStage {
    /// Create a new write stage with the document index.
    pub fn new(index: Arc<DocumentIndex>) -> Self {
        Self {
            index,
            pending: Vec::new(),
            commit_threshold: 10_000, // Commit every 10K relationships
            batch_started: false,
            ever_started: false,
        }
    }

    /// Create with custom commit threshold.
    pub fn with_commit_threshold(index: Arc<DocumentIndex>, threshold: usize) -> Self {
        Self {
            index,
            pending: Vec::new(),
            commit_threshold: threshold,
            batch_started: false,
            ever_started: false,
        }
    }

    /// Ensure a batch is started before writing.
    fn ensure_batch_started(&mut self) -> Result<(), crate::storage::StorageError> {
        if !self.batch_started {
            self.index.start_batch()?;
            self.batch_started = true;
            self.ever_started = true;
        }
        Ok(())
    }

    /// True only while no batch has ever been acquired by this stage.
    pub(crate) fn can_retry_start(&self) -> bool {
        !self.ever_started
    }

    /// Write a batch of resolved relationships.
    ///
    /// Accumulates in memory and commits when threshold reached. The writer is
    /// acquired only when a relationship is actually stored, so an empty batch
    /// never creates a Tantivy writer.
    pub fn write(&mut self, batch: ResolvedBatch) -> crate::storage::StorageResult<WriteStats> {
        let mut stats = WriteStats::default();
        for resolved in batch.relationships {
            self.stage(resolved)?;
            stats.written += 1;
            if !self.batch_started {
                stats.commits += 1;
            }
        }
        Ok(stats)
    }

    /// Write a single resolved relationship.
    pub fn write_one(
        &mut self,
        resolved: ResolvedRelationship,
    ) -> Result<(), crate::storage::StorageError> {
        self.stage(resolved)?;
        Ok(())
    }

    /// Acquire the writer, stage one relationship, and release it at the
    /// commit threshold. A failed store or commit remains non-retryable once
    /// this stage has acquired a writer; Drop rolls back any unfinished batch.
    fn stage(
        &mut self,
        resolved: ResolvedRelationship,
    ) -> Result<(), crate::storage::StorageError> {
        self.ensure_batch_started()?;

        let relationship = Relationship {
            kind: resolved.kind,
            weight: 1.0,
            metadata: resolved.metadata.clone(),
        };

        self.index
            .store_relationship(resolved.from_id, resolved.to_id, &relationship)?;
        self.pending.push(resolved);

        if self.pending.len() >= self.commit_threshold {
            self.commit_internal()?;
        }

        Ok(())
    }

    /// Commit pending writes to disk.
    ///
    /// Call after each pass (Defines, then Calls) to ensure
    /// Pass 2 can query Pass 1 results.
    pub fn commit(&mut self) -> Result<usize, crate::storage::StorageError> {
        let count = self.pending.len();
        self.commit_internal()?;
        Ok(count)
    }

    /// Commit the active batch and release the writer. The next relationship
    /// reacquires a fresh batch. With no batch open there is nothing to commit.
    fn commit_internal(&mut self) -> Result<(), crate::storage::StorageError> {
        if !self.batch_started {
            return Ok(());
        }
        self.index.commit_batch()?;
        self.pending.clear();
        self.batch_started = false;
        Ok(())
    }

    /// Flush any remaining pending writes and release the writer.
    ///
    /// Call at end of Phase 2 to ensure all relationships are committed.
    pub fn flush(&mut self) -> Result<WriteStats, crate::storage::StorageError> {
        let written = self.pending.len();
        if self.batch_started {
            self.index.commit_batch()?;
            self.pending.clear();
            self.batch_started = false;
        }
        Ok(WriteStats {
            written,
            commits: if written > 0 { 1 } else { 0 },
            failed: 0,
        })
    }

    /// Get count of pending (uncommitted) relationships.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

impl Drop for WriteStage {
    fn drop(&mut self) {
        if self.batch_started
            && let Err(error) = self.index.rollback_batch()
        {
            tracing::error!(target: "pipeline", "Failed to discard unfinished relationship batch: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RelationKind;
    use crate::config::Settings;
    use crate::types::SymbolId;
    use tempfile::TempDir;

    fn make_resolved(from: u32, to: u32, kind: RelationKind) -> ResolvedRelationship {
        ResolvedRelationship {
            from_id: SymbolId::new(from).unwrap(),
            to_id: SymbolId::new(to).unwrap(),
            kind,
            metadata: None,
        }
    }

    fn poison_writer(index: Arc<DocumentIndex>) {
        assert!(
            std::thread::spawn(move || {
                let _guard = index.writer.write().unwrap();
                panic!("deterministic unavailable storage fixture");
            })
            .join()
            .is_err()
        );
    }

    #[test]
    fn hardening_review_relationship_writer_propagates_start_failure() {
        let dir = TempDir::new().unwrap();
        let index = Arc::new(DocumentIndex::new(dir.path(), &Settings::default()).unwrap());
        poison_writer(Arc::clone(&index));
        let mut stage = WriteStage::new(index);
        let mut batch = ResolvedBatch::new();
        batch.push(make_resolved(1, 2, RelationKind::Calls));
        assert!(stage.write(batch).is_err());
        assert_eq!(stage.pending_count(), 0);
    }

    #[test]
    fn hardening_review_relationship_commit_failure_preserves_pending_state() {
        let dir = TempDir::new().unwrap();
        let index = Arc::new(DocumentIndex::new(dir.path(), &Settings::default()).unwrap());
        let mut stage = WriteStage::new(Arc::clone(&index));
        stage
            .write_one(make_resolved(1, 2, RelationKind::Calls))
            .unwrap();
        // Force actual metadata publication to fail without permission or timing assumptions.
        let meta_path = dir.path().join("meta.json");
        std::fs::remove_file(&meta_path).unwrap();
        std::fs::create_dir(&meta_path).unwrap();
        assert!(stage.flush().is_err());
        assert_eq!(stage.pending_count(), 1);
    }

    #[test]
    fn upstream_repair_only_the_first_writer_start_is_retryable() {
        let dir = TempDir::new().unwrap();
        let index = Arc::new(DocumentIndex::new(dir.path(), &Settings::default()).unwrap());
        let mut stage = WriteStage::with_commit_threshold(Arc::clone(&index), 1);
        assert!(stage.can_retry_start());
        stage
            .write_one(make_resolved(1, 2, RelationKind::Calls))
            .unwrap();
        assert!(
            !stage.can_retry_start(),
            "a committed batch must never be replayed"
        );
        poison_writer(Arc::clone(&index));
        assert!(
            stage
                .write_one(make_resolved(2, 3, RelationKind::Calls))
                .is_err()
        );
        assert!(
            !stage.can_retry_start(),
            "a later writer/store failure is not an initial acquisition failure"
        );
    }

    #[test]
    fn test_write_empty_batch() {
        let temp_dir = TempDir::new().unwrap();
        let settings = Settings::default();
        let index = Arc::new(DocumentIndex::new(temp_dir.path(), &settings).unwrap());

        let mut stage = WriteStage::new(Arc::clone(&index));
        let batch = ResolvedBatch::new();

        let stats = stage.write(batch).unwrap();
        let flushed = stage.flush().unwrap();

        assert_eq!(stats.written, 0);
        assert_eq!(stats.failed, 0);
        assert_eq!(flushed.written, 0);
        assert_eq!(flushed.commits, 0);

        let other = DocumentIndex::new(temp_dir.path(), &settings).unwrap();
        other
            .start_batch()
            .expect("empty Phase 2 work must not leave the writer acquired");
        other.rollback_batch().unwrap();
    }

    #[test]
    fn test_write_batch_accumulates() {
        let temp_dir = TempDir::new().unwrap();
        let settings = Settings::default();
        let index = Arc::new(DocumentIndex::new(temp_dir.path(), &settings).unwrap());

        // Use high threshold so no auto-commit
        let mut stage = WriteStage::with_commit_threshold(Arc::clone(&index), 100);

        let mut batch = ResolvedBatch::new();
        batch.push(make_resolved(1, 2, RelationKind::Calls));
        batch.push(make_resolved(2, 3, RelationKind::Defines));

        let stats = stage.write(batch).unwrap();

        assert_eq!(stats.written, 2);
        assert_eq!(stage.pending_count(), 2);
        assert_eq!(stats.commits, 0); // No auto-commit yet
    }

    #[test]
    fn test_commit_clears_pending() {
        let temp_dir = TempDir::new().unwrap();
        let settings = Settings::default();
        let index = Arc::new(DocumentIndex::new(temp_dir.path(), &settings).unwrap());

        let mut stage = WriteStage::new(Arc::clone(&index));

        let mut batch = ResolvedBatch::new();
        batch.push(make_resolved(1, 2, RelationKind::Calls));

        stage.write(batch).unwrap();
        assert_eq!(stage.pending_count(), 1);

        let count = stage.commit().unwrap();
        assert_eq!(count, 1);
        assert_eq!(stage.pending_count(), 0);
    }

    #[test]
    fn test_auto_commit_at_threshold() {
        let temp_dir = TempDir::new().unwrap();
        let settings = Settings::default();
        let index = Arc::new(DocumentIndex::new(temp_dir.path(), &settings).unwrap());

        // Low threshold for testing
        let mut stage = WriteStage::with_commit_threshold(Arc::clone(&index), 2);

        let mut batch = ResolvedBatch::new();
        batch.push(make_resolved(1, 2, RelationKind::Calls));
        batch.push(make_resolved(2, 3, RelationKind::Defines));
        batch.push(make_resolved(3, 4, RelationKind::Calls));

        let stats = stage.write(batch).unwrap();

        // 3 written, 1 commit triggered at threshold
        assert_eq!(stats.written, 3);
        assert_eq!(stats.commits, 1);
        // 1 pending after commit (the 3rd one)
        assert_eq!(stage.pending_count(), 1);
    }

    #[test]
    fn test_flush_commits_remaining() {
        let temp_dir = TempDir::new().unwrap();
        let settings = Settings::default();
        let index = Arc::new(DocumentIndex::new(temp_dir.path(), &settings).unwrap());

        let mut stage = WriteStage::new(Arc::clone(&index));

        let mut batch = ResolvedBatch::new();
        batch.push(make_resolved(1, 2, RelationKind::Calls));

        stage.write(batch).unwrap();
        assert_eq!(stage.pending_count(), 1);

        let flush_stats = stage.flush().unwrap();
        assert_eq!(flush_stats.written, 1);
        assert_eq!(flush_stats.commits, 1);
        assert_eq!(stage.pending_count(), 0);
    }

    #[test]
    fn threshold_commit_reacquires_only_for_following_rows_and_flush_releases() {
        let temp_dir = TempDir::new().unwrap();
        let settings = Settings::default();
        let index = Arc::new(DocumentIndex::new(temp_dir.path(), &settings).unwrap());
        let mut stage = WriteStage::with_commit_threshold(Arc::clone(&index), 2);

        let mut batch = ResolvedBatch::new();
        batch.push(make_resolved(1, 2, RelationKind::Calls));
        batch.push(make_resolved(2, 3, RelationKind::Defines));
        batch.push(make_resolved(3, 4, RelationKind::Calls));

        let stats = stage.write(batch).unwrap();
        assert_eq!(stats.written, 3);
        assert_eq!(stats.commits, 1);
        assert_eq!(stage.pending_count(), 1);

        let flushed = stage.flush().unwrap();
        assert_eq!(flushed.written, 1);
        assert_eq!(flushed.commits, 1);
        assert_eq!(index.count_relationships().unwrap(), 3);

        let other = DocumentIndex::new(temp_dir.path(), &settings).unwrap();
        other
            .start_batch()
            .expect("flush must release the Phase 2 writer");
        other.rollback_batch().unwrap();
    }
}
