//! Conservative bootstrap admission, not a second index planner. Count every
//! visited entry and byte for resource limits, but require an enabled source
//! language before starting a child. The normal pipeline remains authoritative.
use crate::Settings;
use crate::parsing::{generic_pack, get_registry};
use rmcp::model::ErrorData;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub(super) fn has_sources(
    settings: &Settings,
    roots: &[PathBuf],
    ct: &CancellationToken,
) -> Result<bool, ErrorData> {
    let fail = super::super::internal;
    if ct.is_cancelled() {
        return Err(fail("Initial source discovery cancelled"));
    }
    let first = roots.first().ok_or_else(|| fail("No bootstrap source root"))?;
    // Snapshot registered extensions once. Never hold the registry lock over
    // filesystem traversal or instantiate parsers/embedding facilities here.
    let extensions: HashSet<&'static str> = get_registry()
        .lock()
        .map_err(|_| fail("Language registry lock is poisoned"))?
        .enabled_extensions(settings)
        .collect();
    let mut walker = ignore::WalkBuilder::new(first);
    for root in roots.iter().skip(1) {
        walker.add(root);
    }
    walker
        .hidden(false)
        .follow_links(false)
        .parents(false)
        .git_global(false)
        .require_git(false)
        .add_custom_ignore_filename(".codannaignore");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut files = 0u32;
    let mut bytes = 0u64;
    let mut found_source = false;
    for (entries, entry) in walker.build().enumerate() {
        if ct.is_cancelled() || Instant::now() >= deadline || entries >= 100_000 {
            return Err(fail("Initial source discovery exceeded its budget; use explicit codanna index"));
        }
        let entry = entry.map_err(|_| fail("Initial source discovery failed"))?;
        // `ignore` can return an entry AND a malformed-ignore-file error.
        // Ignoring that error would silently change the user's source policy.
        if entry.error().is_some() {
            return Err(fail("Invalid ignore rules during initial source discovery"));
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        files += 1;
        bytes = bytes.saturating_add(
            entry.metadata().map_err(|_| fail("Cannot inspect bootstrap source"))?.len(),
        );
        if files > super::MAX_FILES || bytes > 512 * 1024 * 1024 {
            return Err(fail("Workspace exceeds automatic indexing limits (50,000 files / 512 MiB); use explicit codanna index"));
        }
        // Match the pipeline's filename/extension rules, including enabled
        // language packs. Unsupported files still count against admission limits.
        if entry.file_name().to_str().is_some_and(|name| name.starts_with('.')) {
            continue;
        }
        let registered = entry.path().extension().and_then(|ext| ext.to_str())
            .is_some_and(|ext| extensions.contains(ext));
        found_source |= registered || generic_pack::detect_path(entry.path(), settings).is_some();
    }
    Ok(found_source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn hardening_workspace_discovery_waits_for_enabled_sources_without_starting_a_child() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let settings = Settings::default();
        let token = CancellationToken::new();
        fs::write(root.join("notes.not_a_code_language"), "notes").unwrap();
        assert!(!has_sources(&settings, std::slice::from_ref(&root), &token).unwrap());
        fs::write(root.join("lib.rs"), "pub fn identity() {}\n").unwrap();
        assert!(has_sources(&settings, std::slice::from_ref(&root), &token).unwrap());
        let mut disabled = settings;
        disabled.languages.get_mut("rust").unwrap().enabled = false;
        assert!(!has_sources(&disabled, std::slice::from_ref(&root), &token).unwrap());
    }

    #[test]
    fn hardening_workspace_discovery_respects_ignores_and_checks_hidden_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let settings = Settings::default();
        let token = CancellationToken::new();
        fs::create_dir(root.join("excluded")).unwrap();
        fs::write(root.join("excluded/lib.rs"), "pub fn ignored() {}\n").unwrap();
        fs::write(root.join(".codannaignore"), "excluded/\n").unwrap();
        assert!(!has_sources(&settings, std::slice::from_ref(&root), &token).unwrap());
        fs::create_dir(root.join(".sources")).unwrap();
        fs::write(root.join(".sources/lib.rs"), "pub fn included() {}\n").unwrap();
        assert!(has_sources(&settings, std::slice::from_ref(&root), &token).unwrap());
        token.cancel();
        assert!(has_sources(&settings, &[root], &token).is_err());
    }
}
