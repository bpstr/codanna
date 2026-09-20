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

#[test]
fn remote_embedding_preserves_relevant_evidence_beyond_old_character_limit() {
    let server = LocalEmbeddingServer::start();
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path());
    settings.semantic_search.remote_url = Some(server.url.clone());
    settings.semantic_search.remote_model = Some("fixture".into());
    settings.semantic_search.remote_dim = Some(2);
    let source = format!("{}alpha late evidence", "neutral ".repeat(400));
    assert!(source.find("alpha").unwrap() > 2000);
    let path = temp.path().join("late.md");
    std::fs::write(&path, &source).unwrap();
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
                max_chunk_chars: 5000,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    let hits = store.search(query("alpha")).unwrap();
    assert!(
        hits.iter().any(|hit| hit.similarity > 0.99),
        "tail evidence must participate in the vector"
    );
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .flat_map(|request| request["input"].as_array().unwrap())
            .any(|input| input.as_str() == Some(&source))
    );
}

#[test]
fn remote_document_budget_includes_heading_breadcrumbs_and_rolls_back() {
    let server = LocalEmbeddingServer::start();
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path());
    settings.semantic_search.remote_url = Some(server.url.clone());
    settings.semantic_search.remote_model = Some("fixture".into());
    settings.semantic_search.remote_dim = Some(2);
    settings.semantic_search.max_input_tokens = Some(80);
    let path = temp.path().join("headed.md");
    // Each source chunk fits alone; ancestry makes the body input exceed budget.
    std::fs::write(
        &path,
        format!("# {}\n\n{}", "heading ".repeat(5), "body ".repeat(10)),
    )
    .unwrap();
    let mut store = open_from_settings(&settings).unwrap();
    let error = store
        .index_collection(
            "docs",
            &CollectionConfig {
                paths: vec![path],
                ..Default::default()
            },
            &ChunkingConfig {
                min_chunk_chars: 1,
                max_chunk_chars: 80,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("budget"), "{message}");
    assert!(message.contains("heading breadcrumbs"), "{message}");
    assert!(message.contains("not truncated"), "{message}");
    assert!(store.search(query("alpha")).unwrap().is_empty());
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .flat_map(|request| request["input"].as_array().unwrap())
            .all(|input| input.as_str() == Some("probe") || input.as_str() == Some("alpha"))
    );
}

#[test]
fn explicit_remote_revision_rejects_equal_dimension_document_vectors() {
    let server = LocalEmbeddingServer::start();
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path());
    settings.semantic_search.remote_url = Some(server.url.clone());
    settings.semantic_search.remote_model = Some("unchanged-alias".into());
    settings.semantic_search.remote_dim = Some(2);
    settings.semantic_search.model_revision = Some("weights-revision-1".into());
    let path = temp.path().join("guide.md");
    std::fs::write(&path, "alpha policy").unwrap();
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
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    drop(store);
    let state = std::fs::read_to_string(settings.index_path.join("documents/state.json")).unwrap();
    assert!(state.contains("weights-revision-1"));
    assert!(state.contains("complete-input-v2"));
    settings.semantic_search.model_revision = Some("weights-revision-2".into());
    let error = match open_from_settings(&settings) {
        Ok(_) => panic!("equal dimensions cannot hide a model revision change"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("identity") || error.contains("model"),
        "{error}"
    );
}

#[test]
fn remote_preflights_all_batches_and_preserves_multilingual_inputs() {
    let server = LocalEmbeddingServer::start();
    let mut config = codanna::Settings::default().semantic_search;
    config.remote_url = Some(server.url.clone());
    config.remote_model = Some("fixture".into());
    config.remote_dim = Some(2);
    let remote = codanna::indexing::facade::build_embedding_backend(&config).unwrap();
    let mut inputs = vec![String::from("valid"); 64];
    inputs.push("x".repeat(8193));
    let items: Vec<_> = inputs
        .iter()
        .enumerate()
        .map(|(index, text)| {
            (
                codanna::SymbolId::new(index as u32 + 1).unwrap(),
                text.as_str(),
                "document",
            )
        })
        .collect();
    let error = remote.embed_parallel(&items).unwrap_err().to_string();
    assert!(error.contains("input 64"), "{error}");
    assert_eq!(
        server.requests.lock().unwrap().len(),
        1,
        "no batch may reach transport until every input passes"
    );
    let multilingual = format!("{}alpha", "你好🙂".repeat(500));
    assert!(multilingual.chars().count() < 2000);
    assert!(multilingual.len() > 2000);
    remote.embed_one(&multilingual).unwrap();
    assert_eq!(
        server.requests.lock().unwrap().last().unwrap()["input"][0],
        multilingual
    );
    config.max_input_tokens = Some(1);
    let tiny_budget = codanna::indexing::facade::build_embedding_backend(&config).unwrap();
    tiny_budget.embed_one("a").unwrap();
    let calls = server.requests.lock().unwrap().len();
    assert!(tiny_budget.embed_one("ab").is_err());
    assert_eq!(server.requests.lock().unwrap().len(), calls);
}

#[test]
fn configured_tokenizer_counts_complete_remote_input_and_special_tokens() {
    let server = LocalEmbeddingServer::start();
    let temp = tempfile::tempdir().unwrap();
    let model = tokenizers::models::wordlevel::WordLevel::builder()
        .vocab([(String::from("[UNK]"), 0)].into_iter().collect())
        .unk_token("[UNK]".into())
        .build()
        .unwrap();
    let mut tokenizer = tokenizers::Tokenizer::new(model);
    tokenizer.with_pre_tokenizer(Some(tokenizers::pre_tokenizers::whitespace::Whitespace));
    tokenizer.with_post_processor(Some(
        tokenizers::processors::template::TemplateProcessing::builder()
            .try_single("[CLS] $A [SEP]")
            .unwrap()
            .special_tokens(vec![("[CLS]", 1), ("[SEP]", 2)])
            .build()
            .unwrap(),
    ));
    let path = temp.path().join("tokenizer.json");
    tokenizer.save(&path, false).unwrap();
    let mut config = codanna::Settings::default().semantic_search;
    config.remote_url = Some(server.url.clone());
    config.remote_model = Some("fixture".into());
    config.remote_dim = Some(2);
    config.tokenizer_path = Some(path);
    config.max_input_tokens = Some(503);
    let backend = codanna::indexing::facade::build_embedding_backend(&config).unwrap();
    let complete = format!("{}alpha", "neutral ".repeat(500));
    assert!(complete.len() > 2000);
    backend.embed_one(&complete).unwrap();
    let error = backend
        .embed_one(&format!("heading\n\n{complete}"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("504 tokens"), "{error}");
    assert_eq!(
        server.requests.lock().unwrap().len(),
        2,
        "only probe and valid complete input reach transport"
    );
    assert_eq!(
        server.requests.lock().unwrap().last().unwrap()["input"][0],
        complete
    );
}

#[test]
fn code_revision_change_rejects_reopened_vectors_before_query_embedding() {
    let server = LocalEmbeddingServer::start();
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path());
    settings.semantic_search.remote_url = Some(server.url.clone());
    settings.semantic_search.remote_model = Some("unchanged-alias".into());
    settings.semantic_search.remote_dim = Some(2);
    settings.semantic_search.model_revision = Some("weights-revision-1".into());
    let source = temp.path().join("lib.rs");
    std::fs::write(&source, "/// alpha policy.\npub fn alpha_policy() {}\n").unwrap();
    let mut writer =
        codanna::indexing::facade::IndexFacade::new(Arc::new(settings.clone())).unwrap();
    writer.enable_semantic_search().unwrap();
    writer.index_file(&source).unwrap();
    let semantic_path = settings.index_path.join("semantic");
    writer.save_semantic_search(&semantic_path).unwrap();
    assert!(writer.semantic_search_embedding_count() > 0);
    drop(writer);
    let saved = std::fs::read_to_string(semantic_path.join("metadata.json")).unwrap();
    assert!(saved.contains("weights-revision-1"));
    assert!(saved.contains("complete-input-v2"));
    settings.semantic_search.model_revision = Some("weights-revision-2".into());
    let mut reader = codanna::indexing::facade::IndexFacade::new(Arc::new(settings)).unwrap();
    assert!(reader.load_semantic_search(&semantic_path).unwrap());
    let prior_calls = server.requests.lock().unwrap().len();
    let error = reader.ensure_embedding_pool().unwrap_err().to_string();
    assert!(
        error.contains("model revision") && error.contains("Re-index"),
        "{error}"
    );
    assert!(reader.is_semantic_incompatible());
    assert_eq!(
        server.requests.lock().unwrap().len(),
        prior_calls + 1,
        "only the dimension probe reaches the changed provider"
    );
    assert_eq!(
        server.requests.lock().unwrap().last().unwrap()["input"][0],
        "probe"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_code_hot_reload_blocks_direct_and_mcp_queries_without_transport() {
    use codanna::mcp::{CodeIntelligenceServer, SemanticSearchRequest};
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::ContentBlock;

    let server = LocalEmbeddingServer::start();
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path());
    settings.semantic_search.remote_url = Some(server.url.clone());
    settings.semantic_search.remote_model = Some("unchanged-alias".into());
    settings.semantic_search.remote_dim = Some(2);
    settings.semantic_search.model_revision = Some("weights-revision-1".into());
    let source = temp.path().join("lib.rs");
    std::fs::write(&source, "/// alpha policy.\npub fn alpha_policy() {}\n").unwrap();
    let mut facade =
        codanna::indexing::facade::IndexFacade::new(Arc::new(settings.clone())).unwrap();
    facade.enable_semantic_search().unwrap();
    facade.index_file(&source).unwrap();
    let semantic_path = settings.index_path.join("semantic");
    facade.save_semantic_search(&semantic_path).unwrap();
    let metadata_path = semantic_path.join("metadata.json");
    let compatible = std::fs::read(&metadata_path).unwrap();
    let mut incompatible: Value = serde_json::from_slice(&compatible).unwrap();
    incompatible["embedding_identity"] = Value::String(
        incompatible["embedding_identity"]
            .as_str()
            .unwrap()
            .replace("weights-revision-1", "weights-revision-2"),
    );
    assert_eq!(incompatible["dimension"], 2);
    std::fs::write(&metadata_path, serde_json::to_vec(&incompatible).unwrap()).unwrap();
    let calls = server.requests.lock().unwrap().len();
    assert!(facade.load_semantic_search(&semantic_path).is_err());
    assert!(facade.is_semantic_incompatible());
    let error = facade
        .semantic_search_docs_with_language("alpha", 1, Some("rust"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("incompatible") && error.contains("failed reload"),
        "{error}"
    );
    assert!(facade.prepare_semantic_query().is_err());
    assert!(facade.ensure_embedding_pool().is_err());
    assert!(facade.save_semantic_search(&semantic_path).is_err());
    assert_eq!(server.requests.lock().unwrap().len(), calls);

    let mcp = CodeIntelligenceServer::new(facade);
    let request = || {
        Parameters(SemanticSearchRequest {
            query: "alpha".into(),
            limit: 1,
            threshold: None,
            lang: Some("rust".into()),
        })
    };
    let response = mcp.semantic_search_docs(request()).await.unwrap();
    assert_eq!(response.is_error, Some(true));
    assert!(
        response
            .content
            .iter()
            .any(|content| matches!(content, ContentBlock::Text(text)
        if text.text.contains("incompatible") && text.text.contains("failed reload")))
    );
    assert_eq!(
        server.requests.lock().unwrap().len(),
        calls,
        "failed hot reload must block the MCP preparation shortcut and query transport"
    );

    // A compatible explicit reload is the recovery boundary; the prior backend
    // can be reused only after this new generation passes its identity check.
    std::fs::write(&metadata_path, &compatible).unwrap();
    {
        let shared = mcp.get_facade_arc();
        let mut facade = shared.write().await;
        assert!(facade.load_semantic_search(&semantic_path).unwrap());
        assert!(!facade.is_semantic_incompatible());
    }
    let recovered = mcp.semantic_search_docs(request()).await.unwrap();
    assert_ne!(recovered.is_error, Some(true));
    assert_eq!(
        server.requests.lock().unwrap().len(),
        calls + 1,
        "compatible recovery should generate only the requested query embedding"
    );
}
