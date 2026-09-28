//! Bounded, read-only comparison of committed document provenance and sources.

use rmcp::schemars;
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::store::{
    DocumentStoreError, SourceDriftEntry, SourceDriftReport, StoreResult, normalize_source_path,
};
use super::{CollectionConfig, FileState};
use crate::Settings;

/// Limits include discovery work as well as source content reads.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentDriftRequest {
    pub collection: String,
    #[serde(default = "default_files")]
    #[schemars(range(min = 1, max = 1000))]
    pub max_files: usize,
    #[serde(default = "default_bytes")]
    #[schemars(range(min = 1, max = 67108864))]
    pub max_bytes: usize,
    #[serde(default = "default_entries")]
    #[schemars(range(min = 1, max = 100000))]
    pub max_entries: usize,
}
fn default_files() -> usize {
    100
}
fn default_bytes() -> usize {
    8 * 1024 * 1024
}
fn default_entries() -> usize {
    10_000
}
impl DocumentDriftRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.collection.trim().is_empty() {
            return Err("collection must not be empty".into());
        }
        for (name, value, cap) in [
            ("max_files", self.max_files, 1000),
            ("max_bytes", self.max_bytes, 64 * 1024 * 1024),
            ("max_entries", self.max_entries, 100_000),
        ] {
            if !(1..=cap).contains(&value) {
                return Err(format!("{name} must be between 1 and {cap}"));
            }
        }
        Ok(())
    }
}

pub fn inspect_source_drift(
    settings: &Settings,
    request: &DocumentDriftRequest,
) -> StoreResult<SourceDriftReport> {
    inspect(settings, request, None)
}

pub(crate) fn inspect(
    settings: &Settings,
    request: &DocumentDriftRequest,
    boundary: Option<&Path>,
) -> StoreResult<SourceDriftReport> {
    request.validate().map_err(DocumentStoreError::Index)?;
    let boundary = boundary.map(Path::canonicalize).transpose()?;
    let boundary = boundary.as_deref();
    let config = settings
        .documents
        .collections
        .get(&request.collection)
        .ok_or_else(|| {
            DocumentStoreError::Index(format!(
                "Unknown document collection '{}'",
                request.collection
            ))
        })?;
    let workspace = settings
        .workspace_root
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let index = normalize_source_path(&workspace.join(&settings.index_path));
    if let Some(root) = boundary {
        check_boundary(root, &index)?;
    }
    let (generation, states) =
        super::store::load_source_snapshot(&index.join("documents"), boundary)?;
    if let Some(root) = boundary {
        for path in states.keys() {
            check_boundary(root, path)?;
        }
    }
    let config = CollectionConfig {
        paths: config
            .paths
            .iter()
            .map(|path| workspace.join(path))
            .collect(),
        ..config.clone()
    };
    let (paths, entries_visited, discovery_truncated) =
        discover(&config, &index, boundary, request.max_entries)?;
    let mut report = compare_sources(
        &states,
        generation.as_deref(),
        request,
        &paths,
        boundary,
        &index,
    );
    report.entries_visited = entries_visited;
    report.discovery_truncated = discovery_truncated;
    report.truncated |= discovery_truncated;
    Ok(report)
}

fn check_boundary(root: &Path, path: &Path) -> StoreResult<()> {
    crate::indexing::facade::IndexFacade::contained_source(root, path)
        .map(|_| ())
        .map_err(|error| DocumentStoreError::Index(error.to_string()))
}

const MAX_IGNORE_BYTES: u64 = 1024 * 1024;
const MAX_DISCOVERY_DEPTH: usize = 64;

fn load_policy(
    directory: &Path,
    bytes_read: &mut u64,
) -> StoreResult<Option<ignore::gitignore::Gitignore>> {
    let path = directory.join(".codannaignore");
    let file = match open_regular_source(&path) {
        Ok(Some(file)) => file,
        Ok(None) => {
            return Err(DocumentStoreError::Index(format!(
                "Ignore policy is not a regular file: {}",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let remaining = MAX_IGNORE_BYTES.saturating_sub(*bytes_read);
    let length = file.metadata()?.len();
    if length > remaining {
        return Err(DocumentStoreError::Index(
            "Document ignore policies exceed the 1 MiB read budget".into(),
        ));
    }
    let mut bytes = Vec::new();
    (&file).take(remaining).read_to_end(&mut bytes)?;
    *bytes_read += bytes.len() as u64;
    if file.metadata()?.len() != length || bytes.len() as u64 != length {
        return Err(DocumentStoreError::Index(
            "Document ignore policy changed during inspection; retry".into(),
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| DocumentStoreError::Index(error.to_string()))?;
    let mut builder = ignore::gitignore::GitignoreBuilder::new(directory);
    for line in text.lines() {
        builder
            .add_line(Some(path.clone()), line)
            .map_err(|error| DocumentStoreError::Index(error.to_string()))?;
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| DocumentStoreError::Index(error.to_string()))
}

/// Count raw directory entries before ignore evaluation, then prune ignored trees.
fn discover(
    config: &CollectionConfig,
    excluded: &Path,
    boundary: Option<&Path>,
    limit: usize,
) -> StoreResult<(Vec<PathBuf>, usize, bool)> {
    let patterns = config
        .effective_patterns()
        .iter()
        .map(|pattern| {
            glob::Pattern::new(pattern).map_err(|error| {
                DocumentStoreError::Index(format!("Invalid document pattern: {error}"))
            })
        })
        .collect::<StoreResult<Vec<_>>>()?;
    let mut files = BTreeSet::new();
    let mut visited = 0;
    let mut truncated = false;
    let mut ignore_bytes = 0;
    'roots: for configured in &config.paths {
        if visited == limit {
            truncated = true;
            break;
        }
        if let Some(root) = boundary {
            check_boundary(root, configured)?;
        }
        let base = normalize_source_path(configured);
        if base.starts_with(excluded) {
            visited += 1;
            continue;
        }
        let metadata = match std::fs::symlink_metadata(&base) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                visited += 1;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.is_file() {
            visited += 1;
            files.insert(base);
            continue;
        }
        if !metadata.is_dir() {
            visited += 1;
            continue;
        }
        let ancestors: Vec<_> = base.ancestors().skip(1).collect();
        if ancestors.len() > MAX_DISCOVERY_DEPTH {
            return Err(DocumentStoreError::Index(
                "Document root exceeds the ancestor depth budget".into(),
            ));
        }
        let mut policies = Vec::new();
        for parent in ancestors.into_iter().rev() {
            if let Some(policy) = load_policy(parent, &mut ignore_bytes)? {
                policies.push((parent.to_path_buf(), policy));
            }
        }
        let mut walker = walkdir::WalkDir::new(&base)
            .follow_links(false)
            .max_depth(MAX_DISCOVERY_DEPTH)
            .max_open(MAX_DISCOVERY_DEPTH)
            .into_iter();
        loop {
            if visited == limit {
                truncated = true;
                break 'roots;
            }
            let Some(entry) = walker.next() else {
                break;
            };
            visited += 1;
            let entry = entry.map_err(|error| {
                DocumentStoreError::Index(format!("Document discovery failed: {error}"))
            })?;
            let path = entry.path();
            let is_dir = entry.file_type().is_dir();
            policies.retain(|(root, _)| path.starts_with(root));
            let ignored = policies
                .iter()
                .rev()
                .map(|(_, policy)| policy.matched(path, is_dir))
                .find(|matched| !matched.is_none())
                .is_some_and(|matched| matched.is_ignore());
            if path.starts_with(excluded) || (entry.depth() > 0 && ignored) {
                if is_dir {
                    walker.skip_current_dir();
                }
                continue;
            }
            if is_dir {
                if entry.depth() == MAX_DISCOVERY_DEPTH {
                    truncated = true;
                    walker.skip_current_dir();
                    continue;
                }
                if let Some(policy) = load_policy(path, &mut ignore_bytes)? {
                    policies.push((path.to_path_buf(), policy));
                }
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let relative = path.strip_prefix(&base).unwrap_or(path);
            if !patterns.iter().any(|pattern| {
                pattern.matches_path(relative)
                    || pattern.as_str().strip_prefix("**/").is_some_and(|pattern| {
                        glob::Pattern::new(pattern)
                            .is_ok_and(|pattern| pattern.matches_path(relative))
                    })
            }) {
                continue;
            }
            let path = normalize_source_path(path);
            if let Some(root) = boundary {
                check_boundary(root, &path)?;
            }
            if !path.starts_with(excluded) {
                files.insert(path);
            }
        }
    }
    Ok((files.into_iter().collect(), visited, truncated))
}

pub(super) fn compare_sources(
    states: &HashMap<PathBuf, FileState>,
    generation: Option<&str>,
    request: &DocumentDriftRequest,
    current_paths: &[PathBuf],
    boundary: Option<&Path>,
    excluded: &Path,
) -> SourceDriftReport {
    let paths: BTreeSet<_> = states
        .iter()
        .filter(|(_, state)| state.collection == request.collection)
        .map(|(path, _)| path.clone())
        .chain(
            current_paths
                .iter()
                .map(|path| normalize_source_path(path))
                .filter(|path| {
                    states
                        .get(path)
                        .is_none_or(|state| state.collection == request.collection)
                }),
        )
        .collect();
    let mut report = SourceDriftReport {
        collection: request.collection.clone(),
        generation: generation.map(str::to_owned),
        candidate_files: paths.len(),
        truncated: paths.len() > request.max_files,
        bytes_read: 0,
        files: Vec::new(),
        max_files: request.max_files,
        max_bytes: request.max_bytes,
        max_entries: request.max_entries,
        entries_visited: 0,
        discovery_truncated: false,
    };
    for path in paths.into_iter().take(request.max_files) {
        let indexed = states
            .get(&path)
            .filter(|state| state.collection == request.collection)
            .map(|state| state.content_hash.clone());
        let mut entry = SourceDriftEntry {
            path: path.clone(),
            status: "unreadable",
            indexed_sha256: indexed,
            current_sha256: None,
        };
        if boundary.is_some_and(|root| check_boundary(root, &path).is_err())
            || normalize_source_path(&path).starts_with(excluded)
        {
            entry.status = "excluded";
            report.files.push(entry);
            continue;
        }
        match open_regular_source(&path) {
            Ok(Some(file)) => {
                let before = file.metadata().ok();
                let remaining = request.max_bytes.saturating_sub(report.bytes_read);
                if let Some(metadata) = before {
                    if metadata.len() > remaining as u64 {
                        entry.status = "byte_budget_exceeded";
                        report.truncated = true;
                    } else {
                        let mut bytes = Vec::new();
                        let read = (&file).take(remaining as u64).read_to_end(&mut bytes);
                        report.bytes_read += bytes.len();
                        let stable = file.metadata().is_ok_and(|after| {
                            after.len() == metadata.len()
                                && after.modified().ok() == metadata.modified().ok()
                        });
                        if read.is_ok() && bytes.len() as u64 == metadata.len() && stable {
                            if let Ok(content) = std::str::from_utf8(&bytes) {
                                let hash = crate::indexing::file_info::calculate_hash(content);
                                entry.status = match entry.indexed_sha256.as_ref() {
                                    None => "new",
                                    Some(old) if old == &hash => "unchanged",
                                    Some(_) => "changed",
                                };
                                entry.current_sha256 = Some(hash);
                            }
                        } else if read.is_ok() {
                            entry.status = "changed_during_read";
                            report.truncated = true;
                        }
                    }
                }
            }
            Ok(None) => entry.status = "unsupported_source",
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => entry.status = "missing",
            Err(_) => {}
        }
        report.files.push(entry);
    }
    report
}

/// Do not follow replaced symlinks or block on FIFOs, including an open-time race.
pub(crate) fn open_regular_source(path: &Path) -> std::io::Result<Option<std::fs::File>> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Ok(None);
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    Ok(file.metadata()?.is_file().then_some(file))
}
