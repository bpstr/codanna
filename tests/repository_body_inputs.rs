//! Source-input coverage for the unchanged PR #58 task owners, using PR #55's
//! real parser/capture stage. No IndexFacade, model, provider or vector generation.

use codanna::indexing::{calculate_hash, pipeline::{FileContent, stages::parse::{ParseStage, init_parser_cache}}};
use codanna::symbol_representation::{CodeEmbeddingPolicy, MAX_FILE_SOURCE_BYTES, MAX_SYMBOL_SOURCE_BYTES};
use codanna::Settings;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

const ORACLE: &str = include_str!("../contributing/retrieval/evaluations/repository-tasks.json");
const SOURCES: &[(&str, &str)] = &[
    ("src/indexing/pipeline/stages/read.rs", include_str!("fixtures/repository_task_sources/read.rs.fixture")),
    ("src/indexing/pipeline/stages/discover.rs", include_str!("fixtures/repository_task_sources/discover.rs.fixture")),
    ("src/mcp/tools/recall.rs", include_str!("fixtures/repository_task_sources/recall.rs.fixture")),
    ("src/embedding_cache.rs", include_str!("fixtures/repository_task_sources/embedding_cache.rs.fixture")),
    ("src/memory.rs", include_str!("fixtures/repository_task_sources/memory.rs.fixture")),
];

fn words(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() >= 3).map(str::to_lowercase).collect()
}

#[test]
fn repository_body_inputs_capture_real_owners_without_embedding_them() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    assert_eq!(calculate_hash(ORACLE), "129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6");
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings { workspace_root: Some(temp.path().to_owned()), ..Default::default() };
    // Only the parser is constructed. This flag selects capture in that stage;
    // it does not initialize semantic search or any inference backend.
    settings.semantic_search.enabled = true;
    settings.semantic_search.code_representation = CodeEmbeddingPolicy::SymbolBodyV1;
    let settings = Arc::new(settings);
    init_parser_cache(Arc::clone(&settings));
    let parser = ParseStage::new(settings);
    let mut owners_seen = 0;
    let mut documented = 0;
    let mut captured = 0;
    let mut total_owner_bytes = 0;
    for &(relative, source) in SOURCES {
        assert_eq!(calculate_hash(source), oracle["files"][relative]);
        let path = temp.path().join(relative);
        let parsed = parser.parse(FileContent::new(path, source.into(), calculate_hash(source))).unwrap();
        let retained: usize = parsed.raw_symbols.iter().filter_map(|symbol| symbol.embedding_source.as_ref())
            .map(|input| input.header.len() + input.fragments.iter().map(|fragment| fragment.text.len()).sum::<usize>()).sum();
        assert!(retained <= MAX_FILE_SOURCE_BYTES);
        for task in oracle["tasks"].as_array().unwrap().iter().filter(|task| task["path"] == relative) {
            let name = task["name"].as_str().unwrap();
            let matches: Vec<_> = parsed.raw_symbols.iter().filter(|symbol| symbol.name.as_ref() == name).collect();
            assert_eq!(matches.len(), 1, "owner must be unambiguous: {task}");
            let symbol = matches[0];
            owners_seen += 1;
            documented += usize::from(symbol.doc_comment.is_some());
            let input = symbol.embedding_source.as_ref().expect("eligible owner source capture");
            assert!(!input.fragments.is_empty(), "no parser-range source for {name}");
            assert!(input.header.contains(relative));
            let mut body = input.header.clone();
            for fragment in &input.fragments {
                assert_eq!(source.get(fragment.range.clone()), Some(fragment.text.as_str()), "wrong source provenance: {name}");
                body.push_str(&fragment.text);
            }
            let retained_body: usize = input.fragments.iter().map(|fragment| fragment.text.len()).sum();
            assert!(retained_body <= MAX_SYMBOL_SOURCE_BYTES);
            total_owner_bytes += body.len();
            captured += 1;
            for (variant, query) in task["queries"].as_array().unwrap().iter().enumerate() {
                let query = words(query.as_str().unwrap());
                let old = words(symbol.doc_comment.as_deref().unwrap_or(""));
                let new = words(&body);
                println!("repository_body_input={}", json!({
                    "task":task["id"], "variant":variant, "owner":name,
                    "legacy_eligible":symbol.doc_comment.is_some(), "body_eligible":true,
                    "retained_utf8_bytes":body.len(), "fragments":input.fragments.len(),
                    "query_word_count":query.len(), "comment_word_overlap":query.intersection(&old).count(),
                    "representation_word_overlap":query.intersection(&new).count(),
                    "source_excerpts_verified":true, "vector_generated":false, "quality_evaluated":false,
                }));
            }
        }
    }
    assert_eq!(owners_seen, 10);
    assert_eq!(documented, 6);
    assert_eq!(captured, 10);
    println!("repository_body_summary={}", json!({"owners":owners_seen, "legacy_eligible":documented,
        "body_captured":captured, "retained_owner_utf8_bytes":total_owner_bytes, "provider_requests":0,
        "billed_tokens":null, "retrieval_quality":null}));
}

#[test]
fn repository_body_inputs_keep_legacy_policy_opt_in() {
    let mut settings = Settings::default();
    settings.semantic_search.enabled = true;
    assert_eq!(settings.semantic_search.code_representation, CodeEmbeddingPolicy::DocComment);
    let settings = Arc::new(settings);
    init_parser_cache(Arc::clone(&settings));
    let parser = ParseStage::new(settings);
    let parsed = parser.parse(FileContent::new("src/recall.rs".into(), SOURCES[2].1.into(), calculate_hash(SOURCES[2].1))).unwrap();
    assert!(parsed.raw_symbols.iter().all(|symbol| symbol.embedding_source.is_none()));
}
