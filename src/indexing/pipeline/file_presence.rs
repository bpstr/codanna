//! Complete file-row evidence with delta maintenance for the warm watcher cache.

use super::{PipelineResult, SymbolLookupCache};
use crate::{FileId, IndexError, parsing::resolution::FilePresence, storage::DocumentIndex};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(super) struct FileInventory {
    complete: bool,
    by_id: HashMap<FileId, PathBuf>,
    // Counts tolerate duplicate paths in older indexes without manufacturing absence.
    paths: BTreeMap<PathBuf, usize>,
    stems: HashMap<PathBuf, usize>,
}

impl FileInventory {
    pub(super) fn replace(&mut self, id: FileId, path: Option<PathBuf>) {
        if let Some(old) = self.by_id.remove(&id) {
            if let Some(key) = stem_key(&old)
                && let Some(count) = self.stems.get_mut(&key)
            {
                *count -= 1;
                if *count == 0 {
                    self.stems.remove(&key);
                }
            }
            if let Some(count) = self.paths.get_mut(&old) {
                *count -= 1;
                if *count == 0 {
                    self.paths.remove(&old);
                }
            }
        }
        if let Some(path) = path {
            if let Some(key) = stem_key(&path) {
                *self.stems.entry(key).or_default() += 1;
            }
            *self.paths.entry(path.clone()).or_default() += 1;
            self.by_id.insert(id, path);
        }
    }

    fn answer(&self, present: bool) -> FilePresence {
        if present {
            FilePresence::Present
        } else if self.complete {
            FilePresence::Absent
        } else {
            FilePresence::Unknown
        }
    }

    fn sibling(&self, path: &Path) -> FilePresence {
        let Some(key) = stem_key(path) else {
            return FilePresence::Unknown;
        };
        self.answer(self.stems.contains_key(&key))
    }
}

fn stem_key(path: &Path) -> Option<PathBuf> {
    let stem = path.file_name()?.to_str()?.split('.').next()?;
    Some(path.parent()?.join(stem))
}

#[derive(Clone, Copy)]
pub(super) enum InventoryQuery {
    File,
    Sibling,
    Directory,
}

impl SymbolLookupCache {
    pub(super) fn load_file_inventory(&self, index: &DocumentIndex) -> PipelineResult<()> {
        let mut inventory = FileInventory::default();
        index.for_each_file_path::<crate::storage::StorageError>(|id, path| {
            inventory.replace(id, Some(path));
            Ok(())
        })?;
        inventory.complete = true; // Only after the entire fallible visitor succeeds.
        *self
            .file_inventory
            .write()
            .map_err(|_| IndexError::MutexPoisoned)? = inventory;
        Ok(())
    }

    pub(super) fn inventory_presence(&self, path: &Path, query: InventoryQuery) -> FilePresence {
        let Ok(inventory) = self.file_inventory.read() else {
            return FilePresence::Unknown;
        };
        match query {
            InventoryQuery::File => inventory.answer(inventory.paths.contains_key(path)),
            InventoryQuery::Sibling => inventory.sibling(path),
            InventoryQuery::Directory => inventory.answer(
                inventory
                    .paths
                    .range(path.to_path_buf()..)
                    .next()
                    .is_some_and(|(candidate, _)| candidate != path && candidate.starts_with(path)),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_repair_inventory_distinguishes_partial_absent_and_similar_paths() {
        let mut inventory = FileInventory::default();
        assert_eq!(
            inventory.sibling(Path::new("a/service.js")),
            FilePresence::Unknown
        );
        inventory.replace(FileId::new(1).unwrap(), Some("a/service.client.ts".into()));
        inventory.complete = true;
        assert_eq!(
            inventory.sibling(Path::new("a/service.js")),
            FilePresence::Present
        );
        assert_eq!(
            inventory.sibling(Path::new("b/service.js")),
            FilePresence::Absent
        );
        assert_eq!(
            inventory.sibling(Path::new("a/service2.js")),
            FilePresence::Absent
        );
        inventory.replace(FileId::new(1).unwrap(), Some("a/renamed.ts".into()));
        assert_eq!(
            inventory.sibling(Path::new("a/service.js")),
            FilePresence::Absent
        );
    }
}
