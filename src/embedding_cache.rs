//! Bounded, content-addressed embedding reuse shared by code and document indexing.
//!
//! This is an optional accelerator, never a source of truth. A missing, stale, or
//! malformed cache is treated as empty so it cannot prevent an index rebuild.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

const FORMAT_VERSION: u32 = 1;
const PREPROCESSING_VERSION: u32 = 1;
const DEFAULT_MAX_ENTRIES: usize = 4096;

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
    entries: Vec<PersistedEntry>,
}

#[derive(Serialize, Deserialize)]
struct PersistedEntry {
    input_sha256: String,
    embedding: Vec<f32>,
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
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return cache,
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
        if embedding.len() != self.dimension || !embedding.iter().all(|value| value.is_finite()) {
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
        serde_json::to_writer(&mut temp, &persisted).map_err(io::Error::other)?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|error| error.error)?;
        Ok(())
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
}
