//! Workspace-relative file scopes resolved on the same snapshot as symbol search.

use super::DocumentIndex;
use crate::storage::{StorageError, StorageResult};
use std::path::Path;
use std::sync::Arc;
use tantivy::collector::DocSetCollector;
use tantivy::query::TermQuery;
use tantivy::schema::{IndexRecordOption, Value};
use tantivy::{Searcher, TantivyDocument, Term};

const MAX_CACHED_FILES: usize = 25_000;
const MAX_CACHED_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
struct ScopeEntry {
    path: String,
    file_id: u64,
}

#[derive(Debug)]
pub(super) struct ScopeInventory {
    generation: u64,
    entries: Vec<ScopeEntry>,
    logical_bytes: usize,
}

impl ScopeInventory {
    fn cacheable(&self) -> bool {
        self.entries.len() <= MAX_CACHED_FILES && self.logical_bytes <= MAX_CACHED_BYTES
    }

    fn terms(&self, prefix: &str, field: tantivy::schema::Field) -> Vec<Term> {
        let make_term = |entry: &ScopeEntry| Term::from_field_u64(field, entry.file_id);
        if prefix.is_empty() {
            return self.entries.iter().map(make_term).collect();
        }
        // Exact-file matches and descendants are separate ranges. A lexical
        // neighbor such as Calendar.ts.backup sorts before Calendar.ts/.
        let exact_start = self
            .entries
            .partition_point(|entry| entry.path.as_str() < prefix);
        let descendants = format!("{prefix}/");
        let descendant_start = self
            .entries
            .partition_point(|entry| entry.path.as_str() < descendants.as_str());
        self.entries[exact_start..]
            .iter()
            .take_while(|entry| entry.path == prefix)
            .chain(
                self.entries[descendant_start..]
                    .iter()
                    .take_while(|entry| entry.path.starts_with(&descendants)),
            )
            .map(make_term)
            .collect()
    }
}

fn normalize_prefix(prefix: &str) -> StorageResult<String> {
    let portable = prefix.trim().replace('\\', "/");
    let invalid = |reason: &str| StorageError::InvalidFieldValue {
        field: "path_prefix".into(),
        reason: reason.into(),
    };
    if portable.is_empty() {
        return Err(invalid(
            "path prefix must be a non-empty workspace-relative path",
        ));
    }
    if portable.starts_with('/') || portable.as_bytes().get(1) == Some(&b':') {
        return Err(invalid("path prefix must be workspace-relative"));
    }
    let mut parts = Vec::new();
    for part in portable.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(invalid("path prefix cannot escape the workspace with '..'")),
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

impl DocumentIndex {
    fn scoped_stored_path(&self, stored: &str) -> Option<String> {
        let native = Path::new(stored);
        let relative = if native.is_absolute() {
            native.strip_prefix(self.workspace_root.as_ref()?).ok()?
        } else {
            native
        };
        // Do not use display-path shortening: it also strips external roots.
        // Query scope must identify the configured workspace, not a namesake in
        // a separately registered checkout. No filesystem reads happen here.
        let portable = relative.to_str()?.replace('\\', "/");
        if portable.starts_with('/') || portable.as_bytes().get(1) == Some(&b':') {
            return None;
        }
        let mut parts = Vec::new();
        for part in portable.split('/') {
            match part {
                "" | "." => {}
                ".." => return None,
                _ => parts.push(part),
            }
        }
        (!parts.is_empty()).then(|| parts.join("/"))
    }

    fn scope_inventory_for(&self, searcher: &Searcher) -> StorageResult<Arc<ScopeInventory>> {
        let generation = searcher.generation().generation_id();
        if let Ok(cache) = self.scope_inventory.read() {
            if let Some(inventory) = cache.as_ref().filter(|item| item.generation == generation) {
                return Ok(Arc::clone(inventory));
            }
        }
        let query = TermQuery::new(
            Term::from_field_text(self.schema.doc_type, "file_info"),
            IndexRecordOption::Basic,
        );
        let addresses = searcher.search(&query, &DocSetCollector)?;
        let mut entries = Vec::new();
        let mut path_bytes = 0usize;
        for address in addresses {
            let doc: TantivyDocument = searcher.doc(address)?;
            let Some(path) = doc
                .get_first(self.schema.file_path)
                .and_then(|value| value.as_str())
                .and_then(|path| self.scoped_stored_path(path))
            else {
                continue;
            };
            let Some(file_id) = doc
                .get_first(self.schema.file_id)
                .and_then(|value| value.as_u64())
            else {
                continue;
            };
            path_bytes = path_bytes.saturating_add(path.capacity());
            entries.push(ScopeEntry { path, file_id });
        }
        entries.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.file_id.cmp(&right.file_id))
        });
        let logical_bytes = path_bytes.saturating_add(
            entries
                .capacity()
                .saturating_mul(std::mem::size_of::<ScopeEntry>()),
        );
        let inventory = Arc::new(ScopeInventory {
            generation,
            entries,
            logical_bytes,
        });
        if inventory.cacheable() {
            if let Ok(mut cache) = self.scope_inventory.write() {
                // A slow old reader may finish after a new one. It can use its
                // local snapshot but must not replace a newer cached inventory.
                if cache
                    .as_ref()
                    .is_none_or(|item| item.generation <= generation)
                {
                    *cache = Some(Arc::clone(&inventory));
                }
            }
        }
        Ok(inventory)
    }

    pub(super) fn file_scope_terms(
        &self,
        searcher: &Searcher,
        prefix: &str,
    ) -> StorageResult<Vec<Term>> {
        let prefix = normalize_prefix(prefix)?;
        Ok(self
            .scope_inventory_for(searcher)?
            .terms(&prefix, self.schema.file_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexing::pipeline::FileRegistration;
    use crate::parsing::LanguageId;
    use crate::{FileId, Settings};

    fn register(index: &DocumentIndex, id: u32, path: &str) {
        index
            .store_file_registration(&FileRegistration {
                path: path.into(),
                file_id: FileId::new(id).unwrap(),
                content_hash: "fixture".into(),
                language_id: LanguageId::new("rust"),
                timestamp: 0,
                mtime: 0,
            })
            .unwrap();
    }

    #[test]
    fn scope_snapshot_survives_reload_and_does_not_roll_back_newer_cache() {
        let temp = tempfile::tempdir().unwrap();
        let index = DocumentIndex::new(temp.path(), &Settings::default()).unwrap();
        index.start_batch().unwrap();
        register(&index, 1, "src/old.rs");
        index.commit_batch().unwrap();
        let old = index.reader.searcher();
        assert_eq!(index.file_scope_terms(&old, "src").unwrap().len(), 1);
        index.start_batch().unwrap();
        register(&index, 2, "src/new.rs");
        index.commit_batch().unwrap();
        let new = index.reader.searcher();
        assert_ne!(
            old.generation().generation_id(),
            new.generation().generation_id()
        );
        assert_eq!(index.file_scope_terms(&new, "src").unwrap().len(), 2);
        assert_eq!(index.file_scope_terms(&old, "src").unwrap().len(), 1);
        assert_eq!(
            index
                .scope_inventory
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .generation,
            new.generation().generation_id()
        );
    }

    #[test]
    fn scope_inventory_reuses_one_allocation_across_distinct_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        let index = DocumentIndex::new(temp.path(), &Settings::default()).unwrap();
        index.start_batch().unwrap();
        for id in 1..=2048 {
            register(&index, id, &format!("src/group{}/{id}.rs", id % 8));
        }
        index.commit_batch().unwrap();
        let searcher = index.reader.searcher();
        let cold = index.scope_inventory_for(&searcher).unwrap();
        for group in 0..8 {
            assert_eq!(
                index
                    .file_scope_terms(&searcher, &format!("src/group{group}"))
                    .unwrap()
                    .len(),
                256
            );
            let warm = index.scope_inventory_for(&searcher).unwrap();
            assert!(Arc::ptr_eq(&cold, &warm));
        }
        println!(
            "scope_inventory: files={} logical_bytes={} warm_queries=8 shared_inventory=true",
            cold.entries.len(),
            cold.logical_bytes
        );
    }

    #[test]
    fn scope_cache_budget_does_not_truncate_uncached_results() {
        let temp = tempfile::tempdir().unwrap();
        let index = DocumentIndex::new(temp.path(), &Settings::default()).unwrap();
        index.start_batch().unwrap();
        let long = "a".repeat(180);
        for id in 1..=20_000 {
            register(&index, id, &format!("src/{long}/{id}.rs"));
        }
        index.commit_batch().unwrap();
        let searcher = index.reader.searcher();
        assert_eq!(
            index.file_scope_terms(&searcher, "src").unwrap().len(),
            20_000
        );
        assert!(index.scope_inventory.read().unwrap().is_none());
    }
}
