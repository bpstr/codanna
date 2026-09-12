from pathlib import Path

storage = Path("src/semantic/storage.rs")
s = storage.read_text()
marker = '''    /// Returns the number of embeddings stored.
    pub fn embedding_count(&self) -> usize {
'''
method = '''    /// Saves borrowed embeddings without cloning their vector payloads.
    ///
    /// This is the persistence path used by `SimpleSemanticSearch::save` so
    /// snapshotting a large semantic index only allocates a small ID/slice
    /// descriptor vector instead of a second full copy of every embedding.
    pub fn save_borrowed_batch(
        &mut self,
        embeddings: &[(SymbolId, &[f32])],
    ) -> Result<(), SemanticSearchError> {
        for (_, embedding) in embeddings {
            if embedding.len() != self.dimension.get() {
                return Err(SemanticSearchError::DimensionMismatch {
                    expected: self.dimension.get(),
                    actual: embedding.len(),
                    suggestion: "All embeddings must have the same dimension".to_string(),
                });
            }
        }

        let mut vector_batch = Vec::with_capacity(embeddings.len());
        for (symbol_id, embedding) in embeddings {
            let vector_id = VectorId::new(symbol_id.to_u32()).ok_or_else(|| {
                SemanticSearchError::InvalidId {
                    id: symbol_id.to_u32(),
                    suggestion: "Symbol ID must be non-zero".to_string(),
                }
            })?;
            vector_batch.push((vector_id, *embedding));
        }

        self.storage
            .write_batch(&vector_batch)
            .map_err(|e| SemanticSearchError::StorageError {
                message: format!("Failed to save borrowed batch: {e}"),
                suggestion: "Check disk space and file permissions".to_string(),
            })
    }

'''
if marker not in s:
    raise SystemExit("semantic storage insertion marker not found")
storage.write_text(s.replace(marker, method + marker, 1))

simple = Path("src/semantic/simple.rs")
s = simple.read_text()
old = '''        // Convert HashMap to Vec for batch save
        let embeddings: Vec<(SymbolId, Vec<f32>)> = self
            .embeddings
            .iter()
            .map(|(id, embedding)| (*id, embedding.clone()))
            .collect();

        // Save all embeddings
        storage.save_batch(&embeddings)?;
'''
new = '''        // Build a descriptor batch borrowing the existing vectors. Do not
        // clone embedding payloads during persistence: on large repositories
        // that temporary copy can dominate peak RSS.
        let embeddings: Vec<(SymbolId, &[f32])> = self
            .embeddings
            .iter()
            .map(|(id, embedding)| (*id, embedding.as_slice()))
            .collect();

        storage.save_borrowed_batch(&embeddings)?;
'''
if old not in s:
    raise SystemExit("semantic save clone block not found")
simple.write_text(s.replace(old, new, 1))
