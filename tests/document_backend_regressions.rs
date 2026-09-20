//! Configured document backends and query surfaces, with local-only transports.
use codanna::documents::{ChunkingConfig, CollectionConfig, SearchQuery, open_from_settings};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct LocalEmbeddingServer {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LocalEmbeddingServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded = requests.clone();
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("local fixture accept failed: {error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let (header_end, content_length) = loop {
                    let mut chunk = [0_u8; 4096];
                    let read = stream.read(&mut chunk).unwrap();
                    assert!(read > 0, "incomplete local embedding request");
                    bytes.extend_from_slice(&chunk[..read]);
                    assert!(bytes.len() < 1_000_000, "unexpected fixture request size");
                    if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        let length = String::from_utf8_lossy(&bytes[..end])
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break (end + 4, length);
                        }
                    }
                };
                let request: Value =
                    serde_json::from_slice(&bytes[header_end..header_end + content_length])
                        .unwrap();
                let data: Vec<_> = request["input"].as_array().unwrap().iter().enumerate()
                    .map(|(index, text)| json!({"index": index, "embedding":
                        if text.as_str().unwrap().contains("alpha") { vec![1.0, 0.0] } else { vec![0.0, 1.0] }
                    })).collect();
                recorded.lock().unwrap().push(request);
                let body = json!({"data": data}).to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for LocalEmbeddingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn settings(root: &std::path::Path) -> codanna::Settings {
    let mut settings = codanna::Settings {
        index_path: root.join("index"),
        workspace_root: Some(root.to_path_buf()),
        ..Default::default()
    };
    settings.documents.enabled = true;
    settings.semantic_search.model = "deliberately-invalid-local-model".into();
    settings
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        ..Default::default()
    }
}

#[test]
fn disabled_document_embeddings_never_initialize_configured_backend() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path());
    settings.semantic_search.enabled = false;
    settings.semantic_search.remote_url = Some("not-a-valid-provider-url".into());
    let mut store = open_from_settings(&settings).unwrap();
    assert!(store.search(query("alpha")).unwrap().is_empty());
    drop(store);
    assert!(codanna::documents::load_from_settings(&settings).is_some());
}

#[test]
fn configured_remote_documents_use_model_dimension_and_heading_input_consistently() {
    let server = LocalEmbeddingServer::start();
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path());
    settings.semantic_search.enabled = true;
    settings.semantic_search.remote_url = Some(server.url.clone());
    settings.semantic_search.remote_model = Some("fixture@revision-1".into());
    settings.semantic_search.remote_dim = Some(2);
    let path = temp.path().join("guide.md");
    std::fs::write(&path, "# alpha\n\npolicy details").unwrap();
    let mut store = open_from_settings(&settings).unwrap();
    store
        .index_collection(
            "docs",
            &CollectionConfig {
                paths: vec![path],
                ..Default::default()
            },
            &ChunkingConfig {
                min_chunk_chars: 1,
                max_chunk_chars: 100,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    let hits = store.search(query("alpha")).unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit.content_preview.contains("policy details") && hit.similarity > 0.99)
    );
    drop(store);
    let mut reopened = open_from_settings(&settings).unwrap();
    assert!(!reopened.search(query("alpha")).unwrap().is_empty());
    let requests = server.requests.lock().unwrap();
    assert!(
        requests.len() >= 5,
        "probe, index and query use the configured local transport"
    );
    assert!(
        requests
            .iter()
            .all(|request| request["model"] == "fixture@revision-1")
    );
    assert!(
        requests
            .iter()
            .flat_map(|request| request["input"].as_array().unwrap())
            .any(|text| text.as_str().unwrap().contains("alpha\n\npolicy details"))
    );
    let state = std::fs::read_to_string(settings.index_path.join("documents/state.json")).unwrap();
    assert!(!state.contains(&server.url));
}

#[test]
fn one_shot_document_search_reads_index_without_reembedding_changed_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut settings = settings(&root);
    settings.index_path = root.join(".codanna/index");
    settings.semantic_search.enabled = false;
    let src = root.join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("lib.rs"), "pub fn fixture() {}\n").unwrap();
    settings.indexing.indexed_paths = vec![src.clone()];
    let path = root.join("guide.md");
    std::fs::write(&path, "alpha_snapshot\n").unwrap();
    settings.documents.collections.insert(
        "docs".into(),
        CollectionConfig {
            paths: vec![path.clone()],
            ..Default::default()
        },
    );
    std::fs::create_dir(root.join(".codanna")).unwrap();
    std::fs::write(
        root.join(".codanna/settings.toml"),
        toml::to_string(&settings).unwrap(),
    )
    .unwrap();
    let run = |args: &[&str]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_codanna"))
            .args(args)
            .current_dir(&root)
            .env_remove("CODANNA_EMBED_URL")
            .env_remove("CODANNA_EMBED_MODEL")
            .env_remove("CODANNA_EMBED_DIM")
            .env_remove("CODANNA_EMBED_API_KEY")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    run(&["index", src.to_str().unwrap(), "--no-progress"]);
    run(&["documents", "index", "--no-progress"]);
    let state_path = settings.index_path.join("documents/state.json");
    let prior_state = std::fs::read(&state_path).unwrap();
    std::fs::write(&path, "beta_current\n").unwrap();
    let result = run(&["mcp", "search_documents", "query:alpha_snapshot", "--json"]);
    assert!(
        result.contains("alpha_snapshot"),
        "query must retain the indexed evidence: {result}"
    );
    assert!(!result.contains("beta_current"));
    assert_eq!(
        std::fs::read(state_path).unwrap(),
        prior_state,
        "search cannot re-index corpus files"
    );
}
