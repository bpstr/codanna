//! Persistent, reader-content-checked symbol cache for the single-file watcher lane.

use super::{Pipeline, PipelineResult, SymbolLookupCache};
use crate::{FileId, storage::DocumentIndex};
use std::sync::{Arc, Weak};

pub(super) struct CachedSymbols {
    index: Weak<DocumentIndex>,
    snapshot: tantivy::SearcherGeneration,
    symbols: Arc<SymbolLookupCache>,
    #[cfg(test)]
    refreshes: usize,
}

impl Pipeline {
    /// `before` is one reader snapshot immediately before this serialized mutation.
    /// Compare segment IDs and delete opstamps: unchanged delayed reloads can
    /// advance numeric generation without changing any cached symbol documents.
    /// External edits, deletions, merges and other index instances still rebuild.
    pub(super) fn resolution_cache(
        &self,
        index: &Arc<DocumentIndex>,
        before: Option<tantivy::SearcherGeneration>,
        changed_files: &[FileId],
        needs_resolution: bool,
    ) -> PipelineResult<Arc<SymbolLookupCache>> {
        let mut state = self
            .symbol_cache
            .lock()
            .map_err(|_| crate::IndexError::MutexPoisoned)?;
        let current = index.reader_snapshot();
        let reusable = state.as_ref().is_some_and(|cached| {
            cached
                .index
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, index))
                && cached.snapshot.segments() == before.as_ref().unwrap_or(&current).segments()
        });
        if reusable {
            let cached = state.as_mut().expect("reusable state exists");
            // Invalidate before the fallible refresh; a failed reader must not
            // leave a reusable partially-updated cache on the next attempt.
            let symbols = Arc::clone(&cached.symbols);
            let old = state.take().expect("reusable state exists");
            symbols.refresh_files(index, changed_files)?;
            *state = Some(CachedSymbols {
                index: Arc::downgrade(index),
                snapshot: current,
                symbols: Arc::clone(&symbols),
                #[cfg(test)]
                refreshes: old.refreshes + changed_files.len(),
            });
            #[cfg(not(test))]
            drop(old);
            return Ok(symbols);
        }
        *state = None;
        if !needs_resolution {
            return Ok(Arc::new(SymbolLookupCache::new()));
        }
        let symbols = Arc::new(SymbolLookupCache::from_index(index)?);
        *state = Some(CachedSymbols {
            index: Arc::downgrade(index),
            snapshot: current,
            symbols: Arc::clone(&symbols),
            #[cfg(test)]
            refreshes: 0,
        });
        Ok(symbols)
    }

    /// Relationship-only commits advance the reader but do not invalidate the
    /// symbol maps. Only bless the same cache after a successful resolution run.
    pub(super) fn finish_resolution_cache(
        &self,
        index: &Arc<DocumentIndex>,
        symbols: &Arc<SymbolLookupCache>,
    ) -> PipelineResult<()> {
        let mut state = self
            .symbol_cache
            .lock()
            .map_err(|_| crate::IndexError::MutexPoisoned)?;
        if let Some(cached) = state.as_mut() {
            if Arc::ptr_eq(&cached.symbols, symbols) {
                cached.snapshot = index.reader_snapshot();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Range, Settings, Symbol, SymbolId, SymbolKind};

    fn symbol(id: u32, file: u32, name: &str) -> Symbol {
        Symbol::new(
            SymbolId::new(id).unwrap(),
            name,
            SymbolKind::Function,
            FileId::new(file).unwrap(),
            Range::new(0, 0, 0, 1),
        )
    }

    #[test]
    fn hardening_final_cache_refresh_reuses_corpus_and_removes_stale_names() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::default());
        let index = Arc::new(DocumentIndex::new(dir.path(), &settings).unwrap());
        index.start_batch().unwrap();
        for id in 1..=1000 {
            index
                .index_symbol(
                    &symbol(id, id, &format!("item{id}")),
                    &format!("src/{id}.rs"),
                )
                .unwrap();
        }
        index.commit_batch().unwrap();
        let pipeline = Pipeline::with_settings(settings);
        let first = pipeline.resolution_cache(&index, None, &[], true).unwrap();
        assert_eq!(first.len(), 1000);
        let before = index.reader_snapshot();
        first.register_module_alias("old.alias", SymbolId::new(1).unwrap());
        index.start_batch().unwrap();
        index.delete_symbol(SymbolId::new(1).unwrap()).unwrap();
        index
            .index_symbol(&symbol(1001, 1, "replacement"), "src/1.rs")
            .unwrap();
        index.commit_batch().unwrap();
        let next = pipeline
            .resolution_cache(&index, Some(before), &[FileId::new(1).unwrap()], true)
            .unwrap();
        assert!(
            Arc::ptr_eq(&first, &next),
            "single-file delta must retain the warm corpus"
        );
        assert_eq!(next.len(), 1000);
        assert_eq!(next.resolve_module_alias("old.alias"), None);
        assert!(next.lookup_candidates("item1").is_empty());
        assert_eq!(
            next.symbols_in_file(FileId::new(1).unwrap()),
            vec![SymbolId::new(1001).unwrap()]
        );
        next.insert(symbol(1001, 1, "replacement"));
        assert_eq!(
            next.lookup_candidates("replacement"),
            vec![SymbolId::new(1001).unwrap()]
        );
        assert_eq!(
            pipeline
                .symbol_cache
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .refreshes,
            1
        );
    }

    #[test]
    fn hardening_final_single_file_pipeline_keeps_warm_cache_across_edits() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings {
            workspace_root: Some(dir.path().to_path_buf()),
            index_path: dir.path().join("index"),
            ..Settings::default()
        });
        let index =
            Arc::new(DocumentIndex::new(settings.index_path.join("tantivy"), &settings).unwrap());
        let pipeline = Pipeline::with_settings(settings);
        let source = dir.path().join("main.rs");
        std::fs::write(
            &source,
            "pub fn target() {}\npub fn old_caller() { target(); }\n",
        )
        .unwrap();
        pipeline
            .index_file_single(&source, Arc::clone(&index), None, None)
            .unwrap();
        let first = Arc::clone(
            &pipeline
                .symbol_cache
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .symbols,
        );
        std::fs::write(
            &source,
            "pub fn target() {}\npub fn new_caller() { target(); }\n",
        )
        .unwrap();
        pipeline
            .index_file_single(&source, Arc::clone(&index), None, None)
            .unwrap();
        let second = Arc::clone(
            &pipeline
                .symbol_cache
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .symbols,
        );
        assert!(Arc::ptr_eq(&first, &second));
        assert!(second.lookup_candidates("old_caller").is_empty());
        assert_eq!(second.lookup_candidates("new_caller").len(), 1);
        let caller = index
            .find_symbols_by_name("new_caller", None)
            .unwrap()
            .remove(0);
        let target = index
            .find_symbols_by_name("target", None)
            .unwrap()
            .remove(0);
        let edges = index
            .get_relationships_from(caller.id, crate::RelationKind::Calls)
            .unwrap();
        assert!(edges.iter().any(|(_, to, _)| *to == target.id));
    }

    #[test]
    fn unchanged_reader_reload_preserves_warm_symbol_cache() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::default());
        let index = Arc::new(DocumentIndex::new(dir.path(), &settings).unwrap());
        index.start_batch().unwrap();
        index
            .index_symbol(&symbol(1, 1, "unchanged"), "src/one.rs")
            .unwrap();
        index.commit_batch().unwrap();
        let pipeline = Pipeline::with_settings(settings);
        let first = pipeline.resolution_cache(&index, None, &[], true).unwrap();
        let before = index.reader_snapshot();

        // An empty commit explicitly reloads the same documents, deterministically
        // exercising the delayed callback's effect without sleeps or retries.
        index.start_batch().unwrap();
        index.commit_batch().unwrap();
        let after = index.reader_snapshot();
        assert_ne!(before.generation_id(), after.generation_id());
        assert_eq!(before.segments(), after.segments());
        let next = pipeline.resolution_cache(&index, None, &[], true).unwrap();
        assert!(Arc::ptr_eq(&first, &next));
        assert_eq!(
            next.lookup_candidates("unchanged"),
            vec![SymbolId::new(1).unwrap()]
        );
    }

    #[test]
    fn delete_only_commit_invalidates_warm_symbol_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        // Tantivy's per-thread minimum makes this a single-writer-thread fixture,
        // so deleting one of two documents retains the same physical segment.
        settings.indexing.tantivy_heap_mb = 15;
        let settings = Arc::new(settings);
        let index = Arc::new(DocumentIndex::new(dir.path(), &settings).unwrap());
        index.start_batch().unwrap();
        for (id, name) in [(1, "deleted"), (2, "survivor")] {
            index
                .index_symbol(&symbol(id, id, name), &format!("src/{id}.rs"))
                .unwrap();
        }
        index.commit_batch().unwrap();
        let pipeline = Pipeline::with_settings(settings);
        let first = pipeline.resolution_cache(&index, None, &[], true).unwrap();
        let before = index.reader_snapshot();

        index.start_batch().unwrap();
        index.delete_symbol(SymbolId::new(1).unwrap()).unwrap();
        index.commit_batch().unwrap();
        let after = index.reader_snapshot();
        assert_eq!(
            before.segments().keys().collect::<Vec<_>>(),
            after.segments().keys().collect::<Vec<_>>()
        );
        assert_ne!(before.segments(), after.segments());
        let next = pipeline.resolution_cache(&index, None, &[], true).unwrap();
        assert!(!Arc::ptr_eq(&first, &next));
        assert!(next.lookup_candidates("deleted").is_empty());
        assert_eq!(
            next.lookup_candidates("survivor"),
            vec![SymbolId::new(2).unwrap()]
        );
    }

    #[test]
    fn hardening_final_cache_rebuilds_after_external_commit_and_isolates_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::default());
        let pipeline = Pipeline::with_settings(Arc::clone(&settings));
        let index = Arc::new(DocumentIndex::new(dir.path(), &settings).unwrap());
        let initial = pipeline.resolution_cache(&index, None, &[], true).unwrap();
        index.start_batch().unwrap();
        index
            .index_symbol(&symbol(1, 1, "external"), "src/external.rs")
            .unwrap();
        index.commit_batch().unwrap();
        let rebuilt = pipeline.resolution_cache(&index, None, &[], true).unwrap();
        assert!(!Arc::ptr_eq(&initial, &rebuilt));
        assert_eq!(
            rebuilt.lookup_candidates("external"),
            vec![SymbolId::new(1).unwrap()]
        );
        let other = Arc::new(DocumentIndex::new(other.path(), &settings).unwrap());
        assert!(
            pipeline
                .resolution_cache(&other, None, &[], true)
                .unwrap()
                .is_empty()
        );
    }
}
