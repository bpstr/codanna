//! Real stdio/native-watch/process-lock witnesses. No models or remote services.
#![cfg(unix)]
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

type Client = RunningService<RoleClient, ()>;
fn process(home: &Path, root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codanna"));
    command.env_clear().env("HOME", home).env("XDG_DATA_HOME",home.join("data"))
        .env("PATH", "/usr/bin:/bin").current_dir(root).stdin(Stdio::null()).kill_on_drop(true);
    for key in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        if let Some(value) = std::env::var_os(key) { command.env(key, value); }
    }
    command
}
async fn connect(home: &Path, root: &Path) -> Client {
    let mut command = process(home, root);
    command.args(["workspace", "serve"]).stderr(Stdio::null());
    tokio::time::timeout(Duration::from_secs(15), ().serve(TokioChildProcess::new(command).unwrap())).await.unwrap().unwrap()
}
async fn call(client: &Client, name: &str, arguments: Value) -> CallToolResult {
    client.call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments.as_object().unwrap().clone())).await.unwrap()
}
async fn wait_for(client: &Client, query: &str, expected: &str, forbidden: &str) -> String {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let result = call(client, "search_context", json!({"query":query})).await;
            let text = serde_json::to_string(&result).unwrap();
            if result.structured_content.as_ref().is_some_and(|value| value["result"]["ready"] == false) {
                assert!(!text.contains("unavailable"), "{text}");
            } else if text.contains(expected) && !text.contains(forbidden) {
                assert_ne!(result.is_error, Some(true));
                break text;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await.expect("native watcher must converge without explicit indexing")
}
async fn role(client: &Client) -> String {
    let result = call(client, "get_index_info", json!({})).await;
    result.structured_content.unwrap()["result"]["freshness"]["status"].as_str().unwrap_or("starting").to_owned()
}
fn source(root: &Path, marker: &str) {
    fs::write(root.join("lib.rs"),format!("/// {marker}\npub fn watched_symbol() {{}}\n")).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_live_edits_creates_deletes_and_restart_stay_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let a = temp.path().join("project-a");
    let b = temp.path().join("project-b");
    for path in [&home, &a, &b] { fs::create_dir(path).unwrap(); }
    source(&a, "ORIGINAL_A"); source(&b, "PRIVATE_B");
    let first = connect(&home, &a).await;
    let other = connect(&home, &b).await;
    wait_for(&first,"watched_symbol","ORIGINAL_A","PRIVATE_B").await;
    wait_for(&other,"watched_symbol","PRIVATE_B","ORIGINAL_A").await;
    let original_b = fs::read(b.join(".codanna/index/index.meta")).unwrap();
    source(&a,"UPDATED_A");
    wait_for(&first,"watched_symbol","UPDATED_A","ORIGINAL_A").await;
    assert_eq!(fs::read(b.join(".codanna/index/index.meta")).unwrap(), original_b);
    fs::write(a.join("new.rs"), "/// CREATED_A\npub fn new_symbol() {}\n").unwrap();
    wait_for(&first,"new_symbol","CREATED_A","PRIVATE_B").await;
    fs::remove_file(a.join("new.rs")).unwrap();
    wait_for(&first,"new_symbol","No matching code symbols","CREATED_A").await;
    first.cancel().await.unwrap();
    source(&a,"OFFLINE_A");
    let resumed = connect(&home,&a).await;
    wait_for(&resumed,"watched_symbol","OFFLINE_A","UPDATED_A").await;
    assert!(!home.join(".cache").exists());
    resumed.cancel().await.unwrap(); other.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_live_elects_one_writer_and_takes_over_after_disconnect() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home"); let root = temp.path().join("project");
    fs::create_dir(&home).unwrap(); fs::create_dir(&root).unwrap(); source(&root,"INITIAL");
    let a = connect(&home,&root).await; let b = connect(&home,&root).await;
    wait_for(&a,"watched_symbol","INITIAL","FOREIGN").await;
    wait_for(&b,"watched_symbol","INITIAL","FOREIGN").await;
    let (a_role,b_role) = (role(&a).await,role(&b).await);
    assert!(matches!((a_role.as_str(),b_role.as_str()),("watching","following") | ("following","watching")),"{a_role}/{b_role}");
    let metadata = fs::read(root.join(".codanna/index/index.meta")).unwrap();
    let refused = tokio::time::timeout(Duration::from_secs(15),process(&home,&root).args(["index","--force","--no-progress"]).output()).await.unwrap().unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("active writer"));
    assert_eq!(fs::read(root.join(".codanna/index/index.meta")).unwrap(),metadata);
    let follower = if a_role == "watching" { a.cancel().await.unwrap(); b } else { b.cancel().await.unwrap(); a };
    tokio::time::timeout(Duration::from_secs(30), async {
        while role(&follower).await != "watching" { tokio::time::sleep(Duration::from_millis(100)).await; }
    }).await.unwrap();
    source(&root,"AFTER_TAKEOVER");
    wait_for(&follower,"watched_symbol","AFTER_TAKEOVER","INITIAL").await;
    follower.cancel().await.unwrap();
}
