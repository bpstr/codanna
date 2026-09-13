//! Indexed-path management: the indexed_paths list and its canonicalized cache.

use super::Settings;
use std::path::{Path, PathBuf};

impl Settings {
    fn canonical_indexed_path(&self, path: &Path) -> PathBuf {
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(root) = &self.workspace_root {
            root.join(path)
        } else {
            path.to_path_buf()
        };
        resolved.canonicalize().unwrap_or(resolved)
    }

    fn portable_indexed_path(&self, canonical_path: &Path) -> PathBuf {
        let Some(root) = &self.workspace_root else {
            return canonical_path.to_path_buf();
        };
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        canonical_path
            .strip_prefix(&root)
            .map(|relative| {
                if relative.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    relative.to_path_buf()
                }
            })
            .unwrap_or_else(|_| canonical_path.to_path_buf())
    }

    /// The cache is the comparison surface (strip-base selection tests
    /// canonicalized file paths against it); hand-edited or legacy entries
    /// may carry symlink components, so canonicalize here. The serialized
    /// `indexing.indexed_paths` stays verbatim to round-trip the user's file.
    pub(super) fn sync_indexed_path_cache(&mut self) {
        self.indexed_paths_cache = self
            .indexing
            .indexed_paths
            .iter()
            .map(|path| self.canonical_indexed_path(path))
            .collect();
    }

    /// Add a folder to the list of indexed paths
    pub fn add_indexed_path(&mut self, path: PathBuf) -> Result<(), String> {
        // Canonicalize the path to avoid duplicates
        let canonical_path = path
            .canonicalize()
            .map_err(|e| format!("Invalid path: {e}"))?;

        // Track whether we should remove child paths that are covered by the new entry
        let mut has_descendants = false;

        // Check if path already exists or is covered by an existing parent
        for existing in &self.indexed_paths_cache {
            if *existing == canonical_path {
                return Err(format!("Path already indexed: {}", path.display()));
            }

            // If an existing entry is an ancestor of the new path, treat as already indexed
            if canonical_path.starts_with(existing) {
                return Err(format!(
                    "Path already indexed: {} (covered by {})",
                    path.display(),
                    crate::parsing::paths::render_absolute_path(existing).display()
                ));
            }

            // Record descendant paths so we can prune them before inserting the parent
            if existing.starts_with(&canonical_path) {
                has_descendants = true;
            }
        }

        if has_descendants {
            // The serialized and canonical vectors are aligned. Remove child
            // entries by their canonical identity even when the stored form is
            // repository-relative.
            for index in (0..self.indexed_paths_cache.len()).rev() {
                if self.indexed_paths_cache[index].starts_with(&canonical_path) {
                    self.indexed_paths_cache.remove(index);
                    self.indexing.indexed_paths.remove(index);
                }
            }
        }

        // Keep repository-owned settings portable while the comparison cache
        // retains the canonical absolute identity.
        self.indexing
            .indexed_paths
            .push(self.portable_indexed_path(&canonical_path));
        self.indexed_paths_cache.push(canonical_path);
        Ok(())
    }

    /// Remove a folder from the list of indexed paths
    pub fn remove_indexed_path(&mut self, path: &Path) -> Result<(), String> {
        let canonical_path = path
            .canonicalize()
            .map_err(|e| format!("Invalid path: {e}"))?;

        let Some(index) = self
            .indexed_paths_cache
            .iter()
            .position(|existing| existing == &canonical_path)
        else {
            return Err(format!(
                "Path not found in indexed paths: {}",
                path.display()
            ));
        };
        self.indexed_paths_cache.remove(index);
        self.indexing.indexed_paths.remove(index);

        Ok(())
    }

    /// Get all indexed paths
    /// Returns empty vector if none are configured (maintains backward compatibility)
    pub fn get_indexed_paths(&self) -> Vec<PathBuf> {
        self.indexed_paths_cache.clone()
    }
}
