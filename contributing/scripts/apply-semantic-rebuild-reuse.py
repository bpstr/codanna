#!/usr/bin/env python3
"""Temporary, hash-guarded source edit for the dedicated implementation branch."""
import hashlib
from pathlib import Path

simple = Path("src/semantic/simple.rs")
facade = Path("src/indexing/facade.rs")
if "semantic.restore_rebuild_cache(&semantic_path);" in facade.read_text():
    raise SystemExit("Repair already applied; do not apply another source edit")
for path, expected in [
    (simple, "5e49db67311a41cf10ed6cb89e36c1a9907d08bdaeaa347226f0150559cbee9f"),
    (facade, "edff5af2cb3109645a2870b1a5009e27f5af64fd5321163a5420f26ccf8fbe58"),
]:
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f"Source changed; review before applying: {path}: {actual}")

def once(text, before, after):
    if text.count(before) != 1:
        raise SystemExit("Expected exactly one reviewed edit location")
    return text.replace(before, after, 1)

text = once(simple.read_text(),
    "//! Simple semantic search implementation for documentation comments\n",
    "//! Simple semantic search implementation for documentation comments\n\n#[path = \"rebuild_cache.rs\"]\nmod rebuild_cache;\n")
text = once(text,
    """        if let Some(metadata) = &mut self.metadata {
            metadata.embedding_identity = Some(identity.clone());
        }
        self.embedding_cache =
            crate::embedding_cache::EmbeddingCache::empty(identity, self.dimensions);""",
    """        // An empty vector generation can still hold compatible cached inputs
        // (fresh rebuild or the last indexed file was removed). Revalidation
        // must not discard those vectors before the first new file batch.
        if self
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.embedding_identity.as_deref())
            == Some(identity.as_str())
        {
            return Ok(());
        }
        if let Some(metadata) = &mut self.metadata {
            metadata.embedding_identity = Some(identity.clone());
        }
        self.embedding_cache =
            crate::embedding_cache::EmbeddingCache::empty(identity, self.dimensions);""")
updated_facade = once(facade.read_text(),
    """        semantic.set_embedding_identity(
            backend.identity(self.settings.semantic_search.model_revision.as_deref()),
        )?;

        self.semantic_search = Some(Arc::new(Mutex::new(semantic)));""",
    """        semantic.set_embedding_identity(
            backend.identity(self.settings.semantic_search.model_revision.as_deref()),
        )?;
        // --force recreates symbol IDs, not embedding input meaning. Reuse only
        // exact inputs scoped to this backend identity; never old ID mappings.
        semantic.restore_rebuild_cache(&semantic_path);

        self.semantic_search = Some(Arc::new(Mutex::new(semantic)));""")
simple.write_text(text)
facade.write_text(updated_facade)
