//! Immutable semantic checkpoints and ID deltas, committed by one atomic manifest.
//! Readers and compaction hold a filesystem lock so an old pinned checkpoint is
//! never removed while a reader is loading it. Interrupted, unreferenced writes
//! are ignored; a malformed referenced artifact is an error, not an empty index.

use super::{SemanticMetadata, SemanticSearchError, SemanticVectorStorage, SimpleSemanticSearch};
use crate::{SymbolId, vector::VectorDimension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

const MAX_DELTAS: usize = 64;
const MAX_DELTA_IDS: usize = 4096;
const MAX_MANIFEST: u64 = 128 * 1024;
const MAX_DELTA_BYTES: u64 = 256 * 1024 * 1024;

pub(super) fn error(message: impl std::fmt::Display) -> SemanticSearchError {
    SemanticSearchError::StorageError {
        message: message.to_string(),
        suggestion:
            "Preserve the semantic directory and retry/reload; do not overwrite corrupt state."
                .into(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Persistence {
    pub dirty: HashSet<SymbolId>,
    pub saved: Option<(PathBuf, Vec<u8>)>,
    pub revision: u64,
    /// Most recent snapshot durably published by this in-memory owner. An older
    /// prepared save must never overwrite a newer completed save.
    pub committed_revision: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    segments_sha256: Option<String>,
    deltas: Vec<Artifact>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Manifest {
    #[serde(flatten)]
    metadata: SemanticMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    journal: Option<Journal>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Delta {
    entries: Vec<DeltaEntry>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    symbol_segments: HashMap<u32, Vec<super::simple::SymbolSegment>>,
}
type DeltaEntry = (u32, Option<(Vec<f32>, Option<String>)>);

pub(super) struct Snapshot {
    pub embeddings: HashMap<SymbolId, Vec<f32>>,
    pub languages: HashMap<SymbolId, String>,
    pub symbol_segments: super::simple::SymbolSegments,
    pub metadata: SemanticMetadata,
    pub persistence: Persistence,
}

fn lock(path: &Path, write: bool) -> Result<File, SemanticSearchError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.join(".semantic.lock"))
        .map_err(error)?;
    if write {
        fs4::fs_std::FileExt::lock_exclusive(&file)
    } else {
        fs4::fs_std::FileExt::lock_shared(&file)
    }
    .map_err(error)?;
    Ok(file) // dropping the file releases the lock, without unlinking its inode
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, SemanticSearchError> {
    let file = File::open(path).map_err(error)?;
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > max {
        return Err(error("semantic artifact exceeds format budget"));
    }
    Ok(bytes)
}

fn valid_name(name: &str) -> bool {
    name.starts_with("gen-")
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
}

fn manifest(bytes: &[u8]) -> Result<Manifest, SemanticSearchError> {
    let value: Manifest = serde_json::from_slice(bytes).map_err(error)?;
    if !(1..=4).contains(&value.metadata.version) || !(1..=4096).contains(&value.metadata.dimension)
    {
        return Err(error("unsupported semantic metadata version or dimension"));
    }
    if let Some(j) = &value.journal {
        if !(2..=4).contains(&value.metadata.version)
            || (value.metadata.version == 4
                && j.segments_sha256
                    .as_ref()
                    .is_none_or(|digest| digest.len() != 64))
            || j.version != 1
            || !valid_name(&j.base)
            || j.deltas.len() > MAX_DELTAS
            || j.deltas
                .iter()
                .any(|d| !valid_name(&d.name) || d.sha256.len() != 64)
        {
            return Err(error("invalid semantic journal manifest"));
        }
    }
    if value.metadata.version == 4 && value.journal.is_none() {
        return Err(error(
            "symbol segment metadata requires a committed journal",
        ));
    }
    Ok(value)
}

fn sync_dir(path: &Path) -> Result<(), SemanticSearchError> {
    #[cfg(unix)]
    File::open(path).and_then(|f| f.sync_all()).map_err(error)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn atomic_manifest(path: &Path, bytes: &[u8]) -> Result<(), SemanticSearchError> {
    let mut tmp = tempfile::NamedTempFile::new_in(path).map_err(error)?;
    tmp.write_all(bytes).map_err(error)?;
    tmp.as_file().sync_all().map_err(error)?;
    tmp.persist(path.join("metadata.json")).map_err(error)?;
    sync_dir(path)
}

fn validate_segments(
    mut parts: Vec<super::simple::SymbolSegment>,
    dimension: usize,
) -> Result<Vec<super::simple::SymbolSegment>, SemanticSearchError> {
    if parts.is_empty() || parts.len() > crate::symbol_representation::MAX_SYMBOL_SEGMENTS {
        return Err(error("invalid symbol segment group size"));
    }
    for part in &mut parts {
        part.validate(dimension)?;
    }
    Ok(parts)
}

struct BoundedHashWriter<W: Write> {
    writer: W,
    digest: Sha256,
    written: u64,
}
impl<W: Write> Write for BoundedHashWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > MAX_DELTA_BYTES.saturating_sub(self.written) {
            return Err(std::io::Error::other(
                "symbol segment checkpoint exceeds format budget",
            ));
        }
        let count = self.writer.write(bytes)?;
        self.digest.update(&bytes[..count]);
        self.written += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

pub(super) fn save(
    path: &Path,
    state: &mut Persistence,
    embeddings: &HashMap<SymbolId, std::sync::Arc<[f32]>>,
    symbol_segments: &super::simple::SymbolSegments,
    languages: &HashMap<SymbolId, String>,
    mut metadata: SemanticMetadata,
) -> Result<(), SemanticSearchError> {
    fs::create_dir_all(path).map_err(error)?;
    let canonical = path.canonicalize().map_err(error)?;
    let same_path = state.saved.as_ref().is_some_and(|(p, _)| p == &canonical);
    if same_path && state.dirty.is_empty() {
        return Ok(());
    }
    let _lock = lock(path, true)?;
    let disk = match read_bounded(&path.join("metadata.json"), MAX_MANIFEST) {
        Ok(bytes) => Some(bytes),
        Err(e) if !path.join("metadata.json").try_exists().map_err(error)? => {
            let _ = e;
            None
        }
        Err(e) => return Err(e),
    };
    let current = disk.as_deref().map(manifest).transpose()?;
    // Loaded/written instances must not overwrite changes from another writer.
    // A new instance deliberately publishes a complete replacement checkpoint.
    if same_path && disk.as_ref() != state.saved.as_ref().map(|(_, b)| b) {
        return Err(error(
            "semantic generation changed: reload before saving deltas",
        ));
    }
    // Version 4 is required even if the current body-policy corpus has no symbols.
    let uses_segments = !symbol_segments.is_empty()
        || metadata
            .embedding_identity
            .as_deref()
            .is_some_and(|identity| {
                serde_json::from_str::<serde_json::Value>(identity)
                    .ok()
                    .is_some_and(|value| value.get("source_input_policy").is_some())
            });
    let incremental = same_path
        && current
            .as_ref()
            .is_some_and(|m| (m.metadata.version == 4) == uses_segments)
        && current
            .as_ref()
            .and_then(|m| m.journal.as_ref())
            .is_some_and(|j| j.deltas.len() < MAX_DELTAS)
        && state.dirty.len() <= MAX_DELTA_IDS;
    let journal = if incremental {
        let mut journal = current
            .as_ref()
            .and_then(|m| m.journal.clone())
            .expect("checked journal");
        let mut entries = Vec::with_capacity(state.dirty.len());
        for id in &state.dirty {
            entries.push((
                id.value(),
                embeddings
                    .get(id)
                    .map(|v| (v.to_vec(), languages.get(id).cloned())),
            ));
        }
        entries.sort_by_key(|(id, _)| *id);
        let symbol_segments = state
            .dirty
            .iter()
            .filter_map(|id| {
                symbol_segments
                    .get(id)
                    .map(|parts| (id.value(), parts.to_vec()))
            })
            .collect();
        let bytes = serde_json::to_vec(&Delta {
            entries,
            symbol_segments,
        })
        .map_err(error)?;
        if bytes.len() as u64 > MAX_DELTA_BYTES {
            return Err(error("semantic delta exceeds format budget"));
        }
        let mut tmp = tempfile::Builder::new()
            .prefix("gen-")
            .suffix(".delta")
            .tempfile_in(path)
            .map_err(error)?;
        tmp.write_all(&bytes).map_err(error)?;
        tmp.as_file().sync_all().map_err(error)?;
        let name = tmp
            .path()
            .file_name()
            .expect("temp name")
            .to_string_lossy()
            .into_owned();
        let (_, kept) = tmp.keep().map_err(error)?;
        debug_assert_eq!(kept.file_name().unwrap(), name.as_str());
        journal.deltas.push(Artifact {
            name,
            sha256: sha256(&bytes),
        });
        journal
    } else {
        let dir = tempfile::Builder::new()
            .prefix("gen-")
            .tempdir_in(path)
            .map_err(error)?;
        let dimension = VectorDimension::new(metadata.dimension).map_err(error)?;
        let mut storage = SemanticVectorStorage::new(dir.path(), dimension)?;
        storage.save_batch_borrowed(embeddings.iter().map(|(id, v)| (*id, v.as_ref())))?;
        drop(storage);
        OpenOptions::new()
            .write(true)
            .open(dir.path().join("segment_0.vec"))
            .and_then(|f| f.sync_all())
            .map_err(error)?;
        let mut lang_file = File::create(dir.path().join("languages.json")).map_err(error)?;
        let borrowed: HashMap<u32, &str> = languages
            .iter()
            .map(|(id, l)| (id.value(), l.as_str()))
            .collect();
        serde_json::to_writer(&mut lang_file, &borrowed).map_err(error)?;
        lang_file.sync_all().map_err(error)?;
        let segments_sha256 = if uses_segments {
            let borrowed: std::collections::BTreeMap<u32, &[super::simple::SymbolSegment]> =
                symbol_segments
                    .iter()
                    .map(|(id, parts)| (id.value(), parts.as_ref()))
                    .collect();
            let mut file = File::create(dir.path().join("symbol-segments.json")).map_err(error)?;
            let digest = {
                let mut writer = BoundedHashWriter {
                    writer: &mut file,
                    digest: Sha256::new(),
                    written: 0,
                };
                serde_json::to_writer(&mut writer, &borrowed).map_err(error)?;
                format!("{:x}", writer.digest.finalize())
            };
            file.sync_all().map_err(error)?;
            Some(digest)
        } else {
            None
        };
        sync_dir(dir.path())?;
        let base = dir
            .path()
            .file_name()
            .expect("temp name")
            .to_string_lossy()
            .into_owned();
        let _ = dir.keep();
        Journal {
            version: 1,
            base,
            segments_sha256,
            deltas: Vec::new(),
        }
    };
    sync_dir(path)?;
    metadata.version = if uses_segments { 4 } else { 3 };
    let next = Manifest {
        metadata,
        journal: Some(journal),
    };
    let bytes = serde_json::to_vec_pretty(&next).map_err(error)?;
    atomic_manifest(path, &bytes)?;
    state.saved = Some((canonical, bytes));
    state.dirty.clear();
    // Only our versioned files can be collected, only under the exclusive lock,
    // and only after the new manifest is durably published. Never touch v1 data.
    let j = next.journal.as_ref().expect("new journal");
    if j.deltas.is_empty() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if valid_name(&name) && name != j.base {
                    let _ = if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        fs::remove_dir_all(entry.path())
                    } else {
                        fs::remove_file(entry.path())
                    };
                }
            }
        }
    }
    Ok(())
}

pub(super) fn load(path: &Path) -> Result<Snapshot, SemanticSearchError> {
    let _lock = lock(path, false)?;
    let bytes = read_bounded(&path.join("metadata.json"), MAX_MANIFEST)?;
    let current = manifest(&bytes)?;
    let base = current
        .journal
        .as_ref()
        .map(|j| path.join(&j.base))
        .unwrap_or_else(|| path.to_path_buf());
    let mut storage = SemanticVectorStorage::open(&base)?;
    if storage.dimension().get() != current.metadata.dimension {
        return Err(SemanticSearchError::DimensionMismatch {
            expected: current.metadata.dimension,
            actual: storage.dimension().get(),
            suggestion: "Preserve the directory and rebuild explicitly with the correct model"
                .into(),
        });
    }
    let mut embeddings: HashMap<_, _> = storage.load_all()?.into_iter().collect();
    drop(storage);
    // A v2 checkpoint always publishes language metadata with its vectors.
    // Missing legacy metadata remains compatible, but a referenced v2 artifact
    // must never silently degrade to an empty language map.
    if current.journal.is_some() {
        fs::metadata(base.join("languages.json")).map_err(error)?;
    }
    let mut languages = SimpleSemanticSearch::load_symbol_languages(&base)?;
    let mut symbol_segments = super::simple::SymbolSegments::new();
    if current.metadata.version == 4 {
        let bytes = read_bounded(&base.join("symbol-segments.json"), MAX_DELTA_BYTES)?;
        let expected = current
            .journal
            .as_ref()
            .and_then(|journal| journal.segments_sha256.as_deref());
        if expected != Some(sha256(&bytes).as_str()) {
            return Err(error("symbol segment checkpoint checksum mismatch"));
        }
        let records: HashMap<u32, Vec<super::simple::SymbolSegment>> =
            serde_json::from_slice(&bytes).map_err(error)?;
        for (raw, parts) in records {
            let id = SymbolId::new(raw).ok_or_else(|| error("zero symbol segment parent ID"))?;
            let parts = validate_segments(parts, current.metadata.dimension)?;
            symbol_segments.insert(id, std::sync::Arc::from(parts));
        }
    }
    if let Some(j) = &current.journal {
        for artifact in &j.deltas {
            let data = read_bounded(&path.join(&artifact.name), MAX_DELTA_BYTES)?;
            if sha256(&data) != artifact.sha256 {
                return Err(error("semantic delta checksum mismatch"));
            }
            let delta: Delta = serde_json::from_slice(&data).map_err(error)?;
            if delta.entries.len() > MAX_DELTA_IDS {
                return Err(error("semantic delta contains too many IDs"));
            }
            let mut seen = HashSet::new();
            for (raw, value) in delta.entries {
                let id = SymbolId::new(raw).ok_or_else(|| error("zero semantic ID"))?;
                if !seen.insert(id) {
                    return Err(error("duplicate ID in semantic delta"));
                }
                symbol_segments.remove(&id);
                match value {
                    Some((v, language)) => {
                        if v.len() != current.metadata.dimension || v.iter().any(|x| !x.is_finite())
                        {
                            return Err(error("invalid semantic vector in delta"));
                        }
                        embeddings.insert(id, v);
                        if let Some(l) = language {
                            languages.insert(id, l);
                        } else {
                            languages.remove(&id);
                        }
                    }
                    None => {
                        embeddings.remove(&id);
                        languages.remove(&id);
                    }
                }
            }
            if current.metadata.version != 4 && !delta.symbol_segments.is_empty() {
                return Err(error("symbol segments require format version 4"));
            }
            for (raw, parts) in delta.symbol_segments {
                let id =
                    SymbolId::new(raw).ok_or_else(|| error("zero symbol segment parent ID"))?;
                if !seen.contains(&id) || !embeddings.contains_key(&id) {
                    return Err(error("symbol segment delta has no changed live parent"));
                }
                let parts = validate_segments(parts, current.metadata.dimension)?;
                symbol_segments.insert(id, std::sync::Arc::from(parts));
            }
        }
    }
    for (id, parts) in &symbol_segments {
        if embeddings.get(id).map(Vec::as_slice) != Some(parts[0].vector.as_ref()) {
            return Err(error(
                "symbol segment group does not match its primary parent vector",
            ));
        }
    }
    let extra_vectors: usize = symbol_segments
        .values()
        .map(|parts| parts.len().saturating_sub(1))
        .sum();
    if extra_vectors != current.metadata.segment_embedding_count {
        return Err(error(
            "symbol segment count does not match committed manifest",
        ));
    }
    if embeddings.len() != current.metadata.embedding_count {
        return Err(error("semantic count does not match committed manifest"));
    }
    Ok(Snapshot {
        embeddings,
        languages,
        symbol_segments,
        metadata: current.metadata,
        persistence: Persistence {
            dirty: HashSet::new(),
            saved: Some((path.canonicalize().map_err(error)?, bytes)),
            revision: 0,
            committed_revision: Some(0),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seed(path: &Path) -> SimpleSemanticSearch {
        let mut search = SimpleSemanticSearch::new_empty(8, "fixture");
        search.store_embeddings(
            (1..=1000)
                .map(|id| {
                    (
                        SymbolId::new(id).unwrap(),
                        vec![id as f32; 8],
                        "rust".into(),
                    )
                })
                .collect(),
        );
        search.save(path).unwrap();
        search
    }
    fn read(path: &Path) -> Manifest {
        manifest(&fs::read(path.join("metadata.json")).unwrap()).unwrap()
    }
    #[test]
    fn hardening_final_semantic_delta_and_tombstone_do_not_rewrite_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let mut search = seed(dir.path());
        let initial = fs::read(dir.path().join("metadata.json")).unwrap();
        let base = read(dir.path()).journal.unwrap().base;
        search.save(dir.path()).unwrap();
        assert_eq!(initial, fs::read(dir.path().join("metadata.json")).unwrap());
        search.remove_embeddings(&[SymbolId::new(1).unwrap()]);
        search.store_embeddings(vec![(
            SymbolId::new(2).unwrap(),
            vec![0.25; 8],
            "typescript".into(),
        )]);
        search.save(dir.path()).unwrap();
        let j = read(dir.path()).journal.unwrap();
        assert_eq!(j.base, base);
        assert_eq!(j.deltas.len(), 1);
        assert!(
            fs::metadata(dir.path().join(&j.deltas[0].name))
                .unwrap()
                .len()
                < 1024
        );
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.embeddings.len(), 999);
        assert!(!loaded.embeddings.contains_key(&SymbolId::new(1).unwrap()));
        assert_eq!(loaded.embeddings[&SymbolId::new(2).unwrap()], vec![0.25; 8]);
        assert_eq!(loaded.languages[&SymbolId::new(2).unwrap()], "typescript");
    }
    #[test]
    fn hardening_final_semantic_compaction_and_unpublished_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let mut search = seed(dir.path());
        let base = read(dir.path()).journal.unwrap().base;
        for step in 0..=MAX_DELTAS {
            search.store_embeddings(vec![(
                SymbolId::new(1).unwrap(),
                vec![step as f32; 8],
                "rust".into(),
            )]);
            search.save(dir.path()).unwrap();
        }
        let j = read(dir.path()).journal.unwrap();
        assert_ne!(j.base, base);
        assert!(j.deltas.is_empty());
        assert!(!dir.path().join(base).exists());
        fs::write(
            dir.path().join("gen-interrupted.delta"),
            b"truncated unpublished frame",
        )
        .unwrap();
        assert_eq!(
            load(dir.path()).unwrap().embeddings[&SymbolId::new(1).unwrap()],
            vec![MAX_DELTAS as f32; 8]
        );
    }
    #[test]
    fn hardening_final_semantic_corruption_and_conflicting_writer_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = seed(dir.path());
        let mut stale = SimpleSemanticSearch::load_remote(dir.path()).unwrap();
        first.remove_embeddings(&[SymbolId::new(1).unwrap()]);
        first.save(dir.path()).unwrap();
        stale.remove_embeddings(&[SymbolId::new(2).unwrap()]);
        assert!(stale.save(dir.path()).is_err());
        let state = fs::read(dir.path().join("metadata.json")).unwrap();
        let delta = read(dir.path()).journal.unwrap().deltas.remove(0);
        fs::write(dir.path().join(delta.name), b"{}").unwrap();
        assert!(load(dir.path()).is_err());
        assert_eq!(fs::read(dir.path().join("metadata.json")).unwrap(), state);
    }
    #[test]
    fn hardening_final_metadata_only_save_cannot_drop_journal_references() {
        let dir = tempfile::tempdir().unwrap();
        let search = seed(dir.path());
        let committed = fs::read(dir.path().join("metadata.json")).unwrap();
        assert!(search.metadata().unwrap().save(dir.path()).is_err());
        assert_eq!(
            committed,
            fs::read(dir.path().join("metadata.json")).unwrap()
        );
        assert_eq!(load(dir.path()).unwrap().embeddings.len(), 1000);
    }
    #[test]
    fn hardening_final_missing_checkpoint_languages_preserves_committed_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let _search = seed(dir.path());
        let committed = fs::read(dir.path().join("metadata.json")).unwrap();
        let base = read(dir.path()).journal.unwrap().base;
        fs::remove_file(dir.path().join(base).join("languages.json")).unwrap();
        assert!(load(dir.path()).is_err());
        assert_eq!(
            fs::read(dir.path().join("metadata.json")).unwrap(),
            committed
        );
    }
}
