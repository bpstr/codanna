//! Ranking probes use deterministic source text and fixed local vectors only.
#![allow(dead_code)]

#[path = "../src/knowledge/context.rs"]
mod context;
#[path = "../src/knowledge/mod.rs"]
mod knowledge;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore, SearchQuery};
use codanna::vector::{EmbeddingGenerator, VectorDimension, VectorError};

fn graph() -> knowledge::Graph {
    let mut graph = knowledge::Graph::default();
    graph.repositories.insert("core".into(), Default::default());
    graph.repositories.insert("peer".into(), Default::default());
    for (id, repo, label, excerpt) in [
        ("exact", "core", "memo", "restarting replication"),
        ("near", "core", "sync_once", "restart replica"),
        ("substring", "core", "discard", "partial grouping"),
        ("foreign", "peer", "restarting", "replication"),
        ("unrelated", "core", "other", "astronomy calendar"),
    ] {
        let path = format!("src/{id}.rs");
        let hash = knowledge::digest(excerpt);
        graph
            .repositories
            .get_mut(repo)
            .unwrap()
            .files
            .insert(path.clone(), hash.clone());
        graph.nodes.insert(
            id.into(),
            knowledge::Node {
                id: id.into(),
                kind: knowledge::Kind::Symbol,
                label: label.into(),
                excerpt: excerpt.into(),
                source: knowledge::Span {
                    repo: repo.into(),
                    path,
                    start_line: 1,
                    end_line: 1,
                    hash,
                },
            },
        );
    }
    graph.edges.push(knowledge::Edge {
        from: "exact".into(),
        to: "unrelated".into(),
        relation: "calls".into(),
        basis: knowledge::Basis::Resolved,
        evidence: graph.nodes["exact"].source.clone(),
        method: "fixture".into(),
    });
    graph.validate().unwrap();
    graph
}

#[test]
fn context_matches_word_forms_without_synonyms_or_implicit_traversal() {
    let graph = graph();
    let request = context::Request {
        query: "restarting replication".into(),
        repo: Some("core".into()),
        max_nodes: 5,
        max_depth: 0,
        ..Default::default()
    };
    let bundle = context::get(&graph, &request).unwrap();
    let ids: Vec<_> = bundle
        .items
        .iter()
        .map(|item| item.node.id.as_str())
        .collect();
    assert_eq!(ids.first(), Some(&"exact"), "exact coverage must win");
    assert!(
        ids.contains(&"near"),
        "strong surface overlap should recover word forms"
    );
    assert!(!ids.contains(&"foreign"));
    assert!(
        !ids.contains(&"unrelated"),
        "depth zero must not expand a resolved edge"
    );
    assert_eq!(
        serde_json::to_vec(&bundle).unwrap(),
        serde_json::to_vec(&context::get(&graph, &request).unwrap()).unwrap()
    );
    for text in ["art", "qzvnoexist75391", "🙂🙂🙂"] {
        let bundle = context::get(
            &graph,
            &context::Request {
                query: text.into(),
                repo: Some("core".into()),
                max_depth: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            bundle.items.is_empty(),
            "no substring/absent-term result for {text}"
        );
    }
}

#[test]
fn context_exact_entity_precedes_changed_file_and_small_budgets_remain_bounded() {
    let graph = graph();
    let bundle = context::get(
        &graph,
        &context::Request {
            query: "restarting replication".into(),
            entities: vec!["near".into()],
            files: vec!["src/near.rs".into(), "src/exact.rs".into()],
            repo: Some("core".into()),
            max_nodes: 1,
            max_depth: 0,
            max_bytes: 2048,
        },
    )
    .unwrap();
    assert_eq!(bundle.items.len(), 1);
    assert_eq!(bundle.items[0].node.id, "near");
    assert_eq!(bundle.items[0].reason, "explicit entity seed");
    assert!(serde_json::to_vec(&bundle).unwrap().len() <= 2048);
    assert!(bundle.truncated);
}

fn index(root: &Path, files: &[(&str, String)]) -> DocumentStore {
    let mut paths = Vec::new();
    for (name, contents) in files {
        let path = root.join(name);
        std::fs::write(&path, contents).unwrap();
        paths.push(path);
    }
    let mut store =
        DocumentStore::new(root.join("index"), VectorDimension::new(2).unwrap()).unwrap();
    store
        .index_collection(
            "docs",
            &CollectionConfig {
                paths,
                ..Default::default()
            },
            &ChunkingConfig {
                min_chunk_chars: 1,
                max_chunk_chars: 120,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    store
}

fn query(text: &str, limit: usize) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        limit,
        ..Default::default()
    }
}

#[test]
fn lexical_diversity_reaches_other_sources_beyond_a_long_matching_source() {
    let temp = tempfile::tempdir().unwrap();
    let long = format!("# Parcel quota\n\n{}", (0..200).map(|i|
        format!("Parcel quota parcel quota records batch {i}. This paragraph repeats the shared rule for each processing batch.\n\n")
    ).collect::<String>());
    let mut store = index(temp.path(), &[
        ("policy.md", long),
        ("reference.md", "# Reference\n\nParcel quota is recorded by the control.\n".into()),
        ("guide.md", "# Shipping parcels\n\nUse the quota configured in the control before sending each package.\n".into()),
        ("operations.md", "# Operations\n\nThe quota record belongs in the dispatch journal.\n".into()),
        ("unrelated.md", "# Astronomy\n\nThe lunar calendar is maintained separately.\n".into()),
    ]);
    let hits = store.search(query("parcel quota", 5)).unwrap();
    assert_eq!(hits.len(), 5);
    assert_eq!(hits[0].source_path.file_name().unwrap(), "policy.md");
    assert!(
        hits.iter()
            .any(|hit| hit.source_path.file_name().unwrap() == "guide.md")
    );
    let mut counts = BTreeMap::new();
    for hit in &hits {
        *counts.entry(&hit.source_path).or_insert(0) += 1;
        let source = std::fs::read_to_string(&hit.source_path).unwrap();
        let evidence = &source[hit.byte_range.0..hit.byte_range.1];
        assert!(
            evidence.to_lowercase().contains("quota")
                || hit
                    .heading_context
                    .iter()
                    .any(|h| h.to_lowercase().contains("quota"))
        );
    }
    assert!(counts.len() >= 3);
    assert!(counts.values().all(|count| *count <= 2));
    assert!(
        !hits
            .iter()
            .any(|hit| hit.source_path.ends_with("unrelated.md"))
    );
    let repeated = store.search(query("parcel quota", 5)).unwrap();
    assert_eq!(
        serde_json::to_vec(&hits).unwrap(),
        serde_json::to_vec(&repeated).unwrap()
    );
}

#[test]
fn lexical_reranking_covers_full_question_before_repeated_partial_headings() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = index(
        temp.path(),
        &[
            (
                "repeated.md",
                "# Quota\n\nquota quota quota quota quota quota\n".into(),
            ),
            (
                "complete.md",
                "# Guidance\n\nParcel quota is configured here.\n".into(),
            ),
            (
                "inflected.md",
                "# Shipping parcels\n\nRead the quota before sending.\n".into(),
            ),
        ],
    );
    let hits = store.search(query("parcel quota", 3)).unwrap();
    assert_eq!(hits[0].source_path.file_name().unwrap(), "complete.md");
    assert_eq!(hits[1].source_path.file_name().unwrap(), "inflected.md");
    assert_eq!(hits[2].source_path.file_name().unwrap(), "repeated.md");
    assert!(
        store.search(query("shipments", 5)).unwrap().is_empty(),
        "stemming may rerank a literal hit, never manufacture candidates"
    );
}

#[test]
fn one_matching_source_keeps_evidence_and_document_collection_and_limit_filters() {
    let temp = tempfile::tempdir().unwrap();
    let long = (0..12).map(|i| format!("# Part {i}\n\nchecksum readout batch {i} contains the recorded comparison value.\n\n")).collect();
    let mut store = index(
        temp.path(),
        &[
            ("one.md", long),
            ("noise.md", "# Astronomy\n\nNo relevant facts.\n".into()),
        ],
    );
    for limit in [1, 2, 5, 9, 30] {
        let hits = store.search(query("checksum", limit)).unwrap();
        assert_eq!(hits.len(), limit.min(12));
        assert!(hits.iter().all(|hit| hit.source_path.ends_with("one.md")));
        assert_eq!(
            hits.iter()
                .map(|hit| hit.chunk_id)
                .collect::<HashSet<_>>()
                .len(),
            hits.len()
        );
    }
    assert!(store.search(query("checksum", 0)).unwrap().is_empty());
    assert!(
        store
            .search(query("qzvnoexist75391", 5))
            .unwrap()
            .is_empty()
    );
    let mut scoped = query("checksum", 5);
    scoped.collection = Some("missing".into());
    assert!(store.search(scoped).unwrap().is_empty());
    let mut scoped = query("checksum", 5);
    scoped.document = Some(temp.path().join("noise.md"));
    assert!(store.search(scoped).unwrap().is_empty());
    let mut scoped = query("checksum", 5);
    scoped.document = Some(temp.path().join("one.md"));
    assert_eq!(store.search(scoped).unwrap().len(), 5);
}

struct FixedVectors;
impl EmbeddingGenerator for FixedVectors {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        Ok(texts
            .iter()
            .map(|text| {
                if text.contains("complement") {
                    vec![0.96, 0.28]
                } else if text.contains("operations") {
                    vec![0.95, (1.0_f32 - 0.95 * 0.95).sqrt()]
                } else if text.contains("weak") {
                    vec![0.6, 0.8]
                } else if text.contains("opposite") {
                    vec![-1.0, 0.0]
                } else if text.contains("north") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }
    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }
    fn cache_identity(&self) -> String {
        "ranking-fixture@1".into()
    }
}

fn enable_fixed_embeddings(store: DocumentStore) -> DocumentStore {
    enable_embeddings(store, FixedVectors)
}

fn enable_embeddings(
    store: DocumentStore,
    generator: impl EmbeddingGenerator + 'static,
) -> DocumentStore {
    let paths = store.get_indexed_paths();
    let mut store = store.with_embeddings(Box::new(generator)).unwrap();
    store
        .index_collection(
            "docs",
            &CollectionConfig {
                paths,
                ..Default::default()
            },
            &ChunkingConfig {
                min_chunk_chars: 1,
                max_chunk_chars: 120,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    store
}

struct MagnitudeVectors;
impl EmbeddingGenerator for MagnitudeVectors {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        Ok(texts
            .iter()
            .map(|text| {
                if text.contains("overflowing") {
                    vec![1e30, 1e30]
                } else if text.contains("high query") {
                    vec![1e10, 1e10]
                } else if text.contains("zero") {
                    vec![0.0, 0.0]
                } else {
                    vec![1.0, 0.0]
                }
            })
            .collect())
    }

    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }

    fn cache_identity(&self) -> String {
        "magnitude-ranking-fixture@1".into()
    }
}

#[test]
fn overflowing_finite_vectors_return_an_explicit_error_not_empty_or_partial_hits() {
    let temp = tempfile::tempdir().unwrap();
    let vectors = MagnitudeVectors
        .generate_embeddings(&["overflowing", "high query", "ordinary"])
        .unwrap();
    assert!(vectors.iter().flatten().all(|value| value.is_finite()));
    let mut store = enable_embeddings(
        index(
            temp.path(),
            &[
                ("large.md", "overflowing vector magnitude".into()),
                ("ordinary.md", "ordinary direction".into()),
            ],
        ),
        MagnitudeVectors,
    );
    let error = store.search(query("high query", 5)).unwrap_err();
    let codanna::documents::store::DocumentStoreError::Embedding(message) = error else {
        panic!("expected a structured embedding error, got {error}");
    };
    assert!(message.contains("Non-finite cosine similarity"));
    assert!(message.contains("Suggestion:"));
    assert!(message.contains("normalized vectors"));

    // The ordinary candidate remains valid, but it must not mask the invalid
    // candidate in the unfiltered query. Source filtering excludes it safely.
    let mut scoped = query("high query", 5);
    scoped.document = Some(temp.path().join("ordinary.md"));
    let hits = store.search(scoped).unwrap();
    assert_eq!(hits.len(), 1);
    assert!((hits[0].similarity - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
}

#[test]
fn zero_vectors_keep_defined_finite_cosine_scores() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = enable_embeddings(
        index(
            temp.path(),
            &[
                ("zero.md", "zero direction".into()),
                ("ordinary.md", "ordinary direction".into()),
            ],
        ),
        MagnitudeVectors,
    );
    let hits = store.search(query("ordinary", 2)).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].similarity, 1.0);
    assert_eq!(hits[1].similarity, 0.0);
    let hits = store.search(query("zero query", 2)).unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.similarity == 0.0));
}

#[test]
fn semantic_diversity_recovers_close_positive_sources_without_weak_fill() {
    let temp = tempfile::tempdir().unwrap();
    let strong = (0..8)
        .map(|i| format!("# Part {i}\n\nnorth records batch {i} in the shared implementation.\n\n"))
        .collect();
    let mut store = enable_fixed_embeddings(index(
        temp.path(),
        &[
            ("strong.md", strong),
            (
                "complement.md",
                "# Procedure\n\ncomplement policy for the caller.\n\n# Verification\n\ncomplement control for the operator.\n".into(),
            ),
            ("operations.md", "# Guide\n\noperations evidence.\n".into()),
            ("weak.md", "# Weak\n\nweak similarity.\n".into()),
            ("zero.md", "south".into()),
            ("opposite.md", "opposite".into()),
        ],
    ));
    let hits = store.search(query("north", 5)).unwrap();
    assert_eq!(hits.len(), 5);
    assert_eq!(hits[0].similarity, 1.0);
    assert!(
        hits.iter()
            .any(|hit| hit.source_path.ends_with("complement.md")
                && (hit.similarity - 0.96).abs() < 1e-5)
    );
    assert!(
        hits.iter()
            .any(|hit| hit.source_path.ends_with("operations.md")
                && (hit.similarity - 0.95).abs() < 1e-5)
    );
    assert!(hits.iter().all(|hit| hit.similarity >= 0.9));
    let mut counts = BTreeMap::new();
    for hit in &hits {
        *counts.entry(&hit.source_path).or_insert(0) += 1;
    }
    assert!(counts.len() >= 3);
    assert!(counts.values().all(|count| *count <= 2));
    let mut scoped = query("north", 5);
    scoped.document = Some(temp.path().join("strong.md"));
    let hits = store.search(scoped).unwrap();
    assert_eq!(
        hits.len(),
        5,
        "a single source must not lose valid candidates to quotas"
    );
    assert!(hits.iter().all(|hit| hit.similarity == 1.0));
    assert_eq!(store.search(query("north", 1)).unwrap().len(), 1);
    assert!(store.search(query("north", 0)).unwrap().is_empty());
}

#[test]
fn semantic_diversity_prefers_new_sources_before_second_sections() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = enable_fixed_embeddings(index(
        temp.path(),
        &[
            ("strong.md", "# First\n\nnorth first section.\n\n# Second\n\nnorth second section.\n".into()),
            ("a.md", "complement evidence a".into()),
            ("b.md", "complement evidence b".into()),
            ("c.md", "operations evidence c".into()),
            ("d.md", "operations evidence d".into()),
            ("weak.md", "weak evidence".into()),
        ],
    ));
    let hits = store.search(query("north", 5)).unwrap();
    assert_eq!(hits.len(), 5);
    assert_eq!(hits[0].similarity, 1.0);
    assert_eq!(hits.iter().map(|hit| &hit.source_path).collect::<HashSet<_>>().len(), 5);
    assert!(hits.iter().all(|hit| hit.similarity >= 0.9));
}

#[test]
fn semantic_nonpositive_cutoff_does_not_expand_to_worse_vectors() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = enable_fixed_embeddings(index(
        temp.path(),
        &[
            ("positive.md", "north".into()),
            ("zero.md", "south".into()),
            ("negative.md", "opposite".into()),
        ],
    ));
    let hits = store.search(query("north", 2)).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].similarity, 1.0);
    assert_eq!(hits[1].similarity, 0.0);
    assert!(
        !hits
            .iter()
            .any(|hit| hit.source_path.ends_with("negative.md"))
    );
}

#[test]
fn semantic_scores_and_filters_remain_cosine_with_stable_ties() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = index(
        temp.path(),
        &[
            ("a.md", "north first".into()),
            ("b.md", "north second".into()),
            ("c.md", "south".into()),
        ],
    )
    .with_embeddings(Box::new(FixedVectors))
    .unwrap();
    let paths: Vec<PathBuf> = ["a.md", "b.md", "c.md"]
        .into_iter()
        .map(|p| temp.path().join(p))
        .collect();
    store
        .index_collection(
            "docs",
            &CollectionConfig {
                paths,
                ..Default::default()
            },
            &ChunkingConfig {
                min_chunk_chars: 1,
                max_chunk_chars: 120,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    let hits = store.search(query("north", 2)).unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| (hit.similarity - 1.0).abs() < 1e-6));
    assert!(hits[0].chunk_id.get() < hits[1].chunk_id.get());
    let second = store.search(query("north", 2)).unwrap();
    assert_eq!(
        serde_json::to_vec(&hits).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    let mut scoped = query("north", 5);
    scoped.document = Some(temp.path().join("c.md"));
    let hits = store.search(scoped).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].similarity, 0.0);
    let mut scoped = query("north", 5);
    scoped.collection = Some("missing".into());
    assert!(store.search(scoped).unwrap().is_empty());
}
