//! Streamable HTTP serves both protocol generations on rmcp 3.x:
//! 2026-07-28 requests run sessionless (no `Mcp-Session-Id` minted);
//! legacy clients keep protocol sessions through the deprecation
//! window. SSE responses carry no event ids (resumability removed).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

fn codanna_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codanna"))
}

fn write_fixture(workspace: &Path) {
    let src = workspace.join("src");
    std::fs::create_dir_all(&src).expect("create src dir");
    std::fs::write(
        src.join("alpha.rs"),
        r#"
pub fn http_target() -> i32 {
    1
}
"#,
    )
    .expect("write fixture");
}

fn write_settings(workspace: &Path) {
    let codanna_dir = workspace.join(".codanna");
    std::fs::create_dir_all(&codanna_dir).expect("create .codanna");

    let src_abs = workspace
        .join("src")
        .canonicalize()
        .expect("src dir should exist and be resolvable");
    let src_path = crate::common::toml_path_literal(&src_abs);

    let settings = format!(
        r#"
index_path = ".codanna/index"

[indexing]
indexed_paths = [{src_path}]

[semantic_search]
enabled = false
"#
    );

    std::fs::write(codanna_dir.join("settings.toml"), settings).expect("write settings");
}

fn seed_workspace() -> TempDir {
    let workspace = TempDir::new().expect("temp dir");
    write_fixture(workspace.path());
    write_settings(workspace.path());
    let test_home = workspace.path().join(".home");
    std::fs::create_dir_all(&test_home).expect("create test home");
    let status = Command::new(codanna_binary())
        .args(["index", "src", "--no-progress"])
        .current_dir(workspace.path())
        .env("HOME", &test_home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run seed index");
    assert!(status.success(), "seed index should succeed");
    workspace
}

struct HttpServe {
    child: Child,
    port: u16,
    stderr_reader: Option<std::thread::JoinHandle<()>>,
}

impl Drop for HttpServe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn spawn_http_serve(workspace: &Path) -> HttpServe {
    let test_home = workspace.join(".home");
    let child = Command::new(codanna_binary())
        .args(["serve", "--http", "--bind", "127.0.0.1:0"])
        .current_dir(workspace)
        .env("HOME", &test_home)
        .env(
            "CODANNA_MCP_TOKEN",
            "http-session-fixture-credential-1234567890",
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn serve --http");

    let mut serve = HttpServe {
        child,
        port: 0,
        stderr_reader: None,
    };
    let stderr = serve.child.stderr.take().expect("server stderr");
    let (sender, receiver) = std::sync::mpsc::channel();
    serve.stderr_reader = Some(std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if let Some(address) = line.strip_prefix("HTTP MCP server listening on http://") {
                let address: std::net::SocketAddr = address.parse().expect("actual server socket");
                let _ = sender.send(address.port());
            }
        }
    }));
    serve.port = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("server must announce its bound socket");
    assert_ne!(serve.port, 0);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (status, _, _) = http_request(serve.port, "GET", "/health", &[], None);
        if status == 200 {
            return serve;
        }
        assert!(
            Instant::now() < deadline,
            "serve --http did not become healthy within 20s"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Minimal HTTP/1.1 exchange over std TCP. SSE responses never EOF, so
/// the reader stops on read-timeout and returns what arrived; callers
/// assert on the accumulated frames.
fn http_request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> (u16, String, String) {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return (0, String::new(), String::new());
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");

    let body_bytes = body.unwrap_or("");
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n", body_bytes.len()));
    req.push_str(body_bytes);
    if stream.write_all(req.as_bytes()).is_err() {
        return (0, String::new(), String::new());
    }

    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                // A TCP read or HTTP chunk may end inside JSON. Stop only
                // after decoding transfer framing and a complete payload.
                if let Some((_, _, body)) = decode_http_response(&raw)
                    && let Some(payload) = try_response_payload(&body)
                    && (payload.get("result").is_some() || payload.get("error").is_some())
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    decode_http_response(&raw)
        .unwrap_or_else(|| (0, String::new(), String::from_utf8_lossy(&raw).into_owned()))
}

/// Decode complete HTTP chunks before looking for SSE/JSON boundaries. An
/// unfinished chunk remains buffered for the next TCP read.
fn decode_http_response(raw: &[u8]) -> Option<(u16, String, String)> {
    let boundary = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..boundary]).into_owned();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let body = &raw[boundary + 4..];
    let chunked = header_value(&head, "transfer-encoding").is_some_and(|value| {
        value
            .split(',')
            .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
    });
    let decoded = if chunked {
        let mut decoded = Vec::new();
        let mut remaining = body;
        while let Some(end) = remaining.windows(2).position(|bytes| bytes == b"\r\n") {
            let size_line = std::str::from_utf8(&remaining[..end]).expect("ASCII chunk size");
            let size = usize::from_str_radix(size_line.split(';').next().unwrap().trim(), 16)
                .expect("valid HTTP chunk size");
            if size == 0 {
                break;
            }
            remaining = &remaining[end + 2..];
            if remaining.len() < size + 2 {
                break;
            }
            assert_eq!(&remaining[size..size + 2], b"\r\n", "HTTP chunk terminator");
            decoded.extend_from_slice(&remaining[..size]);
            remaining = &remaining[size + 2..];
        }
        decoded
    } else {
        body.to_vec()
    };
    Some((status, head, String::from_utf8_lossy(&decoded).into_owned()))
}

/// ASCII-case-insensitive header lookup preserving the value's case.
fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then_some(v.trim())
    })
}

fn mcp_headers<'a>(method: &'a str, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut h = vec![
        (
            "Authorization",
            "Bearer http-session-fixture-credential-1234567890",
        ),
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
        ("Mcp-Method", method),
    ];
    h.extend_from_slice(extra);
    h
}

/// Extract the first JSON-RPC payload from a response body that may be
/// SSE-framed (`data: {...}`) or plain JSON.
fn try_response_payload(body: &str) -> Option<Value> {
    if let Ok(payload) = serde_json::from_str(body) {
        return Some(payload);
    }
    // Only complete SSE events count. Empty data events and comments may
    // precede the JSON-RPC message; HTTP chunks do not define event boundaries.
    let normalized = body.replace("\r\n", "\n");
    for event in normalized
        .split("\n\n")
        .take(normalized.matches("\n\n").count())
    {
        let data = event
            .lines()
            .filter_map(|line| {
                line.strip_prefix("data:")
                    .map(|value| value.strip_prefix(' ').unwrap_or(value))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if let Ok(payload) = serde_json::from_str(&data) {
            return Some(payload);
        }
    }
    None
}

fn response_payload(body: &str) -> Value {
    try_response_payload(body)
        .unwrap_or_else(|| panic!("no JSON-RPC payload in response body:\n{body}"))
}

#[test]
fn response_reader_reassembles_json_split_across_http_chunks() {
    let chunks: &[&[u8]] = &[
        b"data: \n\n",
        b"data: {\"jsonrpc\":\"2.0\",\"id\":1,\"res",
        b"ult\":{\"tools\":[]}}\n\n",
    ];
    let mut raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for (index, chunk) in chunks.iter().enumerate() {
        raw.extend_from_slice(format!("{:x};fixture=yes\r\n", chunk.len()).as_bytes());
        // A partial TCP read must not be mistaken for a complete payload.
        raw.extend_from_slice(&chunk[..chunk.len() - 1]);
        let (_, _, body) = decode_http_response(&raw).unwrap();
        assert!(try_response_payload(&body).is_none());
        raw.extend_from_slice(&chunk[chunk.len() - 1..]);
        raw.extend_from_slice(b"\r\n");
        let (_, _, body) = decode_http_response(&raw).unwrap();
        if index < chunks.len() - 1 {
            assert!(try_response_payload(&body).is_none());
        }
    }
    raw.extend_from_slice(b"0\r\n\r\n");
    let (status, _, body) = decode_http_response(&raw).unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        response_payload(&body),
        json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": []}})
    );
}

fn stateless_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// A 2026-07-28 client POSTs `tools/list` with per-request `_meta` and
/// the `Mcp-Method` header. The request succeeds without any session:
/// no `Mcp-Session-Id` is minted.
#[test]
fn serve_http_stateless_request_without_session() {
    let workspace = seed_workspace();
    let serve = spawn_http_serve(workspace.path());

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": { "_meta": stateless_meta() }
    })
    .to_string();

    let (status, head, resp_body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("tools/list", &[("MCP-Protocol-Version", "2026-07-28")]),
        Some(&body),
    );

    assert_eq!(status, 200, "stateless POST succeeds\nhead:\n{head}");
    assert!(
        header_value(&head, "mcp-session-id").is_none(),
        "no session is minted for a 2026-07-28 request\nhead:\n{head}"
    );

    let payload = response_payload(&resp_body);
    let tools = payload["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("stateless tools/list returns tools\n{payload}"));
    assert_eq!(
        tools.len(),
        codanna::mcp::catalog::ToolKind::ALL.len(),
        "all tools served sessionless\n{payload}"
    );
}

/// A legacy client completes the initialize handshake, receives an
/// `Mcp-Session-Id`, and keeps working through the deprecation window.
#[test]
fn serve_http_legacy_client_keeps_session() {
    let workspace = seed_workspace();
    let serve = spawn_http_serve(workspace.path());

    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "http-legacy-test", "version": "0"}
        }
    })
    .to_string();

    let (status, head, body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("initialize", &[]),
        Some(&init),
    );
    assert_eq!(status, 200, "legacy initialize succeeds\nhead:\n{head}");
    let payload = response_payload(&body);
    assert!(
        payload["result"]["serverInfo"]["name"].is_string(),
        "initialize result carries serverInfo\n{payload}"
    );
    let sid = header_value(&head, "mcp-session-id")
        .unwrap_or_else(|| panic!("legacy initialize mints a session\nhead:\n{head}"))
        .to_string();

    let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
    let (status, _, _) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("notifications/initialized", &[("Mcp-Session-Id", &sid)]),
        Some(&initialized),
    );
    assert!(
        status == 200 || status == 202,
        "initialized notification accepted, got {status}"
    );

    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string();
    let (status, head, body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("tools/list", &[("Mcp-Session-Id", &sid)]),
        Some(&list),
    );
    assert_eq!(status, 200, "legacy tools/list succeeds\nhead:\n{head}");
    let payload = response_payload(&body);
    assert_eq!(
        payload["result"]["tools"].as_array().map(Vec::len),
        Some(codanna::mcp::catalog::ToolKind::ALL.len()),
        "all tools on the legacy session\n{payload}"
    );
}

/// Resumability is gone from the transport: response frames carry no
/// SSE event ids (nothing for `Last-Event-ID` to reference), and a
/// request re-issued under a new JSON-RPC id succeeds as a fresh
/// request.
#[test]
fn serve_http_no_event_ids_and_reissue_succeeds() {
    let workspace = seed_workspace();
    let serve = spawn_http_serve(workspace.path());

    let request = |id: u64| {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": { "_meta": stateless_meta() }
        })
        .to_string()
    };

    let (status, _, body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("tools/list", &[("MCP-Protocol-Version", "2026-07-28")]),
        Some(&request(1)),
    );
    assert_eq!(status, 200);
    assert!(
        !body.lines().any(|l| l.trim_start().starts_with("id:")),
        "response frames must carry no SSE event ids\n{body}"
    );

    let (status, _, body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("tools/list", &[("MCP-Protocol-Version", "2026-07-28")]),
        Some(&request(2)),
    );
    assert_eq!(status, 200, "re-issued request succeeds as fresh");
    let payload = response_payload(&body);
    assert_eq!(
        payload["id"], 2,
        "response answers the re-issued id\n{payload}"
    );
    assert_eq!(
        payload["result"]["tools"].as_array().map(Vec::len),
        Some(codanna::mcp::catalog::ToolKind::ALL.len()),
        "re-issue serves the full result\n{payload}"
    );
}

/// A 2026-07-28 request (generation named by the header) whose params
/// carry no `_meta` is refused per-request with InvalidParams (-32602);
/// the server keeps serving on the same port.
#[test]
fn serve_http_stateless_missing_meta_invalid_params() {
    let workspace = seed_workspace();
    let serve = spawn_http_serve(workspace.path());

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    })
    .to_string();

    let (_, _, resp_body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("tools/list", &[("MCP-Protocol-Version", "2026-07-28")]),
        Some(&body),
    );
    let payload = response_payload(&resp_body);
    assert_eq!(
        payload["error"]["code"], -32602,
        "missing per-request _meta is InvalidParams\n{payload}"
    );

    let good = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": { "_meta": stateless_meta() }
    })
    .to_string();
    let (status, _, resp_body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("tools/list", &[("MCP-Protocol-Version", "2026-07-28")]),
        Some(&good),
    );
    assert_eq!(status, 200, "server keeps serving after the refusal");
    let payload = response_payload(&resp_body);
    assert_eq!(
        payload["result"]["tools"].as_array().map(Vec::len),
        Some(codanna::mcp::catalog::ToolKind::ALL.len()),
        "well-formed request still serves\n{payload}"
    );
}

/// A stateless request carrying `_meta` but no `MCP-Protocol-Version`
/// header is refused with the missing-header code (-32020); the server
/// keeps serving on the same port.
#[test]
fn serve_http_stateless_missing_protocol_header_coded() {
    let workspace = seed_workspace();
    let serve = spawn_http_serve(workspace.path());

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": { "_meta": stateless_meta() }
    })
    .to_string();

    let (_, _, resp_body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("tools/list", &[]),
        Some(&body),
    );
    let payload = response_payload(&resp_body);
    assert_eq!(
        payload["error"]["code"], -32020,
        "missing MCP-Protocol-Version header is the coded refusal\n{payload}"
    );

    let good = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": { "_meta": stateless_meta() }
    })
    .to_string();
    let (status, _, resp_body) = http_request(
        serve.port,
        "POST",
        "/mcp",
        &mcp_headers("tools/list", &[("MCP-Protocol-Version", "2026-07-28")]),
        Some(&good),
    );
    assert_eq!(status, 200, "server keeps serving after the refusal");
    let payload = response_payload(&resp_body);
    assert_eq!(
        payload["result"]["tools"].as_array().map(Vec::len),
        Some(codanna::mcp::catalog::ToolKind::ALL.len()),
        "well-formed request still serves\n{payload}"
    );
}
