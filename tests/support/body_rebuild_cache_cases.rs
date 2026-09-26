//! Uses the parent CLI fixture's joined loopback server and disposable environment.
//! These tests measure input reuse and persistence, never semantic relevance.
use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

const BODY_SOURCE: &str =
    "/// alpha owner.\npub fn alpha_owner() -> u8 { helper() }\npub fn helper() -> u8 { 1 }\n";

fn tree(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, Option<std::time::SystemTime>)> {
    let mut result = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            result.insert(path.clone(), (Vec::new(), metadata.modified().ok()));
            for entry in std::fs::read_dir(path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else {
            result.insert(
                path.clone(),
                (std::fs::read(path).unwrap(), metadata.modified().ok()),
            );
        }
    }
    result
}

fn plan(workspace: &Workspace, endpoint: &Endpoint) -> Value {
    let before = tree(workspace.root());
    let request_count = endpoint.inputs.lock().unwrap().len();
    let mut command = Command::new(env!("CARGO_BIN_EXE_codanna-index-plan"));
    command
        .env_clear()
        .env("HOME", workspace.root().join(".home"));
    for name in [
        "PATH",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "SYSTEMROOT",
        "WINDIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = command
        .current_dir(workspace.root())
        .args(["--config", ".codanna/settings.toml"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        tree(workspace.root()),
        before,
        "planner changed the workspace"
    );
    assert_eq!(
        endpoint.inputs.lock().unwrap().len(),
        request_count,
        "planner contacted endpoint"
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["provider_requests_made"], 0);
    assert!(report["exact_provider_tokens"].is_null());
    report
}

fn sent(endpoint: &Endpoint, expected: usize) -> Vec<String> {
    let inputs = endpoint.take_inputs();
    let probes = inputs
        .iter()
        .filter(|input| input.as_str() == "probe")
        .count();
    let documents: Vec<_> = inputs
        .into_iter()
        .filter(|input| input != "probe")
        .collect();
    println!(
        "body_reuse: source_inputs={} probes={probes} expected={expected}",
        documents.len()
    );
    assert_eq!(
        probes, 1,
        "each rebuild still initializes its remote backend"
    );
    assert_eq!(documents.len(), expected);
    documents
}

fn current_ids(workspace: &Workspace, names: &[&str]) -> HashSet<SymbolId> {
    let index_path = workspace.root().join(".codanna/index");
    let mut settings = Settings {
        workspace_root: Some(workspace.root().to_path_buf()),
        index_path: index_path.clone(),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let facade = IndexPersistence::new(index_path.clone())
        .load_facade_lite(Arc::new(settings))
        .unwrap();
    let ids: HashSet<_> = names
        .iter()
        .map(|name| {
            let found = facade.find_symbols_by_name(name, None);
            assert_eq!(found.len(), 1, "{name}");
            found[0].id
        })
        .collect();
    let vectors = SimpleSemanticSearch::load_remote(&index_path.join("semantic")).unwrap();
    assert_eq!(vectors.embedding_count(), ids.len());
    let hits = vectors
        .search_with_embedding_and_language(&[1.0, 0.0], 100, None)
        .unwrap();
    assert_eq!(hits.iter().map(|(id, _)| *id).collect::<HashSet<_>>(), ids);
    ids
}

#[test]
fn body_cache_planner_matches_cold_warm_edit_and_deleted_parent() {
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    workspace.source(BODY_SOURCE);
    workspace.configure(
        &endpoint,
        "fixture-model",
        2,
        "code_representation = \"symbol_body_v1\"",
    );
    let cold = plan(&workspace, &endpoint);
    assert_eq!(cold["embedding_candidates"], 2);
    assert_eq!(cold["embedding_inputs"], 2);
    assert_eq!(cold["snapshot_miss_inputs"], 2);
    workspace.rebuild();
    let inputs = sent(&endpoint, 2);
    assert!(
        inputs
            .iter()
            .all(|input| !input.contains(workspace.root().to_str().unwrap())),
        "body inputs must not contain the machine-specific workspace root"
    );
    let before_ids = current_ids(&workspace, &["alpha_owner", "helper"]);

    workspace.source(&format!("pub const SHIFT_ID: u8 = 0;\n{BODY_SOURCE}"));
    let warm = plan(&workspace, &endpoint);
    assert_eq!(warm["snapshot_hit_inputs"], 2);
    workspace.rebuild();
    sent(&endpoint, 0);
    assert_ne!(
        before_ids,
        current_ids(&workspace, &["alpha_owner", "helper"])
    );

    workspace.source(&BODY_SOURCE.replace("{ 1 }", "{ 2 }"));
    let edited = plan(&workspace, &endpoint);
    assert_eq!(edited["snapshot_miss_inputs"], 1);
    workspace.rebuild();
    let changed = sent(&endpoint, 1);
    assert!(changed[0].contains("helper"));
    current_ids(&workspace, &["alpha_owner", "helper"]);

    workspace.source("pub fn helper() -> u8 { 2 }\n");
    assert_eq!(plan(&workspace, &endpoint)["snapshot_miss_inputs"], 0);
    workspace.rebuild();
    sent(&endpoint, 0);
    current_ids(&workspace, &["helper"]);
}

#[test]
fn body_cache_policy_switch_and_corruption_never_reuse_legacy_vectors() {
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    workspace.source(BODY_SOURCE);
    workspace.rebuild();
    sent(&endpoint, 1);
    workspace.configure(
        &endpoint,
        "fixture-model",
        2,
        "code_representation = \"symbol_body_v1\"",
    );
    assert_eq!(plan(&workspace, &endpoint)["snapshot_hit_inputs"], 0);
    workspace.rebuild();
    sent(&endpoint, 2);
    workspace.configure(&endpoint, "fixture-model", 2, "");
    assert_eq!(plan(&workspace, &endpoint)["snapshot_hit_inputs"], 0);
    workspace.rebuild();
    sent(&endpoint, 1);
    workspace.configure(
        &endpoint,
        "fixture-model",
        2,
        "code_representation = \"symbol_body_v1\"",
    );
    std::fs::write(
        workspace
            .root()
            .join(".codanna/index/semantic/embedding-cache.json"),
        "corrupt",
    )
    .unwrap();
    assert_eq!(plan(&workspace, &endpoint)["snapshot_hit_inputs"], 0);
    workspace.rebuild();
    sent(&endpoint, 2);
}

#[test]
fn body_cache_segmentation_matches_planner_and_survives_reopen() {
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    // Homogeneous literal bytes guarantee equal interior segments regardless of
    // the header's length or split boundary alignment. They keep distinct ranges.
    workspace.source(&format!(
        "pub fn large_owner() {{ let payload = \"{}\"; consume(payload); }}\n",
        "a".repeat(12000)
    ));
    workspace.configure(
        &endpoint,
        "fixture-model",
        2,
        "code_representation = \"symbol_body_v1\"\nmax_input_tokens = 2048",
    );
    let before = plan(&workspace, &endpoint);
    let inputs = before["embedding_inputs"].as_u64().unwrap() as usize;
    let unique = before["unique_embedding_inputs"].as_u64().unwrap() as usize;
    assert!((2..=8).contains(&inputs));
    assert!(
        unique < inputs,
        "fixture must contain identical prepared segments with different source ranges"
    );
    workspace.rebuild();
    sent(&endpoint, unique);
    let vectors =
        SimpleSemanticSearch::load_remote(&workspace.root().join(".codanna/index/semantic"))
            .unwrap();
    assert_eq!(vectors.embedding_count(), 1);
    assert_eq!(vectors.vector_count(), inputs);
    println!("body_segments: parents=1 stored_segments={inputs} unique_inference_inputs={unique}");
    drop(vectors);
    assert_eq!(plan(&workspace, &endpoint)["snapshot_miss_inputs"], 0);
    workspace.rebuild();
    sent(&endpoint, 0);
    current_ids(&workspace, &["large_owner"]);
}

#[test]
fn body_cache_pressure_preserves_late_hits_and_continues_learning() {
    // Use the semantic journal's supported maximum dimension, not the larger
    // standalone accelerator limit. 16 MiB / (4096*4 + 256) = 1008 entries.
    const COUNT: usize = 1280;
    let endpoint = Endpoint::start(4096);
    let workspace = Workspace::new(&endpoint);
    let source = (0..COUNT)
        .map(|i| format!("pub fn pressure_{i:04}() -> usize {{ {i} }}\n"))
        .collect::<String>();
    workspace.source(&source);
    workspace.configure(
        &endpoint,
        "fixture-model",
        4096,
        "code_representation = \"symbol_body_v1\"",
    );
    workspace.rebuild();
    sent(&endpoint, COUNT);
    for round in 1..=2 {
        let predicted = plan(&workspace, &endpoint);
        assert_eq!(predicted["embedding_candidates"], COUNT);
        assert_eq!(predicted["snapshot_hit_inputs"], 1008);
        assert_eq!(predicted["snapshot_miss_inputs"], 272);
        workspace.rebuild();
        sent(&endpoint, 272);
        let semantic =
            SimpleSemanticSearch::load_remote(&workspace.root().join(".codanna/index/semantic"))
                .unwrap();
        assert_eq!(semantic.embedding_count(), COUNT);
        assert_eq!(semantic.vector_count(), COUNT);
        println!("body_pressure_round={round} parents={COUNT} snapshot_hits=1008 generated=272");
    }
}
