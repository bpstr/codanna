//! One retained resolution wave, retried before accepting another code mutation.
//! Ownership is shared with the serialized blocking worker, not its async caller.
use crate::IndexResult;
use crate::indexing::{facade::IndexFacade, pipeline::PendingResolution};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct DeferredCodeState {
    pub(super) pending: Option<PendingResolution>,
    retry_at: Option<Instant>,
    failures: u32,
}

impl DeferredCodeState {
    pub(super) fn ready(&self, now: Instant) -> bool {
        self.retry_at.is_none_or(|deadline| now >= deadline)
    }

    pub(super) fn resolve(&mut self, indexer: &mut IndexFacade, now: Instant) -> IndexResult<()> {
        let Some(pending) = self.pending.as_mut() else {
            return Ok(());
        };
        match indexer.resolve_deferred(pending) {
            Ok(()) => {
                self.pending = None;
                self.retry_at = None;
                self.failures = 0;
                Ok(())
            }
            Err(error) if error.is_writer_unavailable() => {
                self.failures = self.failures.saturating_add(1);
                let delay = Duration::from_millis(
                    250u64.saturating_mul(1u64 << self.failures.saturating_sub(1).min(7)),
                )
                .min(Duration::from_secs(30));
                self.retry_at = now.checked_add(delay);
                // Keep exactly one prepared value. Replaying Phase 1 would invalidate IDs.
                Err(error)
            }
            Err(error) => {
                // Partial writes are not retryable. Do not accidentally replay them.
                self.pending = None;
                self.retry_at = None;
                self.failures = 0;
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RelationKind, Settings, storage::DocumentIndex};
    use std::sync::Arc;

    #[test]
    fn upstream_repair_retained_wave_backs_off_and_retries_without_reindexing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("src");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("main.py"),
            "def callee():\n    return 1\n\ndef caller():\n    return callee()\n",
        )
        .unwrap();
        let mut settings = Settings {
            index_path: dir.path().join("index"),
            workspace_root: None,
            ..Settings::default()
        };
        settings.semantic_search.enabled = false;
        settings.add_indexed_path(root.clone()).unwrap();
        let settings = Arc::new(settings);
        let mut facade = IndexFacade::new(Arc::clone(&settings)).unwrap();
        let mut pending = PendingResolution::default();
        facade
            .index_directory_deferred(&root, false, &mut pending)
            .unwrap();
        let caller = facade.find_symbols_by_name("caller", None)[0].id;
        let other = DocumentIndex::new(settings.index_path.join("tantivy"), &settings).unwrap();
        other.start_batch().unwrap();
        let now = Instant::now();
        let mut state = DeferredCodeState {
            pending: Some(pending),
            ..Default::default()
        };
        let result = state.resolve(&mut facade, now);
        other.rollback_batch().unwrap();
        assert!(result.unwrap_err().is_writer_unavailable());
        assert!(state.pending.is_some());
        assert!(!state.ready(now));
        assert!(state.ready(now + Duration::from_millis(250)));
        state
            .resolve(&mut facade, now + Duration::from_millis(250))
            .unwrap();
        assert!(state.pending.is_none());
        assert_eq!(facade.find_symbols_by_name("caller", None)[0].id, caller);
        assert_eq!(
            facade
                .document_index()
                .get_relationships_from(caller, RelationKind::Calls)
                .unwrap()
                .len(),
            1
        );
    }
}
