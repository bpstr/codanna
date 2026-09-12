//! File system walker for discovering source files to index
//!
//! This module provides efficient directory traversal with support for:
//! - .gitignore rules
//! - Custom ignore patterns from configuration
//! - Language filtering
//! - Hidden file handling

use crate::parsing::get_registry;
use crate::{IndexError, IndexResult, Settings};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Walks directories to find source files to index
#[derive(Debug)]
pub struct FileWalker {
    settings: Arc<Settings>,
}

impl FileWalker {
    /// Create a new file walker with the given settings
    pub fn new(settings: Arc<Settings>) -> Self {
        Self { settings }
    }

    /// Walker configuration shared by the file and directory walks:
    /// .gitignore chains (+ global, + exclude), .codannaignore, no
    /// symlink following, gitignore active outside git repos.
    fn configured_builder(root: &Path) -> WalkBuilder {
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            .max_depth(None)
            .require_git(false);
        builder.add_custom_ignore_filename(".codannaignore");
        builder
    }

    /// Walk a directory, preserving discovery failures instead of treating them
    /// as an empty subtree. Callers must consume the complete iterator to claim
    /// a complete inventory (including when applying an output limit).
    pub fn walk(&self, root: &Path) -> Box<dyn Iterator<Item = IndexResult<PathBuf>>> {
        let enabled_extensions = match self.get_enabled_extensions() {
            Ok(extensions) => extensions,
            Err(error) => return Box::new(std::iter::once(Err(error))),
        };
        let root = root.to_path_buf();
        Box::new(
            Self::configured_builder(&root)
                .build()
                .filter_map(move |entry| {
                    let entry = match checked_entry(entry, &root) {
                        Ok(entry) => entry,
                        Err(error) => return Some(Err(error)),
                    };
                    if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                        return None;
                    }
                    let path = entry.path();
                    if path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with('.'))
                    {
                        return None;
                    }
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .filter(|ext| enabled_extensions.iter().any(|enabled| enabled == ext))
                        .map(|_| Ok(path.to_path_buf()))
                }),
        )
    }

    /// Directories visited under `root`, with the same ignore chains as `walk`.
    /// Errors (including invalid ignore rules) are yielded before filtering.
    pub fn walk_dirs(&self, root: &Path) -> impl Iterator<Item = IndexResult<PathBuf>> {
        let root = root.to_path_buf();
        Self::configured_builder(&root)
            .build()
            .filter_map(move |entry| {
                let entry = match checked_entry(entry, &root) {
                    Ok(entry) => entry,
                    Err(error) => return Some(Err(error)),
                };
                if !entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    return None;
                }
                (entry.depth() == 0
                    || entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| !name.starts_with('.')))
                .then(|| Ok(entry.path().to_path_buf()))
            })
    }

    fn get_enabled_extensions(&self) -> IndexResult<Vec<String>> {
        let registry = get_registry()
            .lock()
            .map_err(|_| IndexError::MutexPoisoned)?;
        Ok(registry
            .enabled_extensions(&self.settings)
            .map(str::to_owned)
            .collect())
    }

    /// Count a complete inventory or return its discovery error.
    pub fn count_files(&self, root: &Path) -> IndexResult<usize> {
        self.walk(root)
            .try_fold(0, |count, entry| entry.map(|_| count + 1))
    }
}

/// `ignore` can attach a partial ignore-file error to an otherwise valid entry.
/// Dropping that error would silently change the inventory's exclusion rules.
fn checked_entry(
    entry: Result<ignore::DirEntry, ignore::Error>,
    root: &Path,
) -> IndexResult<ignore::DirEntry> {
    let entry = entry.map_err(|error| IndexError::Discovery {
        path: root.to_path_buf(),
        reason: error.to_string(),
    })?;
    if let Some(error) = entry.error() {
        return Err(IndexError::Discovery {
            path: entry.path().to_path_buf(),
            reason: error.to_string(),
        });
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_settings() -> Arc<Settings> {
        let mut settings = Settings::default();
        // Disable Python and PHP for testing (only Rust enabled)
        settings.languages.get_mut("python").unwrap().enabled = false;
        settings.languages.get_mut("php").unwrap().enabled = false;
        // Rust remains enabled by default
        Arc::new(settings)
    }

    #[test]
    fn test_walk_directory() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create some test files
        fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("lib.rs"), "pub fn lib() {}").unwrap();
        fs::write(root.join("test.py"), "def test(): pass").unwrap();
        fs::write(root.join("README.md"), "# Test").unwrap();

        let settings = create_test_settings();
        let walker = FileWalker::new(settings);

        let files: Vec<_> = walker.walk(root).collect::<IndexResult<Vec<_>>>().unwrap();

        // Should find only Rust files (Python and PHP disabled in test settings)
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p.ends_with("main.rs")));
        assert!(files.iter().any(|p| p.ends_with("lib.rs")));
    }

    #[test]
    fn test_ignore_hidden_files() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create hidden file and visible file
        fs::write(root.join(".hidden.rs"), "fn hidden() {}").unwrap();
        fs::write(root.join("visible.rs"), "fn visible() {}").unwrap();

        let settings = create_test_settings();
        let walker = FileWalker::new(settings);

        let files: Vec<_> = walker.walk(root).collect::<IndexResult<Vec<_>>>().unwrap();

        // Should only find the visible file (hidden files are filtered out)
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("visible.rs"));
    }

    // walk_dirs feeds the watcher's directory-chain registration: a new
    // directory subtree gets watches only where the index walk would
    // traverse -- an ignored tree (node_modules, generated/) is pruned
    // by the same chains before any kernel watch is added.
    #[test]
    fn walk_dirs_prunes_ignored_subtrees() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("newmod/empty_sub")).unwrap();
        fs::create_dir_all(root.join("generated/deep")).unwrap();
        fs::write(root.join(".gitignore"), "generated/\n").unwrap();

        let settings = create_test_settings();
        let walker = FileWalker::new(settings);

        let mut dirs: Vec<_> = walker
            .walk_dirs(root)
            .map(|p| p.unwrap().strip_prefix(root).unwrap().to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        dirs.sort();

        assert_eq!(
            dirs,
            vec![
                std::path::PathBuf::from("newmod"),
                std::path::PathBuf::from("newmod/empty_sub"),
            ],
            "empty traversable dirs are yielded, ignored subtrees pruned"
        );
    }

    #[test]
    fn test_gitignore_respected() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create .gitignore (should work without git init due to require_git(false))
        fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();

        // Create files
        fs::write(root.join("ignored.rs"), "fn ignored() {}").unwrap();
        fs::write(root.join("included.rs"), "fn included() {}").unwrap();

        let settings = create_test_settings();
        let walker = FileWalker::new(settings);

        let files: Vec<_> = walker.walk(root).collect::<IndexResult<Vec<_>>>().unwrap();

        // Should only find the included file
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("included.rs"));
    }
    #[test]
    fn hardening_review_walker_missing_root_is_not_empty_success() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("missing");
        let walker = FileWalker::new(create_test_settings());
        assert!(matches!(
            walker.walk(&missing).collect::<IndexResult<Vec<_>>>(),
            Err(IndexError::Discovery { .. })
        ));
        assert!(matches!(
            walker.walk_dirs(&missing).collect::<IndexResult<Vec<_>>>(),
            Err(IndexError::Discovery { .. })
        ));
        assert!(walker.count_files(&missing).is_err());
    }

    #[test]
    fn hardening_review_walker_invalid_ignore_rules_are_reported() {
        let dir = TempDir::new().unwrap();
        let invalid_rule = "[z-a]";
        let mut rules = ignore::gitignore::GitignoreBuilder::new(dir.path());
        assert!(
            rules.add_line(None, invalid_rule).is_err(),
            "fixture must be invalid gitignore syntax"
        );
        fs::write(
            dir.path().join(".codannaignore"),
            format!("{invalid_rule}\n"),
        )
        .unwrap();
        fs::write(dir.path().join("file.rs"), "fn fixture() {}\n").unwrap();
        let walker = FileWalker::new(create_test_settings());
        assert!(
            walker
                .walk(dir.path())
                .collect::<IndexResult<Vec<_>>>()
                .is_err()
        );
        assert!(
            walker
                .walk_dirs(dir.path())
                .collect::<IndexResult<Vec<_>>>()
                .is_err()
        );
    }
}
