//! Code-journal dimension boundaries through real CLI and persistence calls.
//! Joined loopback transport only; no provider credentials or production stores.
use codanna::indexing::facade::build_embedding_backend;
use codanna::semantic::SimpleSemanticSearch;
use codanna::{Settings, SymbolId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

struct Endpoint {
    url: String,
    inputs: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Endpoint {
    fn new(dimension: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&inputs);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => respond(stream, dimension, &recorded),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("loopback accept: {error}"),
                }
            }
        });
        Self { url, inputs, stop, thread: Some(thread) }
    }
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.inputs.lock().unwrap())
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}
fn respond(mut stream: TcpStream, dimension: usize, recorded: &Mutex<Vec<String>>) {
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut bytes = Vec::new();
    let (start, length) = loop {
        let mut part = [0; 4096];
        let count = stream.read(&mut part).unwrap();
        assert!(count > 0, "incomplete request");
        bytes.extend_from_slice(&part[..count]);
        assert!(bytes.len() < 1_000_000);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            assert!(header.starts_with("POST /v1/embeddings "));
            assert!(!header.to_ascii_lowercase().contains("authorization:"));
            let length = header.lines().find_map(|line| {
                line.to_ascii_lowercase().strip_prefix("content-length:")
                    .map(|value| value.trim().parse::<usize>().unwrap())
            }).unwrap();
            if bytes.len() >= end + 4 + length { break (end + 4, length); }
        }
    };
    let value: Value = serde_json::from_slice(&bytes[start..start + length]).unwrap();
    let inputs: Vec<_> = value["input"].as_array().unwrap().iter()
        .map(|input| input.as_str().unwrap().to_owned()).collect();
    let mut vector = vec![0.0_f32; dimension];
    vector[0] = 1.0;
    let data: Vec<_> = inputs.iter().enumerate()
        .map(|(index, _)| json!({"index":index,"embedding":vector})).collect();
    recorded.lock().unwrap().extend(inputs);
    let body = json!({"data": data}).to_string();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}

type Tree = BTreeMap<PathBuf, (Option<Vec<u8>>, Option<SystemTime>)>;
fn tree(root: &Path) -> Tree {
    let mut result = BTreeMap::new();
    if !root.exists() { return result; }
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let bytes = if metadata.is_dir() {
            pending.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            None
        } else { Some(std::fs::read(&path).unwrap()) };
        result.insert(path.strip_prefix(root).unwrap().to_path_buf(), (bytes, metadata.modified().ok()));
    }
    result
}
struct Workspace(tempfile::TempDir);
impl Workspace {
    fn new() -> Self {
        let value = Self(tempfile::tempdir().unwrap());
        for dir in ["src", ".home", ".codanna"] {
            std::fs::create_dir_all(value.root().join(dir)).unwrap();
        }
        std::fs::write(value.root().join("src/lib.rs"), "/// fixture documented owner.\npub fn owner() { helper(); }\n/// fixture documented helper.\npub fn helper() {}\n").unwrap();
        value
    }
    fn root(&self) -> &Path { self.0.path() }
    fn index(&self) -> PathBuf { self.root().join(".codanna/index") }
    fn configure(&self, endpoint: &Endpoint, dimension: Option<usize>, policy: &str, enabled: bool) {
        let dimension = dimension.map(|v| format!("remote_dim = {v}\n")).unwrap_or_default();
        std::fs::write(self.root().join(".codanna/settings.toml"), format!(
            "index_path = \".codanna/index\"\n[indexing]\nindexed_paths = [\"src\"]\n[documents]\nenabled = false\n[semantic_search]\nenabled = {enabled}\ncode_representation = {policy:?}\nremote_url = {:?}\nremote_model = \"dimension-fixture\"\n{dimension}", endpoint.url
        )).unwrap();
    }
    fn run(&self, planner: bool, dimension_override: Option<&str>) -> Output {
        let binary = if planner { env!("CARGO_BIN_EXE_codanna-index-plan") } else { env!("CARGO_BIN_EXE_codanna") };
        let mut command = Command::new(binary);
        command.env_clear().env("HOME", self.root().join(".home"));
        for name in ["PATH", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "SYSTEMROOT", "WINDIR"] {
            if let Some(value) = std::env::var_os(name) { command.env(name, value); }
        }
        if let Some(value) = dimension_override { command.env("CODANNA_EMBED_DIM", value); }
        command.current_dir(self.root()).args(["--config", ".codanna/settings.toml"]);
        if !planner { command.args(["index", "src", "--force", "--no-progress"]); }
        command.output().unwrap()
    }
    fn seed(&self, endpoint: &Endpoint, policy: &str) {
        self.configure(endpoint, Some(2), policy, true);
        let output = self.run(false, None);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let inputs = endpoint.take();
        assert_eq!(inputs.iter().filter(|v| v.as_str() == "probe").count(), 1);
        assert!(inputs.len() > 1);
    }
}
fn rejected(output: &Output) {
    assert!(!output.status.success(), "unsupported code dimension was accepted");
    let diagnostic = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(diagnostic.contains("dimension") && diagnostic.contains("4096"), "{diagnostic}");
}

#[test]
fn configured_dimensions_fail_before_probe_or_destructive_force_clear() {
    let endpoint = Endpoint::new(2);
    for policy in ["doc_comment", "symbol_body_v1"] {
        let workspace = Workspace::new();
        workspace.seed(&endpoint, policy);
        for dimension in [0, 4097, 16384] {
            workspace.configure(&endpoint, Some(dimension), policy, true);
            let before = tree(&workspace.index());
            let output = workspace.run(false, None);
            rejected(&output);
            assert!(endpoint.take().is_empty(), "known invalid dimension caused a request");
            assert_eq!(tree(&workspace.index()), before, "force mutated an existing index");
        }
    }
}

#[test]
fn unknown_oversized_probe_never_embeds_source_or_clears_the_old_index() {
    let valid = Endpoint::new(2);
    let oversized = Endpoint::new(4097);
    for policy in ["doc_comment", "symbol_body_v1"] {
        let workspace = Workspace::new();
        workspace.seed(&valid, policy);
        workspace.configure(&oversized, None, policy, true);
        let before = tree(&workspace.index());
        rejected(&workspace.run(false, None));
        assert_eq!(oversized.take(), vec!["probe"], "source inference followed an unsupported probe");
        assert_eq!(tree(&workspace.index()), before);
    }
}

#[test]
fn environment_dimension_precedence_is_validated_before_source_work() {
    let endpoint = Endpoint::new(2);
    let workspace = Workspace::new();
    workspace.seed(&endpoint, "symbol_body_v1");
    let before = tree(&workspace.index());
    rejected(&workspace.run(false, Some("4097")));
    assert!(endpoint.take().is_empty());
    assert_eq!(tree(&workspace.index()), before);
    workspace.configure(&endpoint, Some(4097), "symbol_body_v1", true);
    let output = workspace.run(false, Some("2"));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(endpoint.take(), vec!["probe"], "valid override lost compatible cache reuse");
}

#[test]
fn planner_and_empty_workspace_reject_without_side_effects_but_disabled_mode_stays_off() {
    let endpoint = Endpoint::new(2);
    for policy in ["doc_comment", "symbol_body_v1"] {
        let workspace = Workspace::new();
        workspace.configure(&endpoint, Some(4097), policy, true);
        let before = tree(workspace.root());
        rejected(&workspace.run(true, None));
        assert_eq!(tree(workspace.root()), before);
        rejected(&workspace.run(false, None));
        assert!(!workspace.index().exists());
        assert!(endpoint.take().is_empty());
        workspace.configure(&endpoint, Some(4097), policy, false);
        let report = workspace.run(true, None);
        let data: Value = serde_json::from_slice(&report.stdout).unwrap();
        assert_eq!(data["backend"], "disabled");
        assert_eq!(data["provider_requests_made"], 0);
        let output = workspace.run(false, None);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(endpoint.take().is_empty());
        assert!(!workspace.index().join("semantic/metadata.json").exists());
    }
}

#[test]
fn unsupported_first_save_and_replacement_preserve_all_persisted_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let absent = temp.path().join("absent");
    let existing = temp.path().join("existing");
    let valid = SimpleSemanticSearch::new_empty(2, "fixture");
    valid.save(&existing).unwrap();
    let before = tree(&existing);
    for dimension in [0, 4097, 16384] {
        let invalid = SimpleSemanticSearch::new_empty(dimension, "fixture");
        assert!(invalid.save(&absent).is_err());
        assert!(!absent.exists(), "invalid first save created artifacts");
        assert!(invalid.save(&existing).is_err());
        assert_eq!(tree(&existing), before, "invalid replacement changed a valid generation");
    }
    let metadata = existing.join("metadata.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
    value["dimension"] = json!(4097);
    std::fs::write(&metadata, value.to_string()).unwrap();
    let corrupt_before = tree(&existing);
    assert!(SimpleSemanticSearch::load_without_model(&existing).is_err());
    assert!(SimpleSemanticSearch::new_empty(2, "replacement").save(&existing).is_err());
    assert_eq!(tree(&existing), corrupt_before, "existing invalid store was rewritten");
}

#[test]
fn supported_code_endpoints_survive_first_save_delta_and_reopen() {
    for dimension in [1, 4096] {
        let temp = tempfile::tempdir().unwrap();
        let mut search = SimpleSemanticSearch::new_empty(dimension, "fixture");
        let mut vector = vec![0.0; dimension];
        vector[0] = 1.0;
        for id in [1, 2] {
            search.store_embeddings(vec![(SymbolId::new(id).unwrap(), vector.clone(), "rust".into())]);
            search.save(temp.path()).unwrap();
            let reopened = SimpleSemanticSearch::load_without_model(temp.path()).unwrap();
            assert_eq!(reopened.dimensions(), dimension);
            assert_eq!(reopened.embedding_count(), id as usize);
        }
    }
}

#[test]
fn shared_document_backend_is_not_restricted_to_the_code_journal_limit() {
    let endpoint = Endpoint::new(4097);
    let mut settings = Settings::default();
    settings.semantic_search.remote_url = Some(endpoint.url.clone());
    settings.semantic_search.remote_dim = Some(4097);
    settings.semantic_search.remote_model = Some("dimension-fixture".into());
    let backend = build_embedding_backend(&settings.semantic_search).unwrap();
    assert_eq!(backend.dimensions(), 4097);
    assert_eq!(backend.embed_one("synthetic document input").unwrap().len(), 4097);
    assert_eq!(endpoint.take(), vec!["probe", "synthetic document input"]);
}
