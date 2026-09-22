//! Bounded relevance alternatives on the unchanged PR #58 corpus and judgments.
//! Measurement is not a passing quality gate. No model or provider is initialized.

use codanna::indexing::{IndexFacade, calculate_hash};
use codanna::storage::SearchResult;
use codanna::{RelationKind, Settings, SymbolId};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tantivy::tokenizer::{Language, LowerCaser, SimpleTokenizer, Stemmer, TextAnalyzer};

const ORACLE: &str = include_str!("../contributing/retrieval/evaluations/repository-tasks.json");
const SOURCES: &[(&str, &str)] = &[
    (
        "src/indexing/pipeline/stages/read.rs",
        include_str!("fixtures/repository_task_sources/read.rs.fixture"),
    ),
    (
        "src/indexing/pipeline/stages/discover.rs",
        include_str!("fixtures/repository_task_sources/discover.rs.fixture"),
    ),
    (
        "src/mcp/tools/recall.rs",
        include_str!("fixtures/repository_task_sources/recall.rs.fixture"),
    ),
    (
        "src/embedding_cache.rs",
        include_str!("fixtures/repository_task_sources/embedding_cache.rs.fixture"),
    ),
    (
        "src/memory.rs",
        include_str!("fixtures/repository_task_sources/memory.rs.fixture"),
    ),
];

fn fixture() -> (tempfile::TempDir, IndexFacade, Value) {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    assert_eq!(
        calculate_hash(ORACLE),
        "129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6"
    );
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("corpus");
    for &(path, source) in SOURCES {
        assert_eq!(calculate_hash(source), oracle["files"][path]);
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

// Same stop-word policy as the current discovery path: token boundaries and
// stemming are measured independently, without a query-specific synonym list.
const STOPWORDS: &[&str] = &[
    "and", "are", "for", "from", "how", "into", "not", "the", "this", "that", "to", "was", "were",
    "what", "where", "which", "with",
];

fn identifier_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<_> = text.chars().collect();
    for (position, &character) in chars.iter().enumerate() {
        if position > 0 && character.is_uppercase() {
            let previous = chars[position - 1];
            let next_lower = chars
                .get(position + 1)
                .is_some_and(|next| next.is_lowercase());
            if previous.is_lowercase() || (previous.is_uppercase() && next_lower) {
                out.push(' ');
            }
        }
        out.push(if character == '_' { ' ' } else { character });
    }
    out
}

fn tokens(text: &str, stem: bool) -> BTreeSet<String> {
    let text = identifier_words(text);
    let mut analyzer = if stem {
        TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(LowerCaser)
            .filter(Stemmer::new(Language::English))
            .build()
    } else {
        TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(LowerCaser)
            .build()
    };
    let mut stream = analyzer.token_stream(&text);
    let mut result = BTreeSet::new();
    while stream.advance() {
        let word = &stream.token().text;
        if word.len() >= 3 && !STOPWORDS.contains(&word.as_str()) {
            result.insert(word.clone());
        }
    }
    result
}

fn coverage(row: &SearchResult, query: &BTreeSet<String>, stem: bool, paths: bool) -> usize {
    let mut evidence = tokens(&row.name, stem);
    for field in [
        row.doc_comment.as_deref(),
        row.signature.as_deref(),
        row.context.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        evidence.extend(tokens(field, stem));
    }
    if paths {
        evidence.extend(tokens(&row.file_path, stem));
        evidence.extend(tokens(&row.module_path, stem));
    }
    query.intersection(&evidence).count()
}

fn ordered_ids(pool: &[SearchResult], query: &str, mode: &str) -> Vec<SymbolId> {
    let stem = mode.starts_with("stem");
    let terms = tokens(query, stem);
    let mut rows: Vec<_> = pool
        .iter()
        .map(|row| {
            let score = if mode == "raw" {
                0
            } else {
                coverage(row, &terms, stem, mode.ends_with("paths"))
            };
            (row, score)
        })
        .collect();
    rows.sort_by(|(left, left_key), (right, right_key)| {
        right_key
            .cmp(left_key)
            .then_with(|| right.score.total_cmp(&left.score))
            .then_with(|| left.file_path.cmp(&right.file_path))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.column.cmp(&right.column))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.symbol_id.value().cmp(&right.symbol_id.value()))
    });
    rows.into_iter().map(|(row, _)| row.symbol_id).collect()
}

fn diversity_control(pool: &[SearchResult]) -> Vec<SymbolId> {
    // A measured control, not a selected archive/test exclusion policy.
    let mut counts = BTreeMap::new();
    let mut front = Vec::new();
    let mut deferred = Vec::new();
    for row in pool {
        let count = counts.entry(row.file_path.as_str()).or_insert(0);
        if *count < 2 {
            front.push(row.symbol_id);
        } else {
            deferred.push(row.symbol_id);
        }
        *count += 1;
    }
    front.extend(deferred);
    front
}

fn rank(ids: &[SymbolId], owner: SymbolId) -> Option<usize> {
    ids.iter()
        .position(|id| *id == owner)
        .map(|position| position + 1)
}

#[test]
fn compare_bounded_relevance_alternatives_without_relabeling_misses() {
    let (_temp, index, oracle) = fixture();
    let symbols = index.get_all_symbols();
    println!(
        "recovery_symbols={}",
        serde_json::to_string(&symbols).unwrap()
    );
    // Diagnostic graph snapshot only. Per-node exhaustion is recorded, not
    // silently treated as no dependencies. No graph ranking is shipped here.
    for symbol in &symbols {
        let edges = match index.graph_neighbors(symbol.id, RelationKind::Calls, false, Some(32)) {
            Ok(neighbors) => {
                json!({"status":"complete_indexed_neighbors", "targets":neighbors.iter().map(|(target, _)| target.id.value()).collect::<Vec<_>>() })
            }
            Err(error) => {
                json!({"status":"unavailable_or_budget_exceeded", "error":error.to_string()})
            }
        };
        println!(
            "recovery_graph={}",
            json!({"source":symbol.id.value(),"calls":edges})
        );
    }
    let mut measured = 0;
    for task in oracle["tasks"].as_array().unwrap() {
        let name = task["name"].as_str().unwrap();
        let path = task["path"].as_str().unwrap();
        let owners: Vec<_> = symbols
            .iter()
            .filter(|symbol| symbol.name.as_ref() == name && symbol.file_path.as_ref() == path)
            .collect();
        assert_eq!(owners.len(), 1, "owner label changed: {task}");
        let owner = owners[0].id;
        for (family, query) in task["queries"].as_array().unwrap().iter().enumerate() {
            let query = query.as_str().unwrap();
            let baseline = index.search(query, 5, None, None, Some("rust")).unwrap();
            let pool = index.search(query, 200, None, None, Some("rust")).unwrap();
            assert!(pool.len() <= 200);
            let mut variants = BTreeMap::new();
            variants.insert(
                "current",
                rank(
                    &baseline.iter().map(|row| row.symbol_id).collect::<Vec<_>>(),
                    owner,
                ),
            );
            variants.insert("file_diversity_two", rank(&diversity_control(&pool), owner));
            for mode in ["raw", "literal", "literal_paths", "stem", "stem_paths"] {
                variants.insert(mode, rank(&ordered_ids(&pool, query, mode), owner));
            }
            println!(
                "recovery_case={}",
                json!({
                    "task":task["id"], "family":family, "query":query, "owner":owner.value(),
                    "owner_name":name, "owner_path":path, "owner_documented":owners[0].doc_comment.is_some(),
                    "ranks":variants, "pool":pool,
                })
            );
            measured += 1;
        }
    }
    assert_eq!(measured, 20, "all frozen query outcomes must be recorded");
}

#[test]
fn token_controls_distinguish_substrings_without_losing_identifier_parts() {
    assert!(!tokens("redraw", false).contains("raw"));
    assert!(!tokens("before", false).contains("for"));
    assert!(
        tokens("HTTPServer read_file", false).is_superset(&BTreeSet::from([
            "http".into(),
            "server".into(),
            "read".into(),
            "file".into()
        ]))
    );
    assert_eq!(
        tokens("vector vectors", true),
        BTreeSet::from(["vector".into()])
    );
}
