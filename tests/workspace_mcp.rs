//! Real MCP protocol/worker witnesses. All files and homes are temporary;
//! embeddings are disabled and no provider credentials enter subprocesses.
#![cfg(unix)]

use rmcp::model::*;
use rmcp::service::{RequestContext, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn command(home: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codanna"));
    command.env_clear().env("HOME", home).env("PATH", "/usr/bin:/bin")
        .env("NO_COLOR", "1").current_dir(cwd);
    for key in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        if let Some(value) = std::env::var_os(key) { command.env(key, value); }
    }
    command
}

fn run_cli(home: &Path, cwd: &Path, args: &[&str]) {
    let out = tempfile::NamedTempFile::new().unwrap();
    let err = tempfile::NamedTempFile::new().unwrap();
    let mut command = command(home, cwd);
    command.args(args).stdin(Stdio::null())
        .stdout(out.reopen().unwrap()).stderr(err.reopen().unwrap());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "stdout={} stderr={}",
                fs::read_to_string(out.path()).unwrap(), fs::read_to_string(err.path()).unwrap());
            return;
        }
        if Instant::now() > deadline {
            let _ = child.kill(); let _ = child.wait();
            panic!("fixture CLI exceeded deadline: {args:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn checkout(root: &Path, symbol: &str) {
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), format!("pub fn {symbol}() -> u32 {{ 1 }}\n")).unwrap();
}

fn configure(root: &Path) {
    fs::create_dir_all(root.join(".codanna")).unwrap();
    let mut settings = codanna::Settings::default();
    settings.semantic_search.enabled = false;
    settings.indexing.parallelism = 1;
    settings.indexing.indexed_paths = vec![PathBuf::from(".")];
    fs::write(root.join(".codanna/settings.toml"), toml::to_string_pretty(&settings).unwrap()).unwrap();
    fs::write(root.join(".codannaignore"), ".codanna/\n.git/\n").unwrap();
}

struct Fixture {
    _temporary: TempDir,
    home: PathBuf,
    assign: PathBuf,
    codanna: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let home = temporary.path().join("home");
        let assign = temporary.path().join("assign");
        let codanna = temporary.path().join("codanna");
        fs::create_dir_all(&home).unwrap();
        checkout(&assign.join("assign-core"), "product_only_identity");
        checkout(&assign.join("assign-web"), "product_only_identity");
        checkout(&codanna, "unrelated_only_identity");
        configure(&assign); configure(&codanna);
        run_cli(&home, &assign, &["index", "--no-progress"]);
        run_cli(&home, &codanna, &["index", "--no-progress"]);
        Self { _temporary: temporary, home, assign, codanna }
    }
}

async fn connect<C: ClientHandler>(handler: C, home: &Path, cwd: &Path) -> RunningService<RoleClient, C> {
    let mut process = command(home, cwd);
    process.args(["workspace", "serve"]).stderr(Stdio::null());
    let mut process = tokio::process::Command::from(process);
    process.kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(15), handler.serve(TokioChildProcess::new(process).unwrap()))
        .await.expect("MCP handshake deadline").expect("MCP handshake")
}

fn request(name: &str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone())
}

async fn call<C: ClientHandler>(client: &RunningService<RoleClient, C>, name: &str, args: Value) -> CallToolResult {
    let result = tokio::time::timeout(Duration::from_secs(20), client.call_tool(request(name, args)))
        .await.expect("tool deadline").expect("tool call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result
}

fn owner(result: &CallToolResult) -> &str {
    result.structured_content.as_ref().unwrap()["workspace"]["name"].as_str().unwrap()
}

fn rendered(result: &CallToolResult) -> String { serde_json::to_string(result).unwrap() }

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_handshake_and_catalogue_from_home_do_not_create_state() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let client = connect((), &home, &home).await;
    let tools = client.list_tools(Default::default()).await.unwrap();
    assert!(tools.tools.iter().any(|tool| tool.name == "list_workspaces"));
    let schema = serde_json::to_value(tools.tools.iter().find(|tool| tool.name == "search_context").unwrap()).unwrap();
    assert_eq!(schema["inputSchema"]["properties"]["workspace"]["type"], "string");
    assert_eq!(schema["inputSchema"]["properties"]["project_path"]["type"], "string");
    let listed = call(&client, "list_workspaces", json!({})).await;
    assert_eq!(listed.structured_content.unwrap()["workspaces"], json!([]));
    assert!(client.call_tool(request("get_workspace", json!({}))).await.is_err());
    assert!(client.call_tool(request("get_workspace", json!({"workspace": "missing"}))).await.is_err());
    assert!(!home.join(".codanna").exists());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_one_connection_queries_assign_and_codanna_independently() {
    let fixture = Fixture::new();
    let client = connect((), &fixture.home, &fixture.home).await;
    let listed = call(&client, "list_workspaces", json!({})).await;
    assert_eq!(listed.structured_content.unwrap()["workspaces"].as_array().unwrap().len(), 2);
    let before = call(&client, "get_workspace", json!({"workspace": "assign"})).await;
    assert_eq!(before.structured_content.unwrap()["result"]["worker_loaded"], false);
    let (assign, codanna) = tokio::join!(
        call(&client, "search_context", json!({"workspace": "assign", "query": "product_only_identity"})),
        call(&client, "search_context", json!({"workspace": "codanna", "query": "unrelated_only_identity"})),
    );
    assert_eq!(owner(&assign), "assign"); assert_eq!(owner(&codanna), "codanna");
    assert!(rendered(&assign).contains("assign-core"));
    assert!(rendered(&assign).contains("assign-web"));
    assert!(!rendered(&assign).contains("unrelated_only_identity"));
    assert!(rendered(&codanna).contains("unrelated_only_identity"));
    assert!(!rendered(&codanna).contains("product_only_identity"));
    let after = call(&client, "get_workspace", json!({"workspace": "assign"})).await;
    assert_eq!(after.structured_content.unwrap()["result"]["worker_loaded"], true);
    let path = call(&client, "search_context", json!({"project_path": fixture.assign.join("assign-web/src"), "query": "product_only_identity"})).await;
    assert_eq!(owner(&path), "assign");
    assert!(client.call_tool(request("get_workspace", json!({"workspace": "assign", "project_path": fixture.codanna}))).await.is_err());
    assert!(client.peer().send_request(ClientRequest::CustomRequest(CustomRequest::new("requests/codanna/force-reindex", None))).await.is_err());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_explicit_override_never_changes_the_connection_default() {
    let fixture = Fixture::new();
    let client = connect((), &fixture.home, &fixture.assign.join("assign-web/src")).await;
    assert_eq!(owner(&call(&client, "get_workspace", json!({})).await), "assign");
    assert_eq!(owner(&call(&client, "get_workspace", json!({"workspace": "codanna"})).await), "codanna");
    assert_eq!(owner(&call(&client, "get_workspace", json!({})).await), "assign");
    client.cancel().await.unwrap();
}

#[derive(Clone)]
struct RootsClient {
    roots: Arc<RwLock<Vec<PathBuf>>>,
    version: ProtocolVersion,
}

#[allow(deprecated)]
impl ClientHandler for RootsClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            serde_json::from_value(json!({"roots": {"listChanged": true}})).unwrap(),
            Implementation::new("workspace-fixture", "1"),
        ).with_protocol_version(self.version.clone())
    }

    async fn list_roots(&self, _context: RequestContext<RoleClient>) -> Result<ListRootsResult, ErrorData> {
        let roots: Vec<_> = self.roots.read().unwrap().iter().map(|root| {
            json!({"uri": reqwest::Url::from_file_path(root).unwrap().to_string()})
        }).collect();
        serde_json::from_value(json!({"roots": roots})).map_err(|e| ErrorData::internal_error(e.to_string(), None))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_legacy_and_mrtr_roots_select_products_without_paths_in_config() {
    let fixture = Fixture::new();
    for version in ["2025-11-25", "2026-07-28"] {
        let roots = Arc::new(RwLock::new(vec![fixture.assign.join("assign-core"), fixture.assign.join("assign-web")]));
        let handler = RootsClient { roots: roots.clone(), version: serde_json::from_value(json!(version)).unwrap() };
        let client = connect(handler, &fixture.home, &fixture.home).await;
        assert_eq!(owner(&call(&client, "get_workspace", json!({})).await), "assign");
        *roots.write().unwrap() = vec![fixture.codanna.clone()];
        assert_eq!(owner(&call(&client, "get_workspace", json!({})).await), "codanna");
        *roots.write().unwrap() = vec![fixture.assign.clone(), fixture.codanna.clone()];
        assert!(client.call_tool(request("get_workspace", json!({}))).await.is_err());
        assert_eq!(owner(&call(&client, "get_workspace", json!({"workspace": "assign"})).await), "assign");
        let unknown = fixture.home.join("unregistered");
        fs::create_dir_all(unknown.join(".git")).unwrap();
        *roots.write().unwrap() = vec![fixture.assign.clone(), unknown.clone()];
        assert!(client.call_tool(request("get_workspace", json!({}))).await.is_err());
        assert!(!unknown.join(".codanna").exists());
        client.cancel().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_root_continuations_are_bound_and_single_use() {
    let fixture = Fixture::new();
    let handler = RootsClient { roots: Arc::new(RwLock::new(vec![fixture.assign.clone()])), version: ProtocolVersion::V_2026_07_28 };
    let client = connect(handler, &fixture.home, &fixture.home).await;
    let response = client.call_tool_once(request("get_workspace", json!({}))).await.unwrap();
    let state = match response {
        CallToolResponse::InputRequired(result) => result.request_state.unwrap(),
        other => panic!("expected MRTR roots request, got {other:?}"),
    };
    let mut changed = request("search_context", json!({"query": "changed"}));
    changed.request_state = Some(state.clone());
    assert!(client.call_tool_once(changed).await.is_err());
    let mut replay = request("get_workspace", json!({}));
    replay.request_state = Some(state);
    assert!(client.call_tool_once(replay).await.is_err());
    assert_eq!(owner(&call(&client, "get_workspace", json!({"workspace": "assign"})).await), "assign");
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_broken_or_unregistered_workspace_does_not_poison_healthy_reader() {
    let fixture = Fixture::new();
    let client = connect((), &fixture.home, &fixture.home).await;
    call(&client, "search_context", json!({"workspace": "assign", "query": "product_only_identity"})).await;
    fs::write(fixture.codanna.join(".codanna/index/index.meta"), "corrupt fixture").unwrap();
    assert!(client.call_tool(request("search_context", json!({"workspace": "codanna", "query": "identity"}))).await.is_err());
    assert_eq!(owner(&call(&client, "search_context", json!({"workspace": "assign", "query": "product_only_identity"})).await), "assign");
    let registry = codanna::init::workspaces::WorkspaceRegistry::new(fixture.home.join(".codanna/projects.json"));
    registry.remove("assign").unwrap();
    assert!(client.call_tool(request("search_context", json!({"workspace": "assign", "query": "product_only_identity"}))).await.is_err());
    client.cancel().await.unwrap();
}
