//! Prepared vectors prove representation and persistence mechanics, not model quality.
use crate::embedding_input::InputBudget;
use crate::indexing::{
    calculate_hash,
    pipeline::{
        FileContent,
        stages::parse::{ParseStage, init_parser_cache},
    },
};
use crate::semantic::{SimpleSemanticSearch, SymbolSegment};
use crate::symbol_representation::*;
use crate::{Settings, SymbolId};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

fn storage_bytes(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                storage_bytes(&entry.path())
            } else {
                entry.metadata().unwrap().len()
            }
        })
        .sum()
}

#[test]
fn paired_representations_reopen_prepared_vectors_with_bounded_parent_retrieval() {
    let temp = tempfile::tempdir().unwrap();
    let source = format!(
        "/** Merge the latest page. */\nexport function mergePage(current: number[], incoming: number[]) {{ const values = [...new Set([...current, ...incoming])]; {} return values; }}\nexport function undocumented(values: number[]) {{ return [...new Set(values)]; }}\n/** Merge the latest page. */\nexport function unrelated(values: number[]) {{ return values.reverse(); }}\n",
        "void 0; ".repeat(5000)
    );
    let mut measurements = Vec::new();
    for (label, policy) in [
        ("comment", CodeEmbeddingPolicy::DocComment),
        ("body", CodeEmbeddingPolicy::SymbolBodyV2),
    ] {
        let mut settings = Settings {
            workspace_root: Some(temp.path().into()),
            ..Default::default()
        };
        assert_eq!(
            settings.semantic_search.code_representation,
            CodeEmbeddingPolicy::DocComment
        );
        settings.semantic_search.enabled = true;
        settings.semantic_search.code_representation = policy;
        let settings = Arc::new(settings);
        init_parser_cache(settings.clone());
        let parsed = ParseStage::new(settings)
            .parse(FileContent::new(
                temp.path().join("page.ts"),
                source.clone(),
                calculate_hash(&source),
            ))
            .unwrap();
        let mut semantic = SimpleSemanticSearch::new_empty(2, "prepared-f01");
        let identity = policy.bind_identity(serde_json::json!({"backend":"prepared", "model":"prepared-f01", "input_policy":"fixture-1024-bytes"}).to_string());
        semantic.set_embedding_identity(identity.clone()).unwrap();
        let budget = InputBudget::remote(Some(1024), None).unwrap();
        let mut expected_vectors = 0;
        let mut retained = 0;
        for (id, name) in [(1, "mergePage"), (2, "undocumented"), (3, "unrelated")] {
            let symbols: Vec<_> = parsed
                .raw_symbols
                .iter()
                .filter(|s| s.name.as_ref() == name)
                .collect();
            assert_eq!(symbols.len(), 1);
            let symbol = symbols[0];
            if !policy.eligible(symbol.kind, symbol.doc_comment.is_some()) {
                continue;
            }
            let id = SymbolId::new(id).unwrap();
            if policy == CodeEmbeddingPolicy::DocComment {
                assert!(symbol.embedding_source.is_none());
                assert!(!symbol.doc_comment.as_deref().unwrap().contains("new Set"));
                semantic.store_embeddings(vec![(id, vec![0.0, 1.0], "typescript".into())]);
                expected_vectors += 1;
            } else {
                let source = symbol.embedding_source.as_ref().unwrap();
                assert!(
                    source.fragments.iter().map(|f| f.text.len()).sum::<usize>()
                        <= MAX_SYMBOL_SOURCE_BYTES
                );
                assert!(
                    source.header.len() < 2048,
                    "identity fields have separate bounded lengths"
                );
                retained += source.retained_bytes();
                let inputs = source.inputs(&budget).unwrap();
                assert!(inputs.len() <= MAX_SYMBOL_SEGMENTS);
                if name == "mergePage" {
                    assert_eq!(
                        inputs.len(),
                        MAX_SYMBOL_SEGMENTS,
                        "oversized source must exercise segmentation"
                    );
                    let captured: usize = source.fragments.iter().map(|f| f.text.len()).sum();
                    assert!(
                        captured < 40_000,
                        "bounded excerpts must omit oversized source"
                    );
                }
                let segments = inputs
                    .iter()
                    .map(|input| {
                        // Deliberately fixed synthetic oracle: this is not an embedding model.
                        let vector = if input.text.contains("new Set") {
                            vec![1.0, 0.0]
                        } else {
                            vec![0.0, 1.0]
                        };
                        SymbolSegment::new(input.source_range.clone(), Arc::from(vector))
                    })
                    .collect();
                expected_vectors += inputs.len();
                semantic
                    .store_symbol_segments(id, segments, &inputs, "typescript")
                    .unwrap();
            }
        }
        assert!(retained <= MAX_FILE_SOURCE_BYTES);
        let path = temp.path().join(label);
        semantic.save(&path).unwrap();
        let bytes = storage_bytes(&path);
        assert!(bytes >= (expected_vectors * 2 * std::mem::size_of::<f32>()) as u64);
        let reopened = SimpleSemanticSearch::load_remote(&path).unwrap();
        assert_eq!(reopened.vector_count(), expected_vectors);
        assert_eq!(
            reopened.metadata().unwrap().embedding_identity.as_deref(),
            Some(identity.as_str())
        );
        let hits = reopened
            .search_with_embedding(&[1.0, 0.0], 10, 0.9)
            .unwrap();
        let ids: BTreeSet<_> = hits.iter().map(|(id, _)| id.value()).collect();
        assert_eq!(
            hits.len(),
            ids.len(),
            "segments must collapse to real parent IDs"
        );
        assert!(
            !ids.contains(&3),
            "unrelated behavior is a negative control"
        );
        if policy == CodeEmbeddingPolicy::DocComment {
            assert_eq!(reopened.embedding_count(), 2);
            assert!(hits.is_empty());
        } else {
            assert_eq!(reopened.embedding_count(), 3);
            assert_eq!(ids, BTreeSet::from([1, 2]));
        }
        assert_eq!(
            storage_bytes(&path),
            bytes,
            "read-only queries must not rewrite persistence"
        );
        measurements.push((label, reopened.embedding_count(), expected_vectors, bytes));
    }
    assert!(measurements[1].2 > measurements[0].2);
    assert!(measurements[1].3 > measurements[0].3);
    eprintln!(
        "F01 prepared fixture (policy, parents, physical vectors, persisted bytes): {measurements:?}"
    );
}
