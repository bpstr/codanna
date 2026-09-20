use codanna::config::{SemanticSearchConfig, Settings};
use codanna::indexing::facade::IndexFacade;
use codanna::mcp::{
    CodeIntelligenceServer, SemanticSearchRequest, SemanticSearchWithContextRequest,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

fn spawn_embedding_fixture(dimension: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind embedding fixture");
    let address = listener.local_addr().expect("embedding fixture address");

    std::thread::spawn(move || {
        // Enable + initial indexing, then one probe and one query for each
        // freshly loaded MCP server.
        for connection in listener.incoming().take(6) {
            let Ok(mut stream) = connection else {
                break;
            };
            let mut request = Vec::new();
            let mut body_start = None;
            let mut content_length = 0usize;

            loop {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).expect("read embedding request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);

                if body_start.is_none()
                    && let Some(header_end) =
                        request.windows(4).position(|part| part == b"\r\n\r\n")
                {
                    body_start = Some(header_end + 4);
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(str::trim)
                                .and_then(|value| value.parse().ok())
                        })
                        .expect("embedding request content length");
                }

                if body_start.is_some_and(|start| request.len() >= start + content_length) {
                    break;
                }
            }

            let start = body_start.expect("embedding request body");
            let body: Value = serde_json::from_slice(&request[start..start + content_length])
                .expect("parse embedding request");
            let inputs = body["input"].as_array().expect("embedding inputs");
            let data: Vec<Value> = inputs
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    json!({
                        "index": index,
                        "embedding": vec![0.25_f32; dimension],
                    })
                })
                .collect();
            let response_body = json!({ "data": data }).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write embedding response");
        }
    });

    format!("http://{address}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn semantic_mcp_query_initializes_backend_for_loaded_vectors() {
    let workspace = tempfile::tempdir().expect("semantic fixture workspace");
    let source = workspace.path().join("documented.rs");
    std::fs::write(
        &source,
        "/// Finds calendar configuration.\npub fn documented_function() {}\n",
    )
    .expect("write semantic fixture source");

    let index_path = workspace.path().join("index");
    let settings = Arc::new(Settings {
        workspace_root: Some(workspace.path().to_path_buf()),
        index_path: index_path.clone(),
        semantic_search: SemanticSearchConfig {
            enabled: true,
            remote_url: Some(spawn_embedding_fixture(4)),
            remote_model: Some("fixture-model".to_string()),
            remote_dim: Some(4),
            ..Default::default()
        },
        ..Default::default()
    });

    let mut writer = IndexFacade::new(settings.clone()).expect("create semantic fixture index");
    writer
        .enable_semantic_search()
        .expect("enable fixture semantic search");
    writer.index_file(&source).expect("index semantic fixture");
    let semantic_path = index_path.join("semantic");
    writer
        .save_semantic_search(&semantic_path)
        .expect("save semantic fixture");
    drop(writer);

    let mut reader =
        IndexFacade::new(settings.clone()).expect("reopen semantic fixture index for docs");
    assert!(
        reader
            .load_semantic_search(&semantic_path)
            .expect("load semantic fixture")
    );
    let server = CodeIntelligenceServer::new(reader);

    let result = server
        .semantic_search_docs(Parameters(SemanticSearchRequest {
            query: "calendar settings".to_string(),
            limit: 1,
            threshold: None,
            lang: Some("rust".to_string()),
        }))
        .await
        .expect("semantic MCP request");
    let text = result
        .content
        .iter()
        .filter_map(|content| match content {
            ContentBlock::Text(block) => Some(block.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        text.contains("documented_function"),
        "loaded semantic vectors must initialize their query backend, got:\n{text}"
    );

    let mut context_reader =
        IndexFacade::new(settings).expect("reopen semantic fixture index for context");
    assert!(
        context_reader
            .load_semantic_search(&semantic_path)
            .expect("load semantic context fixture")
    );
    let context_server = CodeIntelligenceServer::new(context_reader);
    let context_result = context_server
        .semantic_search_with_context(Parameters(SemanticSearchWithContextRequest {
            query: "calendar settings".to_string(),
            limit: 1,
            threshold: None,
            lang: Some("rust".to_string()),
        }))
        .await
        .expect("semantic context MCP request");
    let context_text = context_result
        .content
        .iter()
        .filter_map(|content| match content {
            ContentBlock::Text(block) => Some(block.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        context_text.contains("documented_function"),
        "context queries must initialize their query backend, got:\n{context_text}"
    );
}
