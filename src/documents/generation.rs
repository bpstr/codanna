//! Immutable document generations. Tantivy's commit payload is the sole commit
//! pointer; state.json is a compatibility mirror, never a recovery authority.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::store::{DocumentStoreError, PersistedState, StoreResult};
use crate::vector::{MmapVectorStorage, SegmentOrdinal, VectorDimension, VectorId};

const PAYLOAD_PREFIX: &str = "codanna-documents-v1:";
const MAX_SEGMENTS: usize = 8;
const MAX_GENERATION_BYTES: u64 = 128 * 1024 * 1024;

/// Serializes publication and orphan collection between independent processes.
/// Query snapshots already own their maps and do not take this writer lock.
pub(super) fn lock(base: &Path) -> StoreResult<File> {
    let file = lock_file(base, "generation.lock")?;
    fs4::fs_std::FileExt::lock_exclusive(&file)?;
    Ok(file)
}

fn lock_file(base: &Path, name: &str) -> std::io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(base.join(name))
}

pub(super) fn publication_lock(base: &Path) -> StoreResult<File> {
    let file = lock_file(base, "publication.lock")?;
    fs4::fs_std::FileExt::lock_exclusive(&file)?;
    Ok(file)
}

pub(super) fn try_lock(base: &Path) -> StoreResult<Option<File>> {
    let file = lock_file(base, "generation.lock")?;
    Ok(fs4::fs_std::FileExt::try_lock_exclusive(&file)?.then_some(file))
}

pub(super) fn read_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> StoreResult<T> {
    let file = File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(DocumentStoreError::Index(format!(
            "Document state {} exceeds the {limit}-byte limit",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(DocumentStoreError::Index(
            "Document state grew beyond its size limit".into(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        DocumentStoreError::Index(format!(
            "Invalid document state {}: {error}",
            path.display()
        ))
    })
}

#[derive(Serialize, Deserialize)]
pub(super) struct Generation {
    pub state: PersistedState,
    pub vectors: Vec<String>,
    #[serde(default)]
    pub new_vector_bytes: u64,
    #[serde(default)]
    pub compacted_vector_bytes: u64,
}

fn safe_name(name: &str, prefix: &str) -> bool {
    name.starts_with(prefix)
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

pub(super) fn payload(name: &str) -> String {
    format!("{PAYLOAD_PREFIX}{name}")
}

pub(super) fn load(
    base: &Path,
    payload: Option<&str>,
) -> StoreResult<Option<(String, Generation)>> {
    let Some(payload) = payload else {
        return Ok(None);
    };
    let name = payload.strip_prefix(PAYLOAD_PREFIX).ok_or_else(|| {
        DocumentStoreError::Index("Unrecognized document generation commit payload".into())
    })?;
    if !safe_name(name, "gen-") || !name.ends_with(".json") {
        return Err(DocumentStoreError::Index(
            "Invalid document generation name in commit payload".into(),
        ));
    }
    let generation: Generation =
        read_json(&base.join("generations").join(name), MAX_GENERATION_BYTES)?;
    if generation.vectors.len() > MAX_SEGMENTS {
        return Err(DocumentStoreError::Index(
            "Document generation exceeds the vector segment limit".into(),
        ));
    }
    let mut unique = HashSet::new();
    for name in &generation.vectors {
        segment_path(base, name)?;
        if !unique.insert(name) {
            return Err(DocumentStoreError::Index(
                "Duplicate segment in document generation".into(),
            ));
        }
    }
    Ok(Some((name.to_owned(), generation)))
}

pub(super) fn prepare(base: &Path, generation: &Generation) -> StoreResult<String> {
    let directory = base.join("generations");
    fs::create_dir_all(&directory)?;
    let mut file = tempfile::Builder::new()
        .prefix("gen-")
        .suffix(".json")
        .tempfile_in(&directory)?;
    serde_json::to_writer(&mut file, generation).map_err(|error| {
        DocumentStoreError::Index(format!("Cannot serialize document generation: {error}"))
    })?;
    file.flush()?;
    if file.as_file().metadata()?.len() > MAX_GENERATION_BYTES {
        return Err(DocumentStoreError::Index(
            "Document generation exceeds its 128 MiB state limit".into(),
        ));
    }
    file.as_file().sync_all()?;
    let (_, path) = file.keep().map_err(|error| error.error)?;
    sync_directory(&directory)?;
    sync_directory(base)?;
    Ok(path
        .file_name()
        .expect("generated file name")
        .to_string_lossy()
        .into_owned())
}

pub(super) fn atomic_json(base: &Path, name: &str, value: &impl Serialize) -> StoreResult<()> {
    let mut file = tempfile::Builder::new()
        .prefix(".document-state-")
        .tempfile_in(base)?;
    serde_json::to_writer_pretty(&mut file, value)
        .map_err(|error| DocumentStoreError::Index(format!("Cannot serialize {name}: {error}")))?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(base.join(name)).map_err(|error| error.error)?;
    sync_directory(base)?;
    Ok(())
}

pub(super) fn repair_state_mirror(base: &Path, state: &PersistedState) -> StoreResult<()> {
    let expected = serde_json::to_value(state).map_err(|error| {
        DocumentStoreError::Index(format!("Cannot serialize document state: {error}"))
    })?;
    let current =
        read_json::<serde_json::Value>(&base.join("state.json"), MAX_GENERATION_BYTES).ok();
    if current.as_ref() != Some(&expected) {
        atomic_json(base, "state.json", state)?;
    }
    Ok(())
}

pub(super) fn sync_directory(path: &Path) -> std::io::Result<()> {
    // Directory fsync establishes rename durability on Unix. Windows does not
    // support opening directories with File::open; file flushes and atomic
    // replacements still protect process-termination recovery there.
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn segment_path(base: &Path, name: &str) -> StoreResult<PathBuf> {
    if name == "legacy" {
        return Ok(base.join("vectors"));
    }
    if !safe_name(name, "segment-") {
        return Err(DocumentStoreError::Index(
            "Invalid document vector segment name".into(),
        ));
    }
    Ok(base.join("vectors").join(name))
}

/// Every mapped segment is immutable and shared with active query snapshots.
#[derive(Clone, Default)]
pub(super) struct SegmentedVectors {
    names: Vec<String>,
    segments: Vec<Arc<Mutex<MmapVectorStorage>>>,
    count: usize,
}

impl SegmentedVectors {
    pub fn open(base: &Path, names: &[String]) -> StoreResult<Self> {
        let mut vectors = Self::default();
        for name in names {
            let storage =
                MmapVectorStorage::open(segment_path(base, name)?, SegmentOrdinal::new(0))?;
            if let Some(dimension) = vectors.dimension()? {
                if dimension != storage.dimension() {
                    return Err(DocumentStoreError::Index(
                        "Document generation contains mixed vector dimensions".into(),
                    ));
                }
            }
            vectors.push(name.clone(), storage);
        }
        Ok(vectors)
    }

    fn push(&mut self, name: String, storage: MmapVectorStorage) {
        self.count += storage.vector_count();
        self.names.push(name);
        self.segments.push(Arc::new(Mutex::new(storage)));
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn vector_count(&self) -> usize {
        self.count
    }

    pub fn dimension(&self) -> StoreResult<Option<VectorDimension>> {
        self.segments
            .first()
            .map(|segment| {
                segment
                    .lock()
                    .map(|storage| storage.dimension())
                    .map_err(|_| DocumentStoreError::LockPoisoned)
            })
            .transpose()
    }

    pub fn missing_ids(&self, ids: &HashSet<VectorId>) -> StoreResult<HashSet<VectorId>> {
        let mut remaining = ids.clone();
        for segment in &self.segments {
            segment
                .lock()
                .map_err(|_| DocumentStoreError::LockPoisoned)?
                .retain_missing_ids(&mut remaining)?;
        }
        Ok(remaining)
    }

    pub fn score_vectors(
        &self,
        ids: &HashSet<VectorId>,
        query: &[f32],
    ) -> StoreResult<Vec<(VectorId, f32)>> {
        let mut remaining = ids.clone();
        let mut scores = Vec::with_capacity(ids.len());
        for segment in &self.segments {
            let selected = segment
                .lock()
                .map_err(|_| DocumentStoreError::LockPoisoned)?
                .score_vectors(&remaining, query)?;
            for (id, score) in selected {
                remaining.remove(&id);
                scores.push((id, score));
            }
        }
        Ok(scores)
    }

    pub fn append(&mut self, staged: StagedVectors) -> StoreResult<u64> {
        let (name, storage) = staged.finish()?;
        let bytes = (storage.vector_count() * (4 + storage.dimension().get() * 4)) as u64;
        self.push(name, storage);
        Ok(bytes)
    }

    pub fn compact(&mut self, base: &Path, live: &HashSet<VectorId>) -> StoreResult<u64> {
        if self.count <= live.len().saturating_mul(2) && self.segments.len() <= MAX_SEGMENTS {
            return Ok(0);
        }
        if live.is_empty() {
            *self = Self::default();
            return Ok(0);
        }
        let dimension = self.dimension()?.ok_or_else(|| {
            DocumentStoreError::Index("Cannot compact missing document vectors".into())
        })?;
        let mut staging = StagedVectors::new(base, dimension)?;
        let mut remaining = live.clone();
        let mut copied = 0;
        for segment in &self.segments {
            copied += segment
                .lock()
                .map_err(|_| DocumentStoreError::LockPoisoned)?
                .copy_selected_to(&mut remaining, &mut staging.storage)?;
        }
        let mut compacted = Self::default();
        if copied > 0 {
            compacted.append(staging)?;
        }
        *self = compacted;
        Ok((copied * (4 + dimension.get() * 4)) as u64)
    }
}

pub(super) struct StagedVectors {
    directory: tempfile::TempDir,
    pub storage: MmapVectorStorage,
}

impl StagedVectors {
    pub fn new(base: &Path, dimension: VectorDimension) -> StoreResult<Self> {
        let vectors = base.join("vectors");
        fs::create_dir_all(&vectors)?;
        let directory = tempfile::Builder::new()
            .prefix("segment-")
            .tempdir_in(vectors)?;
        let storage =
            MmapVectorStorage::open_or_create(directory.path(), SegmentOrdinal::new(0), dimension)?;
        Ok(Self { directory, storage })
    }

    fn finish(self) -> StoreResult<(String, MmapVectorStorage)> {
        let file = self.directory.path().join("segment_0.vec");
        File::open(file)?.sync_all()?;
        sync_directory(self.directory.path())?;
        // Open the complete immutable mmap before making the directory permanent.
        let storage = MmapVectorStorage::open(self.directory.path(), SegmentOrdinal::new(0))?;
        let directory = self.directory.keep();
        sync_directory(directory.parent().expect("vector directory"))?;
        Ok((
            directory
                .file_name()
                .expect("generated segment name")
                .to_string_lossy()
                .into_owned(),
            storage,
        ))
    }
}

/// Unlink obsolete generations only after publication (or after recovery while
/// holding the document writer lock). Existing mmaps keep their inode alive;
/// platforms that disallow deleting mapped files retry on a later collection.
pub(super) fn collect_obsolete(base: &Path, active: Option<&str>, vectors: &[String]) {
    for (directory, prefix, retained) in [
        (
            base.join("generations"),
            "gen-",
            active.into_iter().collect::<HashSet<_>>(),
        ),
        (
            base.join("vectors"),
            "segment-",
            vectors.iter().map(String::as_str).collect(),
        ),
        (base.to_path_buf(), ".document-spool-", HashSet::new()),
        (base.to_path_buf(), ".document-state-", HashSet::new()),
    ] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !safe_name(&name, prefix) || retained.contains(name.as_ref()) {
                continue;
            }
            let result = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                fs::remove_dir_all(entry.path())
            } else {
                fs::remove_file(entry.path())
            };
            if let Err(error) = result {
                tracing::debug!(target: "documents", %error, "document generation cleanup deferred");
            }
        }
    }
    if !vectors.iter().any(|name| name == "legacy") {
        let _ = fs::remove_file(base.join("vectors/segment_0.vec"));
    }
}

/// These hooks are compiled only into the unit-test executable. A child process
/// exits without running any destructor, exercising real reopen recovery.
pub(super) fn publication_boundary(_name: &str) {
    #[cfg(test)]
    if std::env::var("CODANNA_TEST_DOCUMENT_CRASH").as_deref() == Ok(_name) {
        std::process::exit(86);
    }
}
