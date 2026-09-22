//! Planner parity against the production parser, representation and cache contracts.
use super::*;
use crate::symbol_representation::{MAX_FILE_SOURCE_BYTES, MAX_SYMBOL_SEGMENTS};
use serde_json::json;
use std::net::TcpListener;

fn fixture(source: &str, max_input: usize) -> (tempfile::TempDir, Settings, TcpListener) {
    let temp = tempfile::tempdir().unwrap();
    let endpoint = TcpListener::bind("127.0.0.1:0").unwrap();
    endpoint.set_nonblocking(true).unwrap();
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), source).unwrap();
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: root.join(".codanna/index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = true;
    settings.semantic_search.code_representation = CodeEmbeddingPolicy::SymbolBodyV1;
    settings.semantic_search.remote_url = Some(format!("http://{}", endpoint.local_addr().unwrap()));
    settings.semantic_search.remote_model = Some("offline-body-fixture".into());
    settings.semantic_search.remote_dim = Some(2);
    settings.semantic_search.max_input_tokens = Some(max_input);
    settings.add_indexed_path(root.join("src")).unwrap();
    (temp, settings, endpoint)
}

// No planner counters are used to derive these inputs. This is the parser/source
// and segmentation path used by runtime COLLECT/EMBED, without constructing EMBED.
fn runtime_inputs(settings: &Settings) -> (Vec<String>, usize) {
    let options = Arc::new(settings.clone());
    init_parser_cache(Arc::clone(&options));
    let parser = ParseStage::new(Arc::clone(&options));
    let budget = InputBudget::remote(settings.semantic_search.max_input_tokens, None).unwrap();
    let mut inputs = Vec::new();
    let mut retained = 0;
    for file in FileWalker::new(options).snapshot(
        &settings.indexed_paths_cache, MAX_ENTRIES, MAX_FILES, MAX_SOURCE_BYTES,
    ).unwrap() {
        let bytes = file.content.clone();
        for symbol in parser.parse(file).unwrap().raw_symbols {
            if let Some(source) = symbol.embedding_source {
                retained += source.retained_bytes();
                for fragment in &source.fragments {
                    assert_eq!(bytes.get(fragment.range.clone()), Some(fragment.text.as_str()));
                }
                let segments = source.inputs(&budget).unwrap();
                assert!(segments.len() <= MAX_SYMBOL_SEGMENTS);
                for segment in segments {
                    budget.validate([segment.text.as_str()]).unwrap();
                    inputs.push(segment.text);
                }
            }
        }
    }
    (inputs, retained)
}

fn identity(settings: &Settings, policy: CodeEmbeddingPolicy) -> String {
    let budget = InputBudget::remote(settings.semantic_search.max_input_tokens, None).unwrap();
    let endpoint = calculate_hash(settings.semantic_search.remote_url.as_deref().unwrap());
    policy.bind_identity(backend_identity("remote", "offline-body-fixture", Some(&endpoint), None, &budget))
}

fn seed_cache(settings: &Settings, inputs: &[String], policy: CodeEmbeddingPolicy) -> PathBuf {
    let mut cache = EmbeddingCache::empty(identity(settings, policy), 2);
    for input in inputs {
        cache.insert(input, Arc::from(vec![1.0_f32, 0.0]));
    }
    let path = settings.index_path.join("semantic/embedding-cache.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    cache.save(&path).unwrap();
    path
}

fn no_request(endpoint: &TcpListener) {
    assert!(matches!(endpoint.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
}

#[test]
fn body_plan_matches_runtime_segments_bytes_and_cache_after_source_edits() {
    let source = format!("pub fn owner() {{ {} }}\n", "work(); ".repeat(650));
    let (_temp, settings, endpoint) = fixture(&source, 1024);
    let (inputs, retained) = runtime_inputs(&settings);
    assert!(inputs.len() > 1);
    let unique: HashSet<_> = inputs.iter().map(|input| calculate_hash(input)).collect();
    let path = seed_cache(&settings, &inputs, CodeEmbeddingPolicy::SymbolBodyV1);
    let cache_before = std::fs::read(&path).unwrap();
    let report = inspect(settings.clone(), &[]).unwrap();
    assert_eq!(report.status, "complete");
    assert_eq!(report.embedding_inputs, Some(inputs.len()));
    assert_eq!(report.embedding_input_bytes, Some(inputs.iter().map(String::len).sum()));
    assert_eq!(report.unique_embedding_inputs, Some(unique.len()));
    assert_eq!(report.retained_representation_bytes, retained);
    assert_eq!(report.snapshot_hit_inputs, Some(inputs.len()));
    assert_eq!(report.snapshot_miss_inputs, Some(0));
    assert_eq!(std::fs::read(&path).unwrap(), cache_before);
    let root = settings.workspace_root.as_ref().unwrap();
    std::fs::write(root.join("src/lib.rs"), source.replace("work()", "changed_work()")).unwrap();
    let changed = inspect(settings, &[]).unwrap();
    assert_ne!(changed.source_fingerprint, report.source_fingerprint);
    assert!(changed.snapshot_miss_inputs.unwrap() > 0);
    assert_eq!(std::fs::read(path).unwrap(), cache_before);
    no_request(&endpoint);
}

#[test]
fn body_plan_requires_source_identity_even_when_cached_text_matches() {
    let (_temp, settings, endpoint) = fixture("pub fn owner() {}\n", 8192);
    let (inputs, _) = runtime_inputs(&settings);
    seed_cache(&settings, &inputs, CodeEmbeddingPolicy::DocComment);
    let wrong = inspect(settings.clone(), &[]).unwrap();
    assert_eq!(wrong.snapshot_hit_inputs, Some(0));
    assert_eq!(wrong.snapshot_miss_inputs, Some(inputs.len()));
    seed_cache(&settings, &inputs, CodeEmbeddingPolicy::SymbolBodyV1);
    let valid = inspect(settings, &[]).unwrap();
    assert_eq!(valid.snapshot_hit_inputs, Some(inputs.len()));
    assert_eq!(valid.snapshot_miss_inputs, Some(0));
    no_request(&endpoint);
}

#[test]
fn body_plan_unknown_dimension_and_disabled_capture_do_not_invent_reuse() {
    let (_temp, mut settings, endpoint) = fixture("pub fn owner() {}\n", 8192);
    settings.semantic_search.remote_dim = None;
    let unknown_dimension = inspect(settings.clone(), &[]).unwrap();
    assert_eq!(unknown_dimension.status, "complete");
    assert_eq!(unknown_dimension.embedding_inputs, Some(1));
    assert_eq!(unknown_dimension.snapshot_hit_inputs, None);
    settings.semantic_search.enabled = false;
    let disabled = inspect(settings.clone(), &[]).unwrap();
    assert_eq!(disabled.status, "partial");
    assert_eq!(disabled.backend, "disabled");
    assert_eq!(disabled.body_sources, 1);
    assert_eq!(disabled.embedding_inputs, None);
    assert_eq!(disabled.embedding_input_bytes, None);
    assert!(!settings.semantic_search.enabled);
    assert!(!settings.index_path.exists());
    no_request(&endpoint);
}

#[test]
fn body_plan_file_budget_exhaustion_remains_visible() {
    let source = (0..6000).map(|i| format!("pub fn owner_{i}() {{}}\n")).collect::<String>();
    let (_temp, settings, endpoint) = fixture(&source, 8192);
    let report = inspect(settings, &[]).unwrap();
    assert_eq!(report.eligible_symbols, 6000);
    assert!(report.missing_body_sources > 0);
    assert_eq!(report.body_sources + report.missing_body_sources, report.eligible_symbols);
    assert!(report.retained_representation_bytes <= MAX_FILE_SOURCE_BYTES);
    assert_eq!(report.status, "partial");
    assert!(!report.input_inventory_complete);
    no_request(&endpoint);
}

#[test]
fn body_plan_frozen_repository_inputs_match_the_runtime_preparation() {
    const ORACLE: &str = include_str!("../contributing/retrieval/evaluations/repository-tasks.json");
    let oracle: serde_json::Value = serde_json::from_str(ORACLE).unwrap();
    assert_eq!(calculate_hash(ORACLE), "129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6");
    let (_temp, settings, endpoint) = fixture("", 2048);
    let root = settings.workspace_root.as_ref().unwrap();
    std::fs::remove_file(root.join("src/lib.rs")).unwrap();
    for (path, source) in [
        ("src/indexing/pipeline/stages/read.rs", include_str!("../tests/fixtures/repository_task_sources/read.rs.fixture")),
        ("src/indexing/pipeline/stages/discover.rs", include_str!("../tests/fixtures/repository_task_sources/discover.rs.fixture")),
        ("src/mcp/tools/recall.rs", include_str!("../tests/fixtures/repository_task_sources/recall.rs.fixture")),
        ("src/embedding_cache.rs", include_str!("../tests/fixtures/repository_task_sources/embedding_cache.rs.fixture")),
        ("src/memory.rs", include_str!("../tests/fixtures/repository_task_sources/memory.rs.fixture")),
    ] {
        assert_eq!(oracle["files"][path], calculate_hash(source));
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, source).unwrap();
    }
    let (inputs, retained) = runtime_inputs(&settings);
    let body = inspect(settings.clone(), &[]).unwrap();
    assert_eq!(body.files_parsed, 5);
    assert!(body.body_sources >= 10);
    assert_eq!(body.embedding_inputs, Some(inputs.len()));
    assert_eq!(body.embedding_input_bytes, Some(inputs.iter().map(String::len).sum()));
    assert_eq!(body.retained_representation_bytes, retained);
    let mut legacy_settings = settings;
    legacy_settings.semantic_search.code_representation = CodeEmbeddingPolicy::DocComment;
    let legacy = inspect(legacy_settings, &[]).unwrap();
    println!("body_plan_frozen_summary={}", json!({
        "source_files": body.files_parsed, "symbols": body.symbols,
        "comment_parents": legacy.embedding_candidates,
        "comment_inputs": legacy.embedding_inputs, "comment_input_bytes": legacy.embedding_input_bytes,
        "body_parents": body.embedding_candidates, "body_inputs": body.embedding_inputs,
        "body_unique_inputs": body.unique_embedding_inputs,
        "body_retained_bytes": retained, "body_segment_input_bytes": body.embedding_input_bytes,
        "body_header_only": body.body_sources_header_only, "body_missing": body.missing_body_sources,
        "body_rejected": body.input_policy_rejections,
        "input_budget_proxy_bytes": 2048, "provider_requests": 0, "provider_tokens": null,
        "oracle_sha256": calculate_hash(ORACLE), "semantic_relevance_measured": false
    }));
    no_request(&endpoint);
}
