//! Cross-feature process contracts on the combined #62/#65 build.
//! Mock vectors test opt-in, identity and parent behavior, not semantic quality.
use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

const QUERY: &str = "conversation timeout";
const BODY: &str = "/// Conversation timeout dispatch.\npub fn dispatch_ticket() { beta_worker(); }\nfn beta_worker() {}\n";

fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, Option<std::time::SystemTime>)> {
    let mut result = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let bytes = if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
            Vec::new()
        } else {
            std::fs::read(&path).unwrap()
        };
        result.insert(path, (bytes, metadata.modified().ok()));
    }
    result
}

fn ticket(workspace: &Workspace, request: &Value) -> Value {
    let index = workspace.root().join(".codanna/index");
    let before = snapshot(&index);
    let mut command = Command::new(env!("CARGO_BIN_EXE_codanna"));
    command.env_clear().env("HOME", workspace.root().join(".home"));
    for key in ["PATH", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "SYSTEMROOT", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let encoded = request.to_string();
    let output = command.current_dir(workspace.root()).args([
        "--config", ".codanna/settings.toml", "mcp", "search_ticket_context",
        "--args", &encoded, "--json",
    ]).output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(snapshot(&index), before, "ticket query wrote the existing index");
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    envelope["data"].clone()
}

fn configure(workspace: &Workspace, endpoint: &Endpoint, body: bool) {
    let policy = if body { "symbol_body_v1" } else { "doc_comment" };
    workspace.configure(endpoint, "fixture-model", 2, &format!(
        "code_representation = {policy:?}\nmax_input_tokens = 2048\n[documents]\nenabled = false"
    ));
}

fn assert_query_only(endpoint: &Endpoint) {
    let mut inputs = endpoint.take_inputs();
    inputs.sort();
    let mut expected = vec!["probe".to_string(), QUERY.to_string()];
    expected.sort();
    assert_eq!(inputs, expected, "only initialization and one query input are permitted");
}

#[test]
fn retrieval_body_opt_in_preserves_lexical_defaults_and_scope() {
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    workspace.source(BODY);
    configure(&workspace, &endpoint, true);
    workspace.rebuild();
    assert_inputs(&endpoint, 2);

    let direct = ticket(&workspace, &json!({"query": QUERY, "code_limit": 1}));
    assert_eq!(direct["code"]["items"][0]["name"], "dispatch_ticket");
    assert!(direct["code"].get("related_code").is_none());
    assert!(endpoint.take_inputs().is_empty(), "lexical-only request initialized a provider");

    let scoped = ticket(&workspace, &json!({
        "query": QUERY, "code_limit": 1, "include_related_code": true,
        "include_semantic_code": true, "code_path_prefix": "src/lib.rs"
    }));
    assert_eq!(scoped["code"]["items"], direct["code"]["items"]);
    assert_eq!(scoped["code"]["semantic_status"], "not_run_scoped_semantic_unsupported");
    assert_eq!(scoped["code"]["related_code"]["status"], "completed_bounded");
    assert_eq!(scoped["code"]["related_code"]["items"][0]["name"], "beta_worker");
    assert!(endpoint.take_inputs().is_empty(), "unsupported scoped semantics contacted a provider");

    let missing = ticket(&workspace, &json!({
        "query": QUERY, "include_related_code": true, "include_semantic_code": true,
        "code_path_prefix": "src/missing"
    }));
    assert!(missing["code"]["items"].as_array().unwrap().is_empty());
    assert!(missing["code"]["related_code"]["items"].as_array().unwrap().is_empty());
    assert!(endpoint.take_inputs().is_empty());

    let semantic = ticket(&workspace, &json!({
        "query": QUERY, "code_limit": 2, "include_semantic_code": true
    }));
    assert_eq!(semantic["code"]["semantic_status"], "completed_bounded");
    assert!(semantic["code"]["items"].as_array().unwrap().iter().any(|row| {
        row["contributions"].as_array().unwrap().iter().any(|item| item["source"] == "semantic")
    }));
    assert_query_only(&endpoint);
}

#[test]
fn retrieval_body_segments_return_parents_and_failed_load_preserves_lexical_evidence() {
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    workspace.source(&format!(
        "/// Conversation timeout dispatch.\npub fn dispatch_ticket() {{ beta_worker(); }}\nfn beta_worker() {{ let payload = \"{}\"; consume(payload); }}\n", "a".repeat(12000)
    ));
    configure(&workspace, &endpoint, true);
    workspace.rebuild();
    let source_inputs = endpoint.take_inputs();
    assert!(source_inputs.iter().any(|text| text.contains("beta_worker")));
    let semantic_path = workspace.root().join(".codanna/index/semantic");
    let vectors = SimpleSemanticSearch::load_remote(&semantic_path).unwrap();
    assert_eq!(vectors.embedding_count(), 2);
    assert!(vectors.vector_count() > 2, "fixture needs multiple segments per parent");
    drop(vectors);

    let lexical = ticket(&workspace, &json!({"query": QUERY, "code_limit": 2}));
    assert!(endpoint.take_inputs().is_empty());
    let enabled = ticket(&workspace, &json!({
        "query": QUERY, "code_limit": 2, "include_semantic_code": true
    }));
    assert_eq!(enabled["code"]["semantic_status"], "completed_bounded");
    let items = enabled["code"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items.iter().map(|row| row["symbol_id"].as_u64().unwrap()).collect::<HashSet<_>>().len(), 2);
    assert_eq!(items.iter().filter(|row| row["name"] == "beta_worker").count(), 1);
    assert_query_only(&endpoint);

    configure(&workspace, &endpoint, false);
    let mismatch = ticket(&workspace, &json!({
        "query": QUERY, "code_limit": 2, "include_semantic_code": true
    }));
    assert_eq!(mismatch["code"]["semantic_status"], "unavailable");
    assert_eq!(mismatch["code"]["items"], lexical["code"]["items"]);
    assert!(endpoint.take_inputs().is_empty(), "source-policy mismatch must precede provider initialization");

    configure(&workspace, &endpoint, true);
    std::fs::write(semantic_path.join("metadata.json"), "deliberately malformed metadata").unwrap();
    let corrupt = ticket(&workspace, &json!({
        "query": QUERY, "code_limit": 2, "include_semantic_code": true
    }));
    assert_eq!(corrupt["code"]["semantic_status"], "unavailable");
    assert_eq!(corrupt["code"]["items"], lexical["code"]["items"]);
    assert!(endpoint.take_inputs().is_empty(), "corrupt index must not trigger provider initialization or rebuilding");
}
