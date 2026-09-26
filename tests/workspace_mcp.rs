//! Real MCP witnesses. Synthetic projects, temporary homes, no credentials.
#![cfg(unix)]
use rmcp::model::*;
use rmcp::service::{RequestContext, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
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
    let mut child = command(home, cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out.reopen().unwrap())
        .stderr(err.reopen().unwrap())
        .spawn()
        .unwrap();
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
            panic!("CLI fixture timeout: {args:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn source(root: &Path, marker: &str) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        format!("/// {marker}\npub fn shared_symbol() -> u32 {{ 1 }}\n"),
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
    a: PathBuf,
    b: PathBuf,
}
impl Fixture {
    fn fresh() -> Self {
        let temporary = TempDir::new().unwrap();
        let home = temporary.path().join("home");
        fs::create_dir(&home).unwrap();
        let a = temporary.path().join("projecta");
        let b = temporary.path().join("projectb");
        source(&a, "ONLY_PROJECT_A");
        source(&b, "ONLY_PROJECT_B");
        Self {
            _temporary: temporary,
            home,
            a,
            b,
        }
    }
    fn indexed() -> Self {
        let fixture = Self::fresh();
        for root in [&fixture.a, &fixture.b] {
            configure(root);
            run_cli(&fixture.home, root, &["index", "--no-progress"]);
        }
        fixture
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
    .expect("handshake deadline")
    .expect("handshake")
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
async fn ready<C: ClientHandler>(
    client: &RunningService<RoleClient, C>,
    args: Value,
) -> CallToolResult {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let result = call(client, "search_context", args.clone()).await;
            if result
                .structured_content
                .as_ref()
                .is_none_or(|value| value["result"]["ready"] != false)
            {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("automatic first-index deadline")
}
fn owner(result: &CallToolResult) -> &str {
    result.structured_content.as_ref().unwrap()["workspace"]["name"]
        .as_str()
        .unwrap()
}
fn rendered(result: &CallToolResult) -> String {
    serde_json::to_string(result).unwrap()
}
fn assert_only(result: &CallToolResult, yes: &str, no: &str) {
    let text = rendered(result);
    assert!(text.contains(yes), "{text}");
    assert!(!text.contains(no), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_handshake_and_catalogue_from_home_do_not_create_state() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let client = connect((), &home, &home).await;
    let tools = client.list_tools(Default::default()).await.unwrap();
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
    assert_eq!(
        call(&client, "list_workspaces", json!({}))
            .await
            .structured_content
            .unwrap()["workspaces"],
        json!([])
    );
    assert!(
        client
            .call_tool(request("get_workspace", json!({})))
            .await
            .is_err()
    );
    assert!(
        client
            .call_tool(request("get_workspace", json!({"workspace":"missing"})))
            .await
            .is_err()
    );
    assert!(!home.join(".codanna").exists());
    client.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_one_connection_queries_independent_workspaces() {
    let fixture = Fixture::indexed();
    let client = connect((), &fixture.home, &fixture.home).await;
    assert_eq!(
        call(&client, "list_workspaces", json!({}))
            .await
            .structured_content
            .unwrap()["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let before = call(&client, "get_workspace", json!({"workspace":"projecta"})).await;
    assert_eq!(
        before.structured_content.unwrap()["result"]["worker_loaded"],
        false
    );
    let (a, b) = tokio::join!(
        ready(
            &client,
            json!({"workspace":"projecta","query":"shared_symbol"})
        ),
        ready(
            &client,
            json!({"workspace":"projectb","query":"shared_symbol"})
        )
    );
    assert_eq!(owner(&a), "projecta");
    assert_eq!(owner(&b), "projectb");
    assert_only(&a, "ONLY_PROJECT_A", "ONLY_PROJECT_B");
    assert_only(&b, "ONLY_PROJECT_B", "ONLY_PROJECT_A");
    assert_eq!(
        call(&client, "get_workspace", json!({"workspace":"projecta"}))
            .await
            .structured_content
            .unwrap()["result"]["worker_loaded"],
        true
    );
    assert_eq!(
        owner(
            &ready(
                &client,
                json!({"project_path":fixture.a,"query":"shared_symbol"})
            )
            .await
        ),
        "projecta"
    );
    assert!(
        client
            .call_tool(request(
                "get_workspace",
                json!({"workspace":"projecta","project_path":fixture.b})
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
    let fixture = Fixture::indexed();
    let client = connect((), &fixture.home, &fixture.a).await;
    assert_eq!(
        owner(&call(&client, "get_workspace", json!({})).await),
        "projecta"
    );
    assert_eq!(
        owner(&call(&client, "get_workspace", json!({"workspace":"projectb"})).await),
        "projectb"
    );
    assert_eq!(
        owner(&call(&client, "get_workspace", json!({})).await),
        "projecta"
    );
    client.cancel().await.unwrap();
}
#[derive(Clone)]
struct RootsClient {
    roots: Arc<RwLock<Vec<PathBuf>>>,
    version: ProtocolVersion,
    notify: bool,
    calls: Arc<AtomicUsize>,
}
#[allow(deprecated)]
impl ClientHandler for RootsClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            serde_json::from_value(json!({"roots":{"listChanged":self.notify}})).unwrap(),
            Implementation::new("workspace-fixture", "1"),
        )
        .with_protocol_version(self.version.clone())
    }
    async fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, ErrorData> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let roots: Vec<_> = self
            .roots
            .read()
            .unwrap()
            .iter()
            .map(|root| json!({"uri":reqwest::Url::from_file_path(root).unwrap().to_string()}))
            .collect();
        serde_json::from_value(json!({"roots":roots}))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    }
}
fn roots_client(paths: Vec<PathBuf>, version: &str, notify: bool) -> RootsClient {
    RootsClient {
        roots: Arc::new(RwLock::new(paths)),
        version: serde_json::from_value(json!(version)).unwrap(),
        notify,
        calls: Arc::new(AtomicUsize::new(0)),
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_legacy_and_mrtr_roots_select_workspaces_without_paths_in_config() {
    let fixture = Fixture::indexed();
    for version in ["2025-11-25", "2026-07-28"] {
        let handler = roots_client(vec![fixture.a.clone()], version, false);
        let roots = handler.roots.clone();
        let client = connect(handler, &fixture.home, &fixture.home).await;
        assert_eq!(
            owner(&call(&client, "get_workspace", json!({})).await),
            "projecta"
        );
        *roots.write().unwrap() = vec![fixture.b.clone()];
        assert_eq!(
            owner(&call(&client, "get_workspace", json!({})).await),
            "projectb"
        );
        *roots.write().unwrap() = vec![fixture.a.clone(), fixture.b.clone()];
        assert!(
            client
                .call_tool(request("get_workspace", json!({})))
                .await
                .is_err()
        );
        assert_eq!(
            owner(&call(&client, "get_workspace", json!({"workspace":"projecta"})).await),
            "projecta"
        );
        let unknown = fixture.home.join("unregistered");
        fs::create_dir_all(&unknown).unwrap();
        *roots.write().unwrap() = vec![fixture.a.clone(), unknown.clone()];
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
    let fixture = Fixture::indexed();
    let client = connect(
        roots_client(vec![fixture.a.clone()], "2026-07-28", false),
        &fixture.home,
        &fixture.home,
    )
    .await;
    let response = client
        .call_tool_once(request("get_workspace", json!({})))
        .await
        .unwrap();
    let state = match response {
        CallToolResponse::InputRequired(result) => result.request_state.unwrap(),
        other => panic!("expected roots request: {other:?}"),
    };
    let mut changed = request("search_context", json!({"query":"changed"}));
    changed.request_state = Some(state.clone());
    assert!(client.call_tool_once(changed).await.is_err());
    let mut replay = request("get_workspace", json!({}));
    replay.request_state = Some(state);
    assert!(client.call_tool_once(replay).await.is_err());
    assert_eq!(
        owner(&call(&client, "get_workspace", json!({"workspace":"projecta"})).await),
        "projecta"
    );
    client.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_broken_or_unregistered_workspace_does_not_poison_healthy_reader() {
    let fixture = Fixture::indexed();
    let client = connect((), &fixture.home, &fixture.home).await;
    ready(
        &client,
        json!({"workspace":"projecta","query":"shared_symbol"}),
    )
    .await;
    fs::write(
        fixture.b.join(".codanna/index/index.meta"),
        "corrupt fixture",
    )
    .unwrap();
    assert!(
        client
            .call_tool(request(
                "search_context",
                json!({"workspace":"projectb","query":"shared_symbol"})
            ))
            .await
            .is_err()
    );
    assert_eq!(
        owner(
            &ready(
                &client,
                json!({"workspace":"projecta","query":"shared_symbol"})
            )
            .await
        ),
        "projecta"
    );
    let registry = codanna::init::workspaces::WorkspaceRegistry::new(
        fixture.home.join(".codanna/projects.json"),
    );
    registry.remove("projecta").unwrap();
    assert!(
        client
            .call_tool(request(
                "search_context",
                json!({"workspace":"projecta","query":"shared_symbol"})
            ))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_arbitrary_renamed_single_repository_workspaces_keep_identity() {
    let fixture = Fixture::indexed();
    let registry = codanna::init::workspaces::WorkspaceRegistry::new(
        fixture.home.join(".codanna/projects.json"),
    );
    let a = registry.rename("projecta", "renamed-one").unwrap();
    let b = registry.rename("projectb", "renamed-two").unwrap();
    let client = connect((), &fixture.home, &fixture.home).await;
    for (workspace, marker, other) in [
        (a, "ONLY_PROJECT_A", "ONLY_PROJECT_B"),
        (b, "ONLY_PROJECT_B", "ONLY_PROJECT_A"),
    ] {
        for selector in [
            json!({"workspace":workspace.name,"query":"shared_symbol"}),
            json!({"workspace":workspace.id.as_str(),"query":"shared_symbol"}),
            json!({"project_path":workspace.root,"query":"shared_symbol"}),
        ] {
            let result = ready(&client, selector).await;
            assert_only(&result, marker, other);
            assert_eq!(
                result.structured_content.unwrap()["workspace"]["id"],
                workspace.id.as_str()
            );
        }
    }
    client.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_fresh_plain_projects_bootstrap_without_cli_or_configuration() {
    let fixture = Fixture::fresh();
    assert!(!fixture.a.join(".codanna").exists());
    assert!(!fixture.b.join(".codanna").exists());
    let (a, b) = tokio::join!(
        connect((), &fixture.home, &fixture.a),
        connect((), &fixture.home, &fixture.b)
    );
    let (result_a, result_b) = tokio::join!(
        ready(&a, json!({"query":"shared_symbol"})),
        ready(&b, json!({"query":"shared_symbol"}))
    );
    assert_only(&result_a, "ONLY_PROJECT_A", "ONLY_PROJECT_B");
    assert_only(&result_b, "ONLY_PROJECT_B", "ONLY_PROJECT_A");
    assert_ne!(
        result_a.structured_content.unwrap()["workspace"]["id"],
        result_b.structured_content.unwrap()["workspace"]["id"]
    );
    for root in [&fixture.a, &fixture.b] {
        assert!(root.join(".codanna/index/index.meta").is_file());
        let settings: codanna::Settings =
            toml::from_str(&fs::read_to_string(root.join(".codanna/settings.toml")).unwrap())
                .unwrap();
        assert!(!settings.semantic_search.enabled);
    }
    assert!(!fixture.home.join(".codanna/models").exists());
    assert!(!fixture.home.join(".cache").exists());
    a.cancel().await.unwrap();
    b.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_broad_parent_cannot_capture_fresh_plain_child() {
    let fixture = Fixture::fresh();
    let parent = fixture.a.parent().unwrap();
    configure(parent);
    run_cli(&fixture.home, parent, &["index", "--no-progress"]);
    let client = connect((), &fixture.home, &fixture.b).await;
    let result = ready(&client, json!({"query":"shared_symbol"})).await;
    assert_only(&result, "ONLY_PROJECT_B", "ONLY_PROJECT_A");
    assert_eq!(owner(&result), "projectb");
    client.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_empty_project_accepts_first_source_later() {
    let fixture = Fixture::fresh();
    let empty = fixture.a.parent().unwrap().join("empty");
    fs::create_dir(&empty).unwrap();
    let client = connect((), &fixture.home, &empty).await;
    let first = call(&client, "search_context", json!({"query":"shared_symbol"})).await;
    assert_eq!(first.structured_content.unwrap()["result"]["ready"], false);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    source(&empty, "NEW_SOURCE");
    let result = ready(&client, json!({"query":"shared_symbol"})).await;
    assert!(rendered(&result).contains("NEW_SOURCE"));
    client.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_bad_arguments_keep_healthy_reader_and_tool_error() {
    let fixture = Fixture::indexed();
    let client = connect((), &fixture.home, &fixture.b).await;
    ready(&client, json!({"query":"shared_symbol"})).await;
    // The generated tool router returns deserialization failures as tool-error
    // content, not JSON-RPC errors. Preserve that wire contract unchanged.
    let error = client
        .call_tool(request(
            "search_symbols",
            json!({"query":"shared_symbol","not_a_parameter":true}),
        ))
        .await
        .unwrap();
    assert_eq!(error.is_error, Some(true));
    assert!(rendered(&error).contains("unknown field `not_a_parameter`"));
    let result = ready(&client, json!({"query":"shared_symbol"})).await;
    assert_only(&result, "ONLY_PROJECT_B", "ONLY_PROJECT_A");
    assert_eq!(
        call(&client, "get_workspace", json!({}))
            .await
            .structured_content
            .unwrap()["result"]["worker_loaded"],
        true
    );
    client.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_roots_cache_invalidates_on_notification() {
    let fixture = Fixture::indexed();
    let handler = roots_client(vec![fixture.a.clone()], "2025-11-25", true);
    let roots = handler.roots.clone();
    let count = handler.calls.clone();
    let client = connect(handler, &fixture.home, &fixture.home).await;
    call(&client, "get_workspace", json!({})).await;
    call(&client, "get_workspace", json!({})).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    *roots.write().unwrap() = vec![fixture.b.clone()];
    let notification: ClientNotification =
        serde_json::from_value(json!({"method":"notifications/roots/list_changed"})).unwrap();
    client.send_notification(notification).await.unwrap();
    assert_eq!(
        owner(&call(&client, "get_workspace", json!({})).await),
        "projectb"
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    client.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_two_fresh_sessions_share_safe_initial_publication() {
    let fixture = Fixture::fresh();
    let (first, second) = tokio::join!(
        connect((), &fixture.home, &fixture.a),
        connect((), &fixture.home, &fixture.a)
    );
    let (a, b) = tokio::join!(
        ready(&first, json!({"query":"shared_symbol"})),
        ready(&second, json!({"query":"shared_symbol"}))
    );
    assert_only(&a, "ONLY_PROJECT_A", "ONLY_PROJECT_B");
    assert_only(&b, "ONLY_PROJECT_A", "ONLY_PROJECT_B");
    assert_eq!(
        a.structured_content.unwrap()["workspace"]["id"],
        b.structured_content.unwrap()["workspace"]["id"]
    );
    assert!(!fixture.b.join(".codanna").exists());
    let metadata: Value =
        serde_json::from_slice(&fs::read(fixture.a.join(".codanna/index/index.meta")).unwrap())
            .unwrap();
    assert_eq!(metadata["file_count"], 1);
    first.cancel().await.unwrap();
    second.cancel().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_lexical_tools_do_not_call_configured_embedding_endpoint() {
    let fixture = Fixture::indexed();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let stop = tokio_util::sync::CancellationToken::new();
    let cancelled = stop.clone();
    let endpoint = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancelled.cancelled() => break,
                connection = listener.accept() => { let (stream, _) = connection.unwrap(); observed.fetch_add(1, Ordering::SeqCst); drop(stream); }
            }
        }
    });
    let path = fixture.b.join(".codanna/settings.toml");
    let mut settings: codanna::Settings =
        toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    settings.semantic_search.enabled = true;
    settings.semantic_search.remote_url = Some(format!("http://{address}/v1/embeddings"));
    fs::write(path, toml::to_string_pretty(&settings).unwrap()).unwrap();
    let client = connect((), &fixture.home, &fixture.b).await;
    call(&client, "get_index_info", json!({})).await;
    call(&client, "find_symbol", json!({"name":"shared_symbol"})).await;
    ready(&client, json!({"query":"shared_symbol"})).await;
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert!(!fixture.home.join(".codanna/models").exists());
    client.cancel().await.unwrap();
    stop.cancel();
    endpoint.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_fresh_roots_switch_without_manual_setup() {
    let fixture = Fixture::fresh();
    let handler = roots_client(vec![fixture.a.clone()], "2025-11-25", true);
    let roots = handler.roots.clone();
    let client = connect(handler, &fixture.home, &fixture.home).await;
    let a = ready(&client, json!({"query":"shared_symbol"})).await;
    assert_only(&a, "ONLY_PROJECT_A", "ONLY_PROJECT_B");
    assert!(!fixture.b.join(".codanna").exists());

    *roots.write().unwrap() = vec![fixture.b.clone()];
    let notification: ClientNotification =
        serde_json::from_value(json!({"method":"notifications/roots/list_changed"})).unwrap();
    client.send_notification(notification).await.unwrap();
    let b = ready(&client, json!({"query":"shared_symbol"})).await;
    assert_only(&b, "ONLY_PROJECT_B", "ONLY_PROJECT_A");
    assert_ne!(
        a.structured_content.unwrap()["workspace"]["id"],
        b.structured_content.unwrap()["workspace"]["id"]
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_new_document_store_invalidates_cached_absence() {
    let fixture = Fixture::indexed();
    let config = fixture.a.join(".codanna/settings.toml");
    let mut settings: codanna::Settings =
        toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
    settings.documents.enabled = true;
    fs::write(&config, toml::to_string_pretty(&settings).unwrap()).unwrap();
    let client = connect((), &fixture.home, &fixture.a).await;
    ready(&client, json!({"query":"shared_symbol"})).await;

    // First query cached the missing store. Introduce an out-of-scope store;
    // refresh must revalidate it before any model initialization or search.
    let documents = fixture.a.join(".codanna/index/documents");
    fs::create_dir_all(documents.join("tantivy")).unwrap();
    fs::write(documents.join("tantivy/meta.json"), "{}").unwrap();
    let foreign = fixture.b.join("private.md");
    let state = json!({"file_states": {foreign.to_str().unwrap(): {}}});
    fs::write(documents.join("state.json"), state.to_string()).unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Err(rmcp::service::ServiceError::McpError(error)) = client
                .call_tool(request("search_context", json!({"query":"shared_symbol"})))
                .await
            {
                assert!(error.message.contains("workspace"), "{error:?}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("new document state must invalidate a previously missing store");

    // An auxiliary-index refusal must not poison independent code-only tools.
    let code = call(&client, "find_symbol", json!({"name":"shared_symbol"})).await;
    assert_only(&code, "ONLY_PROJECT_A", "ONLY_PROJECT_B");
    assert!(!fixture.home.join(".codanna/models").exists());
    assert!(!fixture.home.join(".cache").exists());
    client.cancel().await.unwrap();
}

/// A notification is not an acknowledgement. The very next implicit query must
/// use the new roots even when a second client is querying another context.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_queued_root_changes_preserve_session_isolation() {
    for version in ["2025-11-25", "2026-07-28"] {
        let fixture = Fixture::fresh();
        let changing = roots_client(vec![fixture.a.clone()], version, true);
        let roots = changing.roots.clone();
        let calls = changing.calls.clone();
        let steady = roots_client(vec![fixture.b.clone()], version, true);
        let steady_calls = steady.calls.clone();
        let (first, second) = tokio::join!(
            connect(changing, &fixture.home, &fixture.home),
            connect(steady, &fixture.home, &fixture.home)
        );
        let (a, b) = tokio::join!(
            ready(&first, json!({"query":"shared_symbol"})),
            ready(&second, json!({"query":"shared_symbol"}))
        );
        assert_only(&a, "ONLY_PROJECT_A", "ONLY_PROJECT_B");
        assert_only(&b, "ONLY_PROJECT_B", "ONLY_PROJECT_A");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(steady_calls.load(Ordering::SeqCst), 1);

        for switch in 0..8 {
            let (path, expected, forbidden) = if switch % 2 == 0 {
                (&fixture.b, "ONLY_PROJECT_B", "ONLY_PROJECT_A")
            } else {
                (&fixture.a, "ONLY_PROJECT_A", "ONLY_PROJECT_B")
            };
            *roots.write().unwrap() = vec![path.clone()];
            let notification: ClientNotification =
                serde_json::from_value(json!({"method":"notifications/roots/list_changed"}))
                    .unwrap();
            first.send_notification(notification).await.unwrap();
            // No sleep, acknowledgement call, or retry of a wrong-workspace
            // response. `ready` waits only for explicitly incomplete indexing.
            let (selected, independent) = tokio::join!(
                ready(&first, json!({"query":"shared_symbol"})),
                ready(&second, json!({"query":"shared_symbol"}))
            );
            assert_only(&selected, expected, forbidden);
            assert_only(&independent, "ONLY_PROJECT_B", "ONLY_PROJECT_A");
            assert_eq!(calls.load(Ordering::SeqCst), switch + 2);
            assert_eq!(steady_calls.load(Ordering::SeqCst), 1);
        }
        tokio::time::timeout(Duration::from_secs(15), first.cancel())
            .await
            .expect("first session shutdown deadline")
            .unwrap();
        let independent = ready(&second, json!({"query":"shared_symbol"})).await;
        assert_only(&independent, "ONLY_PROJECT_B", "ONLY_PROJECT_A");
        tokio::time::timeout(Duration::from_secs(15), second.cancel())
            .await
            .expect("second session shutdown deadline")
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_mcp_root_change_rejects_queued_old_continuation() {
    let fixture = Fixture::fresh();
    let handler = roots_client(vec![fixture.a.clone()], "2026-07-28", true);
    let roots = handler.roots.clone();
    let client = connect(handler, &fixture.home, &fixture.home).await;
    let response = tokio::time::timeout(
        Duration::from_secs(20),
        client.call_tool_once(request("get_workspace", json!({}))),
    )
    .await
    .unwrap()
    .unwrap();
    let state = match response {
        CallToolResponse::InputRequired(result) => result.request_state.unwrap(),
        other => panic!("expected roots request: {other:?}"),
    };
    *roots.write().unwrap() = vec![fixture.b.clone()];
    let notification: ClientNotification =
        serde_json::from_value(json!({"method":"notifications/roots/list_changed"})).unwrap();
    client.send_notification(notification).await.unwrap();
    let mut stale = request("get_workspace", json!({}));
    stale.request_state = Some(state);
    stale.input_responses = Some(
        serde_json::from_value(json!({
            "codanna-workspace-roots": {"roots": [{
                "uri": reqwest::Url::from_file_path(&fixture.a).unwrap().to_string()
            }]}
        }))
        .unwrap(),
    );
    let error = tokio::time::timeout(Duration::from_secs(20), client.call_tool_once(stale))
        .await
        .unwrap()
        .unwrap_err();
    match error {
        rmcp::service::ServiceError::McpError(error) => {
            assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
            assert!(error.message.contains("roots changed"), "{error:?}");
        }
        other => panic!("expected stale-scope error: {other:?}"),
    }
    assert!(!fixture.a.join(".codanna").exists());
    assert!(!fixture.b.join(".codanna").exists());
    assert_eq!(
        owner(&call(&client, "get_workspace", json!({})).await),
        "projectb"
    );
    assert!(!fixture.a.join(".codanna").exists());
    tokio::time::timeout(Duration::from_secs(15), client.cancel())
        .await
        .expect("session shutdown deadline")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ticket_code_fusion_workspace_router_preserves_structured_evidence() {
    let fixture = Fixture::indexed();
    let client = connect((), &fixture.home, &fixture.a).await;
    let tools = client.list_tools(Default::default()).await.unwrap();
    assert!(
        tools
            .tools
            .iter()
            .any(|tool| tool.name == "search_ticket_context")
    );
    ready(&client, json!({"query": "shared_symbol"})).await;
    let response = call(
        &client,
        "search_ticket_context",
        json!({
            "query": "shared_symbol", "code_path_prefix": "src", "code_limit": 1,
        }),
    )
    .await;
    let wrapper = response.structured_content.unwrap();
    let data = &wrapper["result"];
    assert_eq!(data["retrieval"], "ticket-rank-fusion-v1", "{wrapper}");
    assert_eq!(data["code"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(data["code"]["semantic_status"], "not_requested");
    assert_eq!(data["graph"]["query_status"], "not_run");
    client.cancel().await.unwrap();
}

#[test]
fn ticket_code_fusion_cli_json_keeps_evidence_and_rejects_invalid_limits() {
    let fixture = Fixture::indexed();
    let output = command(&fixture.home, &fixture.a)
        .args([
            "mcp",
            "search_ticket_context",
            "query:shared_symbol",
            "code_limit:1",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        envelope["data"]["retrieval"], "ticket-rank-fusion-v1",
        "{envelope}"
    );
    assert_eq!(
        envelope["data"]["code"]["items"].as_array().unwrap().len(),
        1
    );
    let invalid = command(&fixture.home, &fixture.a)
        .args([
            "mcp",
            "search_ticket_context",
            "query:shared_symbol",
            "code_limit:0",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
}
