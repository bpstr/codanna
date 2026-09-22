//! Rebuild cost and identity contracts through real CLI processes.
//! Only a joined loopback HTTP fixture receives synthetic inputs. Child processes
//! have an empty environment and a disposable HOME; no provider keys are used.

use codanna::indexing::facade::IndexFacade;
use codanna::semantic::SimpleSemanticSearch;
use codanna::storage::IndexPersistence;
use codanna::{Settings, SymbolId};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SOURCE: &str = "/// alpha calendar preference owner.\npub fn calendar_owner() -> u8 { 1 }\n/// beta calendar preference consumer.\npub fn read_calendar() -> u8 { calendar_owner() }\n";

struct Endpoint {
    url: String,
    inputs: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Endpoint {
    fn start(dimensions: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded = Arc::clone(&inputs);
        let stopped = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => respond(stream, dimensions, &recorded),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("loopback accept failed: {error}"),
                }
            }
        });
        Self {
            url,
            inputs,
            stop,
            thread: Some(thread),
        }
    }

    fn take_inputs(&self) -> Vec<String> {
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

fn respond(mut stream: TcpStream, dimensions: usize, recorded: &Mutex<Vec<String>>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let (header_end, length) = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "incomplete fixture request");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() < 1_000_000, "fixture request exceeded bound");
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            assert!(headers.starts_with("POST /v1/embeddings "), "{headers}");
            assert!(!headers.to_ascii_lowercase().contains("authorization:"));
            let length = headers
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
    let request: Value = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
    let inputs: Vec<_> = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| input.as_str().unwrap().to_owned())
        .collect();
    let data: Vec<_> = inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let mut vector = vec![0.0_f32; dimensions];
            vector[usize::from(input.contains("beta"))] = 1.0;
            json!({"index": index, "embedding": vector})
        })
        .collect();
    recorded.lock().unwrap().extend(inputs);
    let body = json!({"data": data}).to_string();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}

struct Workspace {
    temp: tempfile::TempDir,
}

impl Workspace {
    fn new(endpoint: &Endpoint) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Self { temp };
        std::fs::create_dir_all(workspace.root().join("src")).unwrap();
        std::fs::create_dir_all(workspace.root().join(".home")).unwrap();
        std::fs::create_dir_all(workspace.root().join(".codanna")).unwrap();
        workspace.source(SOURCE);
        workspace.configure(endpoint, "fixture-model", 2, "");
        workspace
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn source(&self, text: &str) {
        std::fs::write(self.root().join("src/lib.rs"), text).unwrap();
    }

    fn configure(&self, endpoint: &Endpoint, model: &str, dimension: usize, extra: &str) {
        let source = self.root().join("src").canonicalize().unwrap();
        let config = format!(
            "index_path = \".codanna/index\"\n[indexing]\nindexed_paths = [{}]\n[semantic_search]\nenabled = true\nmodel = \"invalid-local-fixture-model\"\nremote_url = {:?}\nremote_model = {:?}\nremote_dim = {dimension}\n{extra}\n",
            serde_json::to_string(&source.to_string_lossy()).unwrap(),
            endpoint.url,
            model,
        );
        std::fs::write(self.root().join(".codanna/settings.toml"), config).unwrap();
    }

    fn rebuild(&self) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codanna"));
        command.env_clear().env("HOME", self.root().join(".home"));
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
            .current_dir(self.root())
            .args([
                "--config",
                ".codanna/settings.toml",
                "index",
                "src",
                "--force",
                "--no-progress",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn assert_current_vectors(&self, names: &[&str], dimension: usize) -> Vec<SymbolId> {
        let index_path = self.root().join(".codanna/index");
        let mut settings = Settings {
            index_path: index_path.clone(),
            workspace_root: Some(self.root().to_path_buf()),
            ..Default::default()
        };
        settings.semantic_search.enabled = false;
        let index: IndexFacade = IndexPersistence::new(index_path.clone())
            .load_facade_lite(Arc::new(settings))
            .unwrap();
        let ids: Vec<_> = names
            .iter()
            .map(|name| {
                let symbols = index.find_symbols_by_name(name, None);
                assert_eq!(symbols.len(), 1, "{name}");
                symbols[0].id
            })
            .collect();
        let vectors = SimpleSemanticSearch::load_remote(&index_path.join("semantic")).unwrap();
        assert_eq!(vectors.embedding_count(), ids.len());
        let mut query = vec![0.0; dimension];
        query[0] = 1.0;
        let found = vectors
            .search_with_embedding_and_language(&query, 100, None)
            .unwrap();
        assert_eq!(
            found.iter().map(|(id, _)| *id).collect::<HashSet<_>>(),
            ids.iter().copied().collect()
        );
        if names.len() == 2 {
            assert_eq!(
                found[0].0, ids[0],
                "cached vector must map to current owner, not a reused numeric ID"
            );
            assert_eq!(found[0].1, 1.0);
        }
        ids
    }
}

fn assert_inputs(endpoint: &Endpoint, documents: usize) -> Vec<String> {
    let inputs = endpoint.take_inputs();
    let docs: Vec<_> = inputs
        .iter()
        .filter(|input| input.as_str() != "probe")
        .cloned()
        .collect();
    println!(
        "provider_inputs={} probe_inputs={} document_inputs={}: {docs:?}",
        inputs.len(),
        inputs.len() - docs.len(),
        docs.len()
    );
    assert_eq!(
        docs.len(),
        documents,
        "unexpected repeated embedding inputs: {inputs:?}"
    );
    assert_eq!(
        inputs.len() - docs.len(),
        1,
        "one initialization probe remains, not a zero-token claim"
    );
    docs
}

#[test]
fn force_rebuild_reuses_exact_inputs_and_remaps_new_symbol_ids() {
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    workspace.rebuild();
    assert_inputs(&endpoint, 2);
    let old = workspace.assert_current_vectors(&["calendar_owner", "read_calendar"], 2);
    workspace.source(&format!(
        "pub fn inserted_without_docs() {{}}\n{}",
        SOURCE.replace("{ 1 }", "{ 2 }")
    ));
    workspace.rebuild();
    assert_inputs(&endpoint, 0);
    let new = workspace.assert_current_vectors(&["calendar_owner", "read_calendar"], 2);
    assert_ne!(old, new, "fixture must exercise ID remapping");
}

#[test]
fn changed_input_embeds_only_the_miss_and_deleted_symbols_do_not_return() {
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    workspace.rebuild();
    assert_inputs(&endpoint, 2);
    workspace.source(&SOURCE.replace("alpha calendar", "alpha changed calendar"));
    workspace.rebuild();
    let inputs = assert_inputs(&endpoint, 1);
    assert!(inputs[0].contains("alpha changed calendar"));
    workspace.assert_current_vectors(&["calendar_owner", "read_calendar"], 2);
    workspace.source(
        "/// alpha changed calendar preference owner.\npub fn calendar_owner() -> u8 { 1 }\n",
    );
    workspace.rebuild();
    assert_inputs(&endpoint, 0);
    workspace.assert_current_vectors(&["calendar_owner"], 2);
}

#[test]
fn missing_and_corrupt_cache_are_misses_not_stale_vector_reuse() {
    for corrupt in [false, true] {
        let endpoint = Endpoint::start(2);
        let workspace = Workspace::new(&endpoint);
        workspace.rebuild();
        assert_inputs(&endpoint, 2);
        let cache = workspace
            .root()
            .join(".codanna/index/semantic/embedding-cache.json");
        if corrupt {
            std::fs::write(cache, "{invalid cache").unwrap();
        } else {
            std::fs::remove_file(cache).unwrap();
        }
        workspace.rebuild();
        assert_inputs(&endpoint, 2);
        workspace.assert_current_vectors(&["calendar_owner", "read_calendar"], 2);
    }
}

#[test]
fn model_revision_and_input_policy_changes_cannot_reuse_cached_vectors() {
    for (model, extra) in [
        ("different-model", ""),
        ("fixture-model", "model_revision = \"new-revision\""),
        ("fixture-model", "max_input_tokens = 1024"),
    ] {
        let endpoint = Endpoint::start(2);
        let workspace = Workspace::new(&endpoint);
        workspace.rebuild();
        assert_inputs(&endpoint, 2);
        workspace.configure(&endpoint, model, 2, extra);
        workspace.rebuild();
        assert_inputs(&endpoint, 2);
        workspace.assert_current_vectors(&["calendar_owner", "read_calendar"], 2);
    }
}

#[test]
fn endpoint_or_dimension_changes_cannot_reuse_cached_vectors() {
    for dimension in [2, 4] {
        let first = Endpoint::start(2);
        let second = Endpoint::start(dimension);
        let workspace = Workspace::new(&first);
        workspace.rebuild();
        assert_inputs(&first, 2);
        workspace.configure(&second, "fixture-model", dimension, "");
        workspace.rebuild();
        assert_inputs(&second, 2);
        assert!(first.take_inputs().is_empty());
        workspace.assert_current_vectors(&["calendar_owner", "read_calendar"], dimension);
    }
}

fn pressure_source(count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        use std::fmt::Write as _;
        writeln!(
            &mut source,
            "/// pressure embedding input {index:04}.\npub fn pressure_{index:04}() -> usize {{ {index} }}"
        )
        .unwrap();
    }
    source
}

#[test]
fn large_force_rebuild_reuses_late_snapshot_hits_before_admitting_early_misses() {
    const COUNT: usize = 4_500;
    const CACHE_CAPACITY: usize = 4_096;
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    workspace.source(&pressure_source(COUNT));

    workspace.rebuild();
    assert_inputs(&endpoint, COUNT);

    // The persisted cache holds the newest CACHE_CAPACITY inputs. On the next
    // same-order rebuild, the first COUNT-CACHE_CAPACITY inputs are true misses.
    // Miss admission must not evict the later compatible hits before lookup.
    workspace.rebuild();
    let documents = assert_inputs(&endpoint, COUNT - CACHE_CAPACITY);
    assert!(
        documents
            .iter()
            .all(|input| input.starts_with("pressure embedding input 0")),
        "only the oldest uncached inputs should require embedding"
    );

    // New misses are still admitted, so the cache continues learning rather
    // than freezing one favorable snapshot. Capacity means a later rebuild
    // must still miss COUNT-CACHE_CAPACITY inputs, but not the entire corpus.
    workspace.rebuild();
    assert_inputs(&endpoint, COUNT - CACHE_CAPACITY);
}

fn duplicate_pressure_source(count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        use std::fmt::Write as _;
        writeln!(
            &mut source,
            "/// shared duplicate embedding input.\npub fn duplicate_{index:04}() -> usize {{ {index} }}"
        )
        .unwrap();
    }
    source
}

#[test]
fn duplicate_missing_inputs_are_embedded_once_per_collector_batch() {
    let endpoint = Endpoint::start(2);
    let workspace = Workspace::new(&endpoint);
    workspace.source(&duplicate_pressure_source(200));

    workspace.rebuild();
    let documents = assert_inputs(&endpoint, 1);
    assert_eq!(documents, vec!["shared duplicate embedding input."]);

    workspace.rebuild();
    assert_inputs(&endpoint, 0);
}

#[path = "support/body_rebuild_cache_cases.rs"]
mod body_rebuild_cache;

#[path = "support/retrieval_body_cases.rs"]
mod retrieval_body;
