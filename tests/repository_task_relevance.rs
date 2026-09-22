//! Source-grounded relevance measurement, not a synthetic perfect-score gate.
//! Expected owner labels and queries stay outside the indexed source directory.

use codanna::indexing::{IndexFacade, calculate_hash};
use codanna::storage::SearchResult;
use codanna::Settings;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;

const ORACLE: &str = include_str!("../contributing/retrieval/evaluations/repository-tasks.json");
const SOURCES: &[(&str, &str)] = &[
    (
        "src/indexing/pipeline/stages/read.rs",
        include_str!("../src/indexing/pipeline/stages/read.rs"),
    ),
    (
        "src/indexing/pipeline/stages/discover.rs",
        include_str!("../src/indexing/pipeline/stages/discover.rs"),
    ),
    (
        "src/mcp/tools/recall.rs",
        include_str!("../src/mcp/tools/recall.rs"),
    ),
    ("src/embedding_cache.rs", include_str!("../src/embedding_cache.rs")),
    ("src/memory.rs", include_str!("../src/memory.rs")),
];

fn fixture() -> (tempfile::TempDir, IndexFacade, Value) {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("corpus");
    for &(path, source) in SOURCES {
        assert_eq!(calculate_hash(source), oracle["files"][path].as_str().unwrap(),
            "frozen source changed: {path}; update judgments deliberately, not silently");
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, source).unwrap();
    }
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.join("src")).unwrap();
    let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
    index.index_directory(&root.join("src"), true).unwrap();
    (temp, index, oracle)
}

fn owner_rank(results: &[SearchResult], name: &str, path: &str) -> Option<usize> {
    results
        .iter()
        .position(|hit| hit.name == name && hit.file_path == path)
        .map(|position| position + 1)
}

#[test]
fn repository_task_relevance_records_misses_instead_of_claiming_perfect_recall() {
    let (_temp, index, oracle) = fixture();
    let mut hits = [0usize; 2];
    let mut reciprocal_ranks = [0.0f64; 2];
    let mut classified = [0usize; 3];
    let mut measured = 0;
    let mut query_micros = 0u128;
    for task in oracle["tasks"].as_array().unwrap() {
        let name = task["name"].as_str().unwrap();
        let path = task["path"].as_str().unwrap();
        let owners: Vec<_> = index
            .find_symbols_by_name(name, Some("rust"))
            .into_iter()
            .filter(|symbol| symbol.file_path.as_ref() == path)
            .collect();
        assert_eq!(owners.len(), 1, "invalid owner judgment for {name} at {path}");
        for (variant, query) in task["queries"].as_array().unwrap().iter().enumerate() {
            assert!(variant < 2);
            let query = query.as_str().unwrap();
            let start = Instant::now();
            let results = index.search(query, 5, None, None, Some("rust")).unwrap();
            query_micros += start.elapsed().as_micros();
            let expanded = index.search(query, 200, None, None, Some("rust")).unwrap();
            let rank = owner_rank(&results, name, path);
            let expanded_rank = owner_rank(&expanded, name, path);
            let classification = if let Some(rank) = rank {
                hits[variant] += 1;
                reciprocal_ranks[variant] += 1.0 / rank as f64;
                classified[0] += 1;
                "owner_in_top_5"
            } else if expanded_rank.is_some() {
                classified[1] += 1;
                "owner_retrievable_at_200_not_top_5"
            } else {
                classified[2] += 1;
                "owner_not_retrieved_with_200_budget"
            };
            println!("repository_task_relevance={}", json!({
                "task": task["id"], "variant": if variant == 0 {"operational"} else {"paraphrase"},
                "query": query, "owner_name": name, "owner_path": path,
                "owner_documented": owners[0].doc_comment.is_some(),
                "rank_at_5": rank, "expanded_coverage_rank_at_200": expanded_rank,
                "classification": classification,
                "top_5": results.iter().map(|hit| json!({
                    "name": hit.name, "path": hit.file_path, "raw_score": hit.score
                })).collect::<Vec<_>>(),
            }));
            measured += 1;
        }
    }
    let tasks = oracle["tasks"].as_array().unwrap().len();
    assert_eq!(measured, tasks * 2);
    assert_eq!(classified.iter().sum::<usize>(), measured);
    println!("repository_task_summary={}", json!({
        "oracle_sha256": calculate_hash(ORACLE), "source_files": SOURCES.len(),
        "indexed_symbols": index.symbol_count(), "tasks": tasks, "queries": measured,
        "operational_hit_at_5": hits[0], "paraphrase_hit_at_5": hits[1],
        "operational_mrr_at_5": reciprocal_ranks[0] / tasks as f64,
        "paraphrase_mrr_at_5": reciprocal_ranks[1] / tasks as f64,
        "owner_in_top_5": classified[0], "owner_only_at_200": classified[1],
        "owner_not_retrieved_at_200": classified[2], "measured_query_microseconds": query_micros,
        "measurement_only": true, "semantic_indexing": false,
    }));
    // This assertion protects measurement completeness. It is not a quality
    // threshold: observed misses must stay visible until independently repaired.
}

#[test]
fn repository_task_relevance_retains_exact_identifier_and_absence_controls() {
    let (_temp, index, oracle) = fixture();
    for control in oracle["identifier_controls"].as_array().unwrap() {
        let results = index
            .search(control["query"].as_str().unwrap(), 5, None, None, Some("rust"))
            .unwrap();
        assert_eq!(
            owner_rank(&results, control["name"].as_str().unwrap(), control["path"].as_str().unwrap()),
            Some(1), "exact identifier control: {control}"
        );
    }
    assert!(index
        .search(oracle["absent_control"].as_str().unwrap(), 5, None, None, Some("rust"))
        .unwrap().is_empty());
}
