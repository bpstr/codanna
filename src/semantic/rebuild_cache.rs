//! A rebuild may reuse content-addressed vectors, never prior symbol IDs.

use super::SimpleSemanticSearch;
use std::path::Path;

impl SimpleSemanticSearch {
    /// Load only the optional accelerator after binding the actual backend.
    /// Never load old symbol mappings or an old persistence generation here.
    pub(crate) fn restore_rebuild_cache(&mut self, semantic_path: &Path) {
        if !self.embeddings.is_empty() {
            return;
        }
        let Some(identity) = self
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.embedding_identity.as_deref())
        else {
            // Model name or equal dimensions alone are not enough evidence.
            return;
        };
        self.embedding_cache = crate::embedding_cache::EmbeddingCache::load(
            &semantic_path.join("embedding-cache.json"),
            identity,
            self.dimensions,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SymbolId;

    fn prepared(path: &Path) -> SimpleSemanticSearch {
        let mut search = SimpleSemanticSearch::new_empty(2, "fixture");
        search.set_embedding_identity("backend@revision:policy".into()).unwrap();
        let id = SymbolId::new(11).unwrap();
        search.store_embeddings_with_inputs(
            vec![(id, vec![1.0, 0.0], "rust".into())],
            &[(id, "exact input", "rust")],
        );
        search.save(path).unwrap();
        search
    }

    #[test]
    fn restoring_cache_does_not_restore_old_ids_or_publish_vectors() {
        let temp = tempfile::tempdir().unwrap();
        let old = prepared(temp.path());
        let mut fresh = SimpleSemanticSearch::new_empty(2, "fixture");
        fresh.set_embedding_identity("backend@revision:policy".into()).unwrap();
        fresh.restore_rebuild_cache(temp.path());
        assert_eq!(fresh.embedding_count(), 0);
        // The facade revalidates the same identity before the first file batch.
        fresh.set_embedding_identity("backend@revision:policy".into()).unwrap();
        let id = SymbolId::new(22).unwrap();
        assert!(fresh.reuse_cached_embeddings(&[(id, "exact input", "typescript")]).is_empty());
        assert_eq!(fresh.embedding_count(), 1);
        assert!(!fresh.embeddings.contains_key(&SymbolId::new(11).unwrap()));
        assert_eq!(fresh.embeddings[&id].as_ref(), &[1.0, 0.0]);
        assert!(fresh.language_symbols["typescript"].contains(&id));
        assert!(fresh.language_symbols.get("rust").is_none());
        assert_eq!(old.embedding_count(), 1);
    }

    #[test]
    fn rebinding_after_last_vector_removal_retains_exact_input_cache() {
        let temp = tempfile::tempdir().unwrap();
        let mut search = prepared(temp.path());
        search.clear();
        search.set_embedding_identity("backend@revision:policy".into()).unwrap();
        assert!(search.reuse_cached_embeddings(&[(SymbolId::new(22).unwrap(), "exact input", "rust")]).is_empty());
    }

    #[test]
    fn incompatible_identity_or_dimensions_must_not_restore_vectors() {
        let temp = tempfile::tempdir().unwrap();
        let _old = prepared(temp.path());
        for (identity, dimension) in [("different-backend:policy", 2), ("backend@revision:policy", 3)] {
            let mut fresh = SimpleSemanticSearch::new_empty(dimension, "fixture");
            fresh.set_embedding_identity(identity.into()).unwrap();
            fresh.restore_rebuild_cache(temp.path());
            assert_eq!(fresh.reuse_cached_embeddings(&[(SymbolId::new(22).unwrap(), "exact input", "rust")]).len(), 1);
            assert_eq!(fresh.embedding_count(), 0);
        }
    }

    #[test]
    fn unbound_identity_cannot_restore_cache_by_model_name() {
        let temp = tempfile::tempdir().unwrap();
        let _old = prepared(temp.path());
        let mut fresh = SimpleSemanticSearch::new_empty(2, "fixture");
        fresh.restore_rebuild_cache(temp.path());
        assert_eq!(fresh.reuse_cached_embeddings(&[(SymbolId::new(22).unwrap(), "exact input", "rust")]).len(), 1);
    }

    #[test]
    fn changing_empty_index_identity_discards_the_previous_input_cache() {
        let temp = tempfile::tempdir().unwrap();
        let mut search = prepared(temp.path());
        search.clear();
        search.set_embedding_identity("another-backend:policy".into()).unwrap();
        assert_eq!(search.reuse_cached_embeddings(&[(SymbolId::new(22).unwrap(), "exact input", "rust")]).len(), 1);
    }
}
