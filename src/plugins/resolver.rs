//! Git repository operations for plugin fetching
//!
//! Delegates to crate::git which uses system git binary.

use super::error::{PluginError, PluginResult};
use crate::git::GitError;
use std::path::Path;

fn map_git_error(e: GitError, operation: &str) -> PluginError {
    let detail = match &e {
        GitError::CommandFailed { .. } => e.message(),
        _ => format!("{operation}: {}", e.message()),
    };
    PluginError::GitOperationFailed { operation: detail }
}

/// Clone a repository with shallow depth. Returns commit SHA.
pub fn clone_repository(
    repo_url: &str,
    target_dir: &Path,
    git_ref: Option<&str>,
) -> PluginResult<String> {
    let commit = crate::git::clone_repository(repo_url, target_dir, git_ref)
        .map_err(|e| map_git_error(e, "clone"))?;
    // Validate even a whole-repository plugin, before reading its manifests.
    validate_tree(target_dir)?;
    Ok(commit)
}

/// Resolve a git reference to a commit SHA without cloning.
pub fn resolve_reference(repo_url: &str, git_ref: &str) -> PluginResult<String> {
    crate::git::resolve_reference(repo_url, git_ref).map_err(|e| match e {
        GitError::ReferenceNotFound { ref_name, .. } => PluginError::InvalidReference {
            ref_name,
            reason: "Reference not found in repository".to_string(),
        },
        other => map_git_error(other, "resolve reference"),
    })
}

/// Extract a subdirectory from a cloned repository
pub fn extract_subdirectory(repo_dir: &Path, subdir: &str, target_dir: &Path) -> PluginResult<()> {
    use std::path::Component;
    if Path::new(subdir).is_absolute()
        || Path::new(subdir).components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(unsafe_path(Path::new(subdir)));
    }
    // Preflight the complete clone before copying anything. This also rejects
    // intermediate directory links and preserves the direct-call boundary.
    validate_tree(repo_dir)?;
    let source_dir = repo_dir.join(subdir);

    if !source_dir.exists() {
        return Err(PluginError::PluginNotFound {
            name: subdir.to_string(),
        });
    }

    // Create target directory
    std::fs::create_dir_all(target_dir)?;

    // Copy subdirectory contents
    copy_dir_contents(&source_dir, target_dir)?;

    Ok(())
}

fn unsafe_path(path: &Path) -> PluginError {
    PluginError::InvalidPluginManifest {
        reason: format!(
            "Plugin trees must contain only regular files and directories, without symlinks or path escapes: {}",
            path.display()
        ),
    }
}

/// Reject links and special files without dereferencing them. Git cannot
/// provide hard links; a fresh private clone is the trusted staging boundary.
fn validate_tree(root: &Path) -> PluginResult<()> {
    let metadata = std::fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() {
        return Err(unsafe_path(root));
    }
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(unsafe_path(root));
    }
    for entry in std::fs::read_dir(root)? {
        validate_tree(&entry?.path())?;
    }
    Ok(())
}

/// Copy only preflighted regular files; recheck entry types during the copy.
fn copy_dir_contents(source: &Path, dest: &Path) -> PluginResult<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let source_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if kind.is_dir() {
            std::fs::create_dir_all(&dest_path)?;
            copy_dir_contents(&source_path, &dest_path)?;
        } else if kind.is_file() {
            std::fs::copy(&source_path, &dest_path)?;
        } else {
            return Err(unsafe_path(&source_path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    #[ignore] // Requires network
    fn test_resolve_reference() {
        // Test resolving a tag in a public repo
        let result = resolve_reference("https://github.com/rust-lang/rust.git", "1.0.0");
        assert!(result.is_ok());

        // Test invalid reference
        let result = resolve_reference("https://github.com/rust-lang/rust.git", "nonexistent-ref");
        assert!(result.is_err());
    }

    #[test]
    #[ignore] // Requires network
    fn test_clone_repository() {
        let temp_dir = tempdir().unwrap();
        let clone_path = temp_dir.path().join("test-repo");

        // Clone a small public repo
        let result = clone_repository(
            "https://github.com/rust-lang/rustlings.git",
            &clone_path,
            Some("main"),
        );

        assert!(result.is_ok());
        assert!(clone_path.exists());
        assert!(clone_path.join(".git").exists());

        // Verify we got a commit SHA
        let sha = result.unwrap();
        assert_eq!(sha.len(), 40); // Git SHA is 40 hex chars
    }

    #[test]
    fn test_extract_subdirectory() {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");

        // Create test structure
        std::fs::create_dir_all(source_dir.join("subdir")).unwrap();
        std::fs::write(source_dir.join("subdir/file.txt"), "test content").unwrap();

        // Extract subdirectory
        let result = extract_subdirectory(&source_dir, "subdir", &target_dir);
        assert!(result.is_ok());
        assert!(target_dir.join("file.txt").exists());

        // Test non-existent subdirectory
        let result = extract_subdirectory(&source_dir, "nonexistent", &target_dir);
        assert!(matches!(result, Err(PluginError::PluginNotFound { .. })));
    }

    #[test]
    fn hardening_review_plugin_extraction_rejects_parent_and_absolute_paths() {
        let root = tempdir().unwrap();
        let target = root.path().join("out");
        assert!(extract_subdirectory(root.path(), "..", &target).is_err());
        assert!(extract_subdirectory(root.path(), "/", &target).is_err());
        assert!(!target.exists());
    }

    #[test]
    #[cfg(unix)]
    fn hardening_review_plugin_extraction_rejects_all_symlink_shapes_before_copy() {
        for link_target in [
            "absolute-file",
            "parent-file",
            "directory",
            "intermediate",
            "internal",
        ] {
            let root = tempdir().unwrap();
            let source = root.path().join("clone");
            let out = root.path().join("installed");
            let secret = root.path().join("secret.txt");
            std::fs::write(&secret, "host secret").unwrap();
            std::fs::create_dir_all(source.join("subdir")).unwrap();
            std::fs::write(source.join("subdir/real.txt"), "safe").unwrap();
            let (target, link) = match link_target {
                "absolute-file" => (secret.clone(), source.join("subdir/link")),
                "parent-file" => ("../../secret.txt".into(), source.join("subdir/link")),
                "directory" => (root.path().to_path_buf(), source.join("subdir/link")),
                "intermediate" => (root.path().to_path_buf(), source.join("escape")),
                _ => ("real.txt".into(), source.join("subdir/link")),
            };
            std::os::unix::fs::symlink(target, link).unwrap();
            assert!(
                extract_subdirectory(&source, "subdir", &out).is_err(),
                "{link_target}"
            );
            assert!(
                !out.exists(),
                "preflight must precede copying: {link_target}"
            );
        }
    }
}
