//! Bounded, content-addressed embedding reuse shared by code and document indexing.
//!
//! This is an optional accelerator, never a source of truth. A missing, stale, or
//! malformed cache is treated as empty so it cannot prevent an index rebuild.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;

const FORMAT_VERSION: u32 = 1;
const PREPROCESSING_VERSION: u32 = 2;
const DEFAULT_MAX_ENTRIES: usize = 4096;
const MAX_CACHE_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_VECTOR_DIMENSION: usize = 16_384;
const MAX_VECTOR_BYTES: usize = 16 * 1024 * 1024;
const ENTRY_OVERHEAD_BYTES: usize = 256;

#[derive(Clone, Debug)]
pub(crate) struct EmbeddingCache {
    model_identity: String,
    dimension: usize,
    entries: HashMap<String, Arc<[f32]>>,
    insertion_order: VecDeque<String>,
    max_entries: usize,
}

#[derive(Serialize, Deserialize)]
struct PersistedCache {
    format_version: u32,
    preprocessing_version: u32,
    model_identity: String,
    dimension: usize,
    #[serde(deserialize_with = "bounded_entries")]
    entries: Vec<PersistedEntry>,
}

#[derive(Serialize, Deserialize)]
struct PersistedEntry {
    #[serde(deserialize_with = "bounded_hash")]
    input_sha256: String,
    #[serde(deserialize_with = "bounded_embedding")]
    embedding: Vec<f32>,
}

fn bounded_hash<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
    if !is_sha256_hex(&value) {
        return Err(serde::de::Error::custom(
            "invalid embedding cache input hash",
        ));
    }
    Ok(value.into_owned())
}

fn bounded_embedding<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<f32>, D::Error> {
    struct BoundedVector;
    impl<'de> serde::de::Visitor<'de> for BoundedVector {
        type Value = Vec<f32>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an embedding within the vector dimension limit")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = sequence.next_element()? {
                if values.len() >= MAX_VECTOR_DIMENSION {
                    return Err(serde::de::Error::custom(
                        "embedding cache vector exceeds dimension limit",
                    ));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(BoundedVector)
}

fn bounded_entries<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<PersistedEntry>, D::Error> {
    struct BoundedEntries;
    impl<'de> serde::de::Visitor<'de> for BoundedEntries {
        type Value = Vec<PersistedEntry>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("bounded embedding cache entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            let mut memory_bytes = 0usize;
            while let Some(entry) = sequence.next_element::<PersistedEntry>()? {
                memory_bytes +=
                    entry.embedding.len() * std::mem::size_of::<f32>() + ENTRY_OVERHEAD_BYTES;
                if entries.len() >= DEFAULT_MAX_ENTRIES || memory_bytes > MAX_VECTOR_BYTES {
                    return Err(serde::de::Error::custom(
                        "embedding cache exceeds entry or memory limit",
                    ));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_seq(BoundedEntries)
}

impl EmbeddingCache {
    pub(crate) fn empty(model_identity: impl Into<String>, dimension: usize) -> Self {
        Self::with_capacity(model_identity, dimension, DEFAULT_MAX_ENTRIES)
    }

    fn with_capacity(
        model_identity: impl Into<String>,
        dimension: usize,
        max_entries: usize,
    ) -> Self {
        let entry_bytes = dimension
            .saturating_mul(std::mem::size_of::<f32>())
            .saturating_add(ENTRY_OVERHEAD_BYTES);
        let max_entries = if dimension == 0 || dimension > MAX_VECTOR_DIMENSION {
            0
        } else {
            max_entries
                .min(DEFAULT_MAX_ENTRIES)
                .min(MAX_VECTOR_BYTES / entry_bytes)
        };
        Self {
            model_identity: model_identity.into(),
            dimension,
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            max_entries,
        }
    }

    pub(crate) fn load(path: &Path, model_identity: &str, dimension: usize) -> Self {
        let mut cache = Self::empty(model_identity, dimension);
        if cache.max_entries == 0 {
            return cache;
        }
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return cache,
            Err(error) => {
                tracing::warn!(target: "embedding_cache", %error, path = %path.display(), "ignoring unreadable embedding cache");
                return cache;
            }
        };
        // Check the already-open handle before allocation, then bound the read as
        // well so a concurrently growing cache cannot bypass the size check.
        match file.metadata() {
            Ok(metadata) if metadata.len() <= MAX_CACHE_FILE_BYTES => {}
            _ => {
                tracing::warn!(target: "embedding_cache", path = %path.display(), "ignoring oversized or unreadable embedding cache");
                return cache;
            }
        }
        let mut bytes = Vec::new();
        let bytes = match file.take(MAX_CACHE_FILE_BYTES + 1).read_to_end(&mut bytes) {
            Ok(_) if bytes.len() as u64 <= MAX_CACHE_FILE_BYTES => bytes,
            Ok(_) => return cache,
            Err(error) => {
                tracing::warn!(target: "embedding_cache", %error, path = %path.display(), "ignoring unreadable embedding cache");
                return cache;
            }
        };
        let persisted: PersistedCache = match serde_json::from_slice(&bytes) {
            Ok(persisted) => persisted,
            Err(error) => {
                tracing::warn!(target: "embedding_cache", %error, path = %path.display(), "ignoring malformed embedding cache");
                return cache;
            }
        };
        if persisted.format_version != FORMAT_VERSION
            || persisted.preprocessing_version != PREPROCESSING_VERSION
            || persisted.model_identity != model_identity
            || persisted.dimension != dimension
        {
            tracing::debug!(target: "embedding_cache", path = %path.display(), "ignoring incompatible embedding cache");
            return cache;
        }
        for entry in persisted.entries {
            if entry.embedding.len() == dimension
                && entry.embedding.iter().all(|value| value.is_finite())
                && is_sha256_hex(&entry.input_sha256)
            {
                cache.insert_hash(entry.input_sha256, Arc::from(entry.embedding));
            }
        }
        cache
    }

    pub(crate) fn get(&self, input: &str) -> Option<Arc<[f32]>> {
        self.entries.get(&input_hash(input)).cloned()
    }

    pub(crate) fn insert(&mut self, input: &str, embedding: Arc<[f32]>) -> bool {
        if self.max_entries == 0
            || embedding.len() != self.dimension
            || !embedding.iter().all(|value| value.is_finite())
        {
            return false;
        }
        self.insert_hash(input_hash(input), embedding);
        true
    }

    fn insert_hash(&mut self, hash: String, embedding: Arc<[f32]>) {
        if self.entries.insert(hash.clone(), embedding).is_none() {
            self.insertion_order.push_back(hash);
        }
        while self.entries.len() > self.max_entries {
            if let Some(oldest) = self.insertion_order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "embedding cache has no parent")
        })?;
        std::fs::create_dir_all(parent)?;
        let persisted = PersistedCache {
            format_version: FORMAT_VERSION,
            preprocessing_version: PREPROCESSING_VERSION,
            model_identity: self.model_identity.clone(),
            dimension: self.dimension,
            entries: self
                .insertion_order
                .iter()
                .filter_map(|hash| {
                    self.entries.get(hash).map(|embedding| PersistedEntry {
                        input_sha256: hash.clone(),
                        embedding: embedding.to_vec(),
                    })
                })
                .collect(),
        };
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(
            &mut LimitedWriter {
                inner: &mut temp,
                remaining: MAX_CACHE_FILE_BYTES,
            },
            &persisted,
        )
        .map_err(io::Error::other)?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

struct LimitedWriter<W> {
    inner: W,
    remaining: u64,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(io::Error::other(
                "embedding cache exceeds its file size limit",
            ));
        }
        let count = self.inner.write(bytes)?;
        self.remaining -= count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn input_hash(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_exact_content_and_invalidates_scope_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = EmbeddingCache::empty("model@revision", 2);
        assert!(cache.insert("exact input", Arc::from(vec![1.0, 2.0])));
        cache.save(&path).unwrap();

        assert_eq!(
            EmbeddingCache::load(&path, "model@revision", 2)
                .get("exact input")
                .unwrap()
                .as_ref(),
            &[1.0, 2.0]
        );
        assert!(
            EmbeddingCache::load(&path, "other-model", 2)
                .get("exact input")
                .is_none()
        );
        assert!(
            EmbeddingCache::load(&path, "model@revision", 3)
                .get("exact input")
                .is_none()
        );
    }

    #[test]
    fn bounds_entries_and_ignores_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = EmbeddingCache::with_capacity("model", 1, 2);
        cache.insert("one", Arc::from(vec![1.0]));
        cache.insert("two", Arc::from(vec![2.0]));
        cache.insert("three", Arc::from(vec![3.0]));
        assert!(cache.get("one").is_none());
        assert!(cache.get("two").is_some());
        assert!(cache.get("three").is_some());

        std::fs::write(&path, b"not json").unwrap();
        assert!(EmbeddingCache::load(&path, "model", 1).get("two").is_none());
    }

    #[test]
    fn oversized_cache_is_ignored_before_reading_or_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oversized-cache.json");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"{\"entries\":[").unwrap();
        // A sparse malformed file exercises the metadata guard without allocating
        // or reading its apparent length into the test process.
        file.set_len(MAX_CACHE_FILE_BYTES + 1).unwrap();
        let cache = EmbeddingCache::load(&path, "model", 2);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn bounded_decoder_rejects_large_vectors_and_excess_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let entry = |dimension| PersistedEntry {
            input_sha256: input_hash("oversized"),
            embedding: vec![1.0; dimension],
        };
        for entries in [
            vec![entry(MAX_VECTOR_DIMENSION + 1)],
            (0..=DEFAULT_MAX_ENTRIES).map(|_| entry(1)).collect(),
        ] {
            let persisted = PersistedCache {
                format_version: FORMAT_VERSION,
                preprocessing_version: PREPROCESSING_VERSION,
                model_identity: "model".into(),
                dimension: 1,
                entries,
            };
            std::fs::write(&path, serde_json::to_vec(&persisted).unwrap()).unwrap();
            assert!(EmbeddingCache::load(&path, "model", 1).entries.is_empty());
        }
    }

    #[test]
    fn in_memory_capacity_accounts_for_vector_bytes() {
        let cache = EmbeddingCache::empty("model", MAX_VECTOR_DIMENSION);
        assert!(cache.max_entries < DEFAULT_MAX_ENTRIES);
        assert!(
            cache.max_entries * (MAX_VECTOR_DIMENSION * 4 + ENTRY_OVERHEAD_BYTES)
                <= MAX_VECTOR_BYTES
        );
        let mut invalid = EmbeddingCache::empty("model", MAX_VECTOR_DIMENSION + 1);
        assert!(!invalid.insert("input", Arc::from(vec![1.0; MAX_VECTOR_DIMENSION + 1])));
    }
}
