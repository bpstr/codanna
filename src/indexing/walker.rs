//! File system walker for discovering source files to index
//!
//! This module provides efficient directory traversal with support for:
//! - .gitignore rules
//! - Custom ignore patterns from configuration
//! - Language filtering
//! - Hidden file handling

use crate::parsing::{generic_pack, get_registry};
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
        let settings = self.settings.clone();
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
                    if path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| enabled_extensions.iter().any(|enabled| enabled == ext))
                    {
                        return Some(Ok(path.to_path_buf()));
                    }
                    generic_pack::detect_path(path, &settings).map(|_| Ok(path.to_path_buf()))
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

    /// Read an authorized, bounded inventory before a network mutation starts.
    /// Every visited entry counts, even when its extension is not indexable.
    /// Prepared bytes are consumed directly by the pipeline; it cannot rewalk a
    /// changed directory or reopen a swapped symlink after this preflight.
    pub(crate) fn snapshot(
        &self,
        roots: &[PathBuf],
        max_entries: usize,
        max_files: usize,
        max_bytes: usize,
    ) -> IndexResult<Vec<crate::indexing::pipeline::FileContent>> {
        use crate::indexing::{file_info::calculate_hash, pipeline::FileContent};
        use std::{collections::HashSet, io::Read};
        let fail = |path: &Path, message: String| IndexError::Discovery {
            path: path.to_path_buf(),
            reason: message,
        };
        let extensions = self.get_enabled_extensions()?;
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        let mut entries = 0usize;
        let mut bytes = 0usize;
        for root in roots {
            for entry in Self::configured_builder(root).build() {
                entries += 1;
                if entries > max_entries {
                    return Err(fail(root, "reindex entry budget exceeded".into()));
                }
                let entry = checked_entry(entry, root)?;
                let Some(kind) = entry.file_type() else {
                    return Err(fail(entry.path(), "missing file type".into()));
                };
                if kind.is_symlink() {
                    return Err(fail(
                        entry.path(),
                        "reindex does not follow symlinks".into(),
                    ));
                }
                if !kind.is_file()
                    || entry
                        .file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with('.'))
                {
                    continue;
                }
                let registered = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| extensions.iter().any(|x| x == e));
                if !registered && generic_pack::detect_path(entry.path(), &self.settings).is_none()
                {
                    continue;
                }
                let path = entry
                    .path()
                    .canonicalize()
                    .map_err(|e| fail(entry.path(), e.to_string()))?;
                if !roots.iter().any(|root| path.starts_with(root)) {
                    return Err(fail(&path, "source escaped authorized roots".into()));
                }
                if !seen.insert(path.clone()) {
                    continue;
                }
                if files.len() == max_files {
                    return Err(fail(&path, "reindex file budget exceeded".into()));
                }
                let file = std::fs::File::open(&path).map_err(|e| fail(&path, e.to_string()))?;
                let opened = file.metadata().map_err(|e| fail(&path, e.to_string()))?;
                let named =
                    std::fs::symlink_metadata(&path).map_err(|e| fail(&path, e.to_string()))?;
                if !opened.is_file() || !named.is_file() {
                    return Err(fail(&path, "source changed type during preflight".into()));
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if opened.dev() != named.dev() || opened.ino() != named.ino() {
                        return Err(fail(
                            &path,
                            "source changed identity during preflight".into(),
                        ));
                    }
                }
                let remaining = max_bytes.saturating_sub(bytes).min(32 * 1024 * 1024);
                let mut content = String::new();
                file.take(remaining as u64 + 1)
                    .read_to_string(&mut content)
                    .map_err(|e| fail(&path, e.to_string()))?;
                if content.len() > remaining {
                    return Err(fail(&path, "reindex source byte budget exceeded".into()));
                }
                if path
                    .canonicalize()
                    .map_err(|e| fail(&path, e.to_string()))?
                    != path
                {
                    return Err(fail(
                        &path,
                        "source changed containment during preflight".into(),
                    ));
                }
                bytes += content.len();
                let hash = calculate_hash(&content);
                files.push(FileContent::new(path, content, hash));
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
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

        // Rich Python is disabled; Rust and generic Markdown remain discoverable.
        assert_eq!(files.len(), 3);
        assert!(files.iter().any(|p| p.ends_with("main.rs")));
        assert!(files.iter().any(|p| p.ends_with("lib.rs")));
        assert!(files.iter().any(|p| p.ends_with("README.md")));
        assert!(!files.iter().any(|p| p.ends_with("test.py")));
    }

    #[test]
    fn generic_languages_are_discovered_and_snapshotted() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().canonicalize().unwrap();
        let source = root.join("main.zig");
        fs::write(&source, "pub fn main() void {}\n").unwrap();

        let walker = FileWalker::new(Arc::new(Settings::default()));
        let files = walker.walk(&root).collect::<IndexResult<Vec<_>>>().unwrap();
        assert_eq!(files, vec![source.clone()]);

        let snapshot = walker.snapshot(&[root], 10, 10, 1024).unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].path, source);
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

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn hardening_final_reindex_snapshot_budgets_count_unindexable_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("one.rs"), "pub fn one() {}\n").unwrap();
        std::fs::write(root.join("two.rs"), "pub fn two() {}\n").unwrap();
        std::fs::write(root.join("notes.txt"), "not indexed").unwrap();
        let walker = FileWalker::new(Arc::new(Settings::default()));
        for (entries, files, bytes, expected) in [
            (2, 10, 1024, "entry"),
            (10, 1, 1024, "file"),
            (10, 10, 1, "byte"),
        ] {
            let error = walker
                .snapshot(std::slice::from_ref(&root), entries, files, bytes)
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
        let sources = walker.snapshot(&[root.clone(), root], 20, 2, 1024).unwrap();
        assert_eq!(
            sources.len(),
            2,
            "overlapping roots cannot duplicate source work"
        );
    }
    #[test]
    fn hardening_final_reindex_consumes_captured_bytes_and_resolves_cross_file_edges() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("src");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("a.rs"), "pub fn caller() { target(); }\n").unwrap();
        std::fs::write(source.join("b.rs"), "pub fn target() {}\n").unwrap();
        let settings = Arc::new(Settings {
            workspace_root: Some(root.clone()),
            index_path: root.join("index"),
            ..Settings::default()
        });
        let sources = FileWalker::new(Arc::clone(&settings))
            .snapshot(std::slice::from_ref(&source), 10, 2, 1024)
            .unwrap();
        std::fs::write(
            source.join("b.rs"),
            "pub fn replaced_after_preflight() {}\n",
        )
        .unwrap();
        let mut facade = crate::indexing::facade::IndexFacade::new(settings).unwrap();
        let mut pending = crate::indexing::pipeline::PendingResolution::default();
        for file in sources {
            facade.index_prepared_file(file, &mut pending).unwrap();
        }
        facade.resolve_deferred(pending).unwrap();
        assert!(
            facade
                .find_symbols_by_name("replaced_after_preflight", None)
                .is_empty()
        );
        let caller = facade.find_symbols_by_name("caller", None).remove(0);
        let target = facade.find_symbols_by_name("target", None).remove(0);
        assert!(
            facade
                .get_called_functions(caller.id)
                .iter()
                .any(|symbol| symbol.id == target.id)
        );
    }
    #[cfg(unix)]
    #[test]
    fn hardening_final_snapshot_rejects_symlink_before_any_index_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "fn secret() {}").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.rs"), root.join("escape.rs"))
            .unwrap();
        assert!(
            FileWalker::new(Arc::new(Settings::default()))
                .snapshot(&[root], 10, 10, 1024)
                .is_err()
        );
    }
}
