from pathlib import Path

p = Path("src/semantic/simple.rs")
s = p.read_text()

old = """        let embedding = embeddings.into_iter().next().unwrap();

        // Validate dimensions
        if embedding.len() != self.dimensions {
"""
new = """        let embedding = embeddings.into_iter().next().unwrap();

        // Non-finite values make cosine ranking undefined and historically
        // could panic sorting through partial_cmp(...).unwrap(). Reject them
        // at the ingestion boundary instead of letting corruption propagate.
        if embedding.iter().any(|value| !value.is_finite()) {
            return Err(SemanticSearchError::EmbeddingError(
                "Embedding contains non-finite values".to_string(),
            ));
        }
        if embedding.len() != self.dimensions {
"""
if old not in s:
    raise SystemExit("index_doc_comment validation block not found")
s = s.replace(old, new, 1)

old = """            if embedding.len() == self.dimensions {
                self.embeddings.insert(symbol_id, embedding);
                self.symbol_languages.insert(symbol_id, language);
                count += 1;
            } else {
                dropped += 1;
            }
"""
new = """            if embedding.len() == self.dimensions
                && embedding.iter().all(|value| value.is_finite())
            {
                self.embeddings.insert(symbol_id, embedding);
                self.symbol_languages.insert(symbol_id, language);
                count += 1;
            } else {
                dropped += 1;
            }
"""
if old not in s:
    raise SystemExit("store_embeddings validation block not found")
s = s.replace(old, new, 1)
s = s.replace("b.1.partial_cmp(&a.1).unwrap()", "b.1.total_cmp(&a.1)")
p.write_text(s)
