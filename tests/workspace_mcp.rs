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
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("NO_COLOR", "1")
        .current_dir(cwd);
    for key in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

fn run_cli(home: &Path, cwd: &Path, args: &[&str]) {
    let out = tempfile::NamedTempFile::new().unwrap();
    let err = tempfile::NamedTempFile::new().unwrap();
    let mut command = command(home, cwd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(out.reopen().unwrap())
        .stderr(err.reopen().unwrap());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "stdout={} stderr={}",
                fs::read_to_string(out.path()).unwrap(),
                fs::read_to_string(err.path()).unwrap()
            );
            return;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("fixture CLI exceeded deadline: {args:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn checkout(root: &Path, symbol: &str) {
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        format!("pub fn {symbol}() -> u32 {{ 1 }}\n"),
    )
    .unwrap();
}

fn configure(root: &Path) {
    fs::create_dir_all(root.join(".codanna")).unwrap();
    let mut settings = codanna::Settings::default();
    settings.semantic_search.enabled = false;
    settings.indexing.parallelism = 1;
    settings.indexing.indexed_paths = vec![PathBuf::from(".")];
    fs::write(
        root.join(".codanna/settings.toml"),
        toml::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();
    fs::write(root.join(".codannaignore"), ".codanna/\n.git/\n").unwrap();
}

struct Fixture {
    _temporary: TempDir,
    home: PathBuf,
    primary: PathBuf,
    unrelated: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let home = temporary.path().join("home");
        let primary = temporary.path().join("workspace-a");
        let unrelated = temporary.path().join("workspace-b");
        fs::create_dir_all(&home).unwrap();
        checkout(&primary.join("repo-a"), "primary_only_identity");
        checkout(&primary.join("repo-b"), "primary_only_identity");
        checkout(&unrelated, "unrelated_only_identity");
        configure(&primary);
        configure(&unrelated);
        run_cli(&home, &primary, &["index", "--no-progress"]);
        run_cli(&home, &unrelated, &["index", "--no-progress"]);
        Self {
            _temporary: temporary,
            home,
            primary,
            unrelated,
        }
    }
}

async fn connect<C: ClientHandler>(
    handler: C,
    home: &Path,
    cwd: &Path,
) -> RunningService<RoleClient, C> {
    let mut process = command(home, cwd);
    process.args(["workspace", "serve"]).stderr(Stdio::null());
    let mut process = tokio::process::Command::from(process);
    process.kill_on_drop(true);
    tokio::time::timeout(
        Duration::from_secs(15),
        handler.serve(TokioChildProcess::new(process).unwrap()),
    )
    .await
    .expect("MCP handshake deadline")
    .expect("MCP handshake")
}

fn request(name: &str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone())
}

async fn call<C: ClientHandler>(
    client: &RunningService<RoleClient, C>,
    name: &str,
    args: Value,
) -> CallToolResult {
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        client.call_tool(request(name, args)),
    )
    .await
    .expect("tool deadline")
    .expect("tool call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result
}

fn owner(result: &CallToolResult) -> &str {
    result.structured_content.as_ref().unwrap()["workspace"]["name"]
        .as_str()
        .unwrap()
}

fn rendered(result: &CallToolResult) -> String {
    serde_json::to_string(result).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_handshake_and_catalogue_from_home_do_not_create_state() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let client = connect((), &home, &home).await;
    let tools = client.list_tools(Default::default()).await.unwrap();
    assert!(
        tools
            .tools
            .iter()
            .any(|tool| tool.name == "list_workspaces")
    );
    let schema = serde_json::to_value(
        tools
            .tools
            .iter()
            .find(|tool| tool.name == "search_context")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        schema["inputSchema"]["properties"]["workspace"]["type"],
        "string"
    );
    assert_eq!(
        schema["inputSchema"]["properties"]["project_path"]["type"],
        "string"
    );
    let listed = call(&client, "list_workspaces", json!({})).await;
    assert_eq!(listed.structured_content.unwrap()["workspaces"], json!([]));
    assert!(
        client
            .call_tool(request("get_workspace", json!({})))
            .await
            .is_err()
    );
    assert!(
        client
            .call_tool(request("get_workspace", json!({"workspace": "missing"})))
            .await
            .is_err()
    );
    assert!(!home.join(".codanna").exists());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_one_connection_queries_independent_workspaces() {
    let fixture = Fixture::new();
    let client = connect((), &fixture.home, &fixture.home).await;
    let listed = call(&client, "list_workspaces", json!({})).await;
    assert_eq!(
        listed.structured_content.unwrap()["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let before = call(
        &client,
        "get_workspace",
        json!({"workspace": "workspace-a"}),
    )
    .await;
    assert_eq!(
        before.structured_content.unwrap()["result"]["worker_loaded"],
        false
    );
    let (primary, unrelated) = tokio::join!(
        call(
            &client,
            "search_context",
            json!({"workspace": "workspace-a", "query": "primary_only_identity"})
        ),
        call(
            &client,
            "search_context",
            json!({"workspace": "workspace-b", "query": "unrelated_only_identity"})
        ),
    );
    assert_eq!(owner(&primary), "workspace-a");
    assert_eq!(owner(&unrelated), "workspace-b");
    assert!(rendered(&primary).contains("repo-a"));
    assert!(rendered(&primary).contains("repo-b"));
    assert!(!rendered(&primary).contains("unrelated_only_identity"));
    assert!(rendered(&unrelated).contains("unrelated_only_identity"));
    assert!(!rendered(&unrelated).contains("primary_only_identity"));
    let after = call(
        &client,
        "get_workspace",
        json!({"workspace": "workspace-a"}),
    )
    .await;
    assert_eq!(
        after.structured_content.unwrap()["result"]["worker_loaded"],
        true
    );
    let path = call(&client, "search_context", json!({"project_path": fixture.primary.join("repo-b/src"), "query": "primary_only_identity"})).await;
    assert_eq!(owner(&path), "workspace-a");
    assert!(
        client
            .call_tool(request(
                "get_workspace",
                json!({"workspace": "workspace-a", "project_path": fixture.unrelated})
            ))
            .await
            .is_err()
    );
    assert!(
        client
            .peer()
            .send_request(ClientRequest::CustomRequest(CustomRequest::new(
                "requests/codanna/force-reindex",
                None
            )))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_explicit_override_never_changes_the_connection_default() {
    let fixture = Fixture::new();
    let client = connect((), &fixture.home, &fixture.primary.join("repo-b/src")).await;
    assert_eq!(
        owner(&call(&client, "get_workspace", json!({})).await),
        "workspace-a"
    );
    assert_eq!(
        owner(
            &call(
                &client,
                "get_workspace",
                json!({"workspace": "workspace-b"})
            )
            .await
        ),
        "workspace-b"
    );
    assert_eq!(
        owner(&call(&client, "get_workspace", json!({})).await),
        "workspace-a"
    );
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
        )
        .with_protocol_version(self.version.clone())
    }

    async fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, ErrorData> {
        let roots: Vec<_> = self
            .roots
            .read()
            .unwrap()
            .iter()
            .map(|root| json!({"uri": reqwest::Url::from_file_path(root).unwrap().to_string()}))
            .collect();
        serde_json::from_value(json!({"roots": roots}))
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_legacy_and_mrtr_roots_select_workspaces_without_paths_in_config() {
    let fixture = Fixture::new();
    for version in ["2025-11-25", "2026-07-28"] {
        let roots = Arc::new(RwLock::new(vec![
            fixture.primary.join("repo-a"),
            fixture.primary.join("repo-b"),
        ]));
        let handler = RootsClient {
            roots: roots.clone(),
            version: serde_json::from_value(json!(version)).unwrap(),
        };
        let client = connect(handler, &fixture.home, &fixture.home).await;
        assert_eq!(
            owner(&call(&client, "get_workspace", json!({})).await),
            "workspace-a"
        );
        *roots.write().unwrap() = vec![fixture.unrelated.clone()];
        assert_eq!(
            owner(&call(&client, "get_workspace", json!({})).await),
            "workspace-b"
        );
        *roots.write().unwrap() = vec![fixture.primary.clone(), fixture.unrelated.clone()];
        assert!(
            client
                .call_tool(request("get_workspace", json!({})))
                .await
                .is_err()
        );
        assert_eq!(
            owner(
                &call(
                    &client,
                    "get_workspace",
                    json!({"workspace": "workspace-a"})
                )
                .await
            ),
            "workspace-a"
        );
        let unknown = fixture.home.join("unregistered");
        fs::create_dir_all(unknown.join(".git")).unwrap();
        *roots.write().unwrap() = vec![fixture.primary.clone(), unknown.clone()];
        assert!(
            client
                .call_tool(request("get_workspace", json!({})))
                .await
                .is_err()
        );
        assert!(!unknown.join(".codanna").exists());
        client.cancel().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_root_continuations_are_bound_and_single_use() {
    let fixture = Fixture::new();
    let handler = RootsClient {
        roots: Arc::new(RwLock::new(vec![fixture.primary.clone()])),
        version: ProtocolVersion::V_2026_07_28,
    };
    let client = connect(handler, &fixture.home, &fixture.home).await;
    let response = client
        .call_tool_once(request("get_workspace", json!({})))
        .await
        .unwrap();
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
    assert_eq!(
        owner(
            &call(
                &client,
                "get_workspace",
                json!({"workspace": "workspace-a"})
            )
            .await
        ),
        "workspace-a"
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_broken_or_unregistered_workspace_does_not_poison_healthy_reader() {
    let fixture = Fixture::new();
    let client = connect((), &fixture.home, &fixture.home).await;
    call(
        &client,
        "search_context",
        json!({"workspace": "workspace-a", "query": "primary_only_identity"}),
    )
    .await;
    fs::write(
        fixture.unrelated.join(".codanna/index/index.meta"),
        "corrupt fixture",
    )
    .unwrap();
    assert!(
        client
            .call_tool(request(
                "search_context",
                json!({"workspace": "workspace-b", "query": "identity"})
            ))
            .await
            .is_err()
    );
    assert_eq!(
        owner(
            &call(
                &client,
                "search_context",
                json!({"workspace": "workspace-a", "query": "primary_only_identity"})
            )
            .await
        ),
        "workspace-a"
    );
    let registry = codanna::init::workspaces::WorkspaceRegistry::new(
        fixture.home.join(".codanna/projects.json"),
    );
    registry.remove("workspace-a").unwrap();
    assert!(
        client
            .call_tool(request(
                "search_context",
                json!({"workspace": "workspace-a", "query": "primary_only_identity"})
            ))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_arbitrary_renamed_single_repository_workspaces_stay_isolated() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let registry =
        codanna::init::workspaces::WorkspaceRegistry::new(home.join(".codanna/projects.json"));
    let mut workspaces = Vec::new();
    for index in 0..2 {
        let directory = format!("checkout-{index}");
        let root = temp.path().join(&directory);
        let symbol = format!("isolated_symbol_{index}");
        checkout(&root, &symbol);
        configure(&root);
        run_cli(&home, &root, &["index", "--no-progress"]);
        let original = registry.get(&directory).unwrap();
        let alias = format!("renamed_{index}_73");
        let renamed = registry.rename(original.id.as_str(), &alias).unwrap();
        assert_eq!(original.id, renamed.id);
        workspaces.push((renamed, root, symbol, directory));
    }
    let client = connect((), &home, &home).await;
    for (workspace, root, symbol, old_alias) in &workspaces {
        let result = call(
            &client,
            "search_context",
            json!({"workspace": &workspace.name, "query": symbol}),
        )
        .await;
        assert_eq!(owner(&result), workspace.name);
        assert_eq!(
            result.structured_content.as_ref().unwrap()["workspace"]["id"],
            workspace.id.as_str()
        );
        assert!(rendered(&result).contains(symbol));
        for (other, _, other_symbol, _) in &workspaces {
            if other.id != workspace.id {
                assert!(!rendered(&result).contains(other_symbol));
            }
        }
        let by_id = call(
            &client,
            "get_workspace",
            json!({"workspace": workspace.id.as_str()}),
        )
        .await;
        assert_eq!(owner(&by_id), workspace.name);
        let by_path = call(
            &client,
            "get_workspace",
            json!({"project_path": root.join("src")}),
        )
        .await;
        assert_eq!(owner(&by_path), workspace.name);
        assert!(
            client
                .call_tool(request("get_workspace", json!({"workspace": old_alias})))
                .await
                .is_err()
        );
    }
    client.cancel().await.unwrap();
}
