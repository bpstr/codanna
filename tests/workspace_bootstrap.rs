//! End-to-end first-use regressions. No fixture runs `init`, `index`, registry
//! commands, or model setup. Every client gets the same project-agnostic entry.
#![cfg(unix)]

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

async fn connect(home: &Path, project: &Path) -> RunningService<RoleClient, ()> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_codanna"));
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("NO_COLOR", "1")
        .current_dir(project)
        .args(["workspace", "serve"])
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for key in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    tokio::time::timeout(
        Duration::from_secs(15),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .expect("handshake deadline")
    .expect("handshake must work before initialization")
}

async fn search(client: &RunningService<RoleClient, ()>) -> CallToolResult {
    let request = CallToolRequestParams::new("search_context")
        .with_arguments(json!({"query": "bootstrap_target"}).as_object().unwrap().clone());
    let result = tokio::time::timeout(Duration::from_secs(15), client.call_tool(request))
        .await
        .expect("query deadline")
        .expect("query must not fail during initial setup");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result
}

async fn wait_for_ready(client: &RunningService<RoleClient, ()>) -> CallToolResult {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let result = search(client).await;
            if result.structured_content.as_ref().unwrap()["result"]["ready"] != false {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("initial code index must become ready")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_bootstrap_non_source_files_do_not_freeze_empty_graph() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("projectb");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&project).unwrap();
    fs::write(project.join("notes.not_a_code_language"), "planning notes").unwrap();
    let client = connect(&home, &project).await;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let result = search(&client).await;
            let status = &result.structured_content.as_ref().unwrap()["result"]["status"];
            if status == "empty" {
                break;
            }
            assert_eq!(status, "indexing", "not-yet-indexable code is not ready: {result:?}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("no supported source must be reported as empty");
    assert!(!project.join(".codanna/index/index.meta").exists());
    fs::write(
        project.join("lib.rs"),
        "/// FIRST_REAL_SOURCE\npub fn bootstrap_target() -> u32 { 7 }\n",
    )
    .unwrap();
    let result = wait_for_ready(&client).await;
    assert!(serde_json::to_string(&result).unwrap().contains("FIRST_REAL_SOURCE"));
    assert!(project.join(".codanna/index/index.meta").is_file());
    assert!(!home.join(".codanna/models").exists());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_bootstrap_empty_config_and_ignore_rules_remain_authoritative() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("projecta");
    fs::create_dir(&home).unwrap();
    fs::create_dir_all(project.join(".codanna")).unwrap();
    fs::create_dir(project.join("excluded")).unwrap();
    fs::write(
        project.join("lib.rs"),
        "/// INCLUDED_SOURCE\npub fn bootstrap_target() -> u32 { 1 }\n",
    )
    .unwrap();
    fs::write(
        project.join("excluded/lib.rs"),
        "/// EXCLUDED_SOURCE\npub fn bootstrap_target() -> u32 { 2 }\n",
    )
    .unwrap();
    let config = b"[semantic_search]\nenabled = false\n[indexing]\nindexed_paths = []\n";
    let ignores = b"excluded/\n.codanna/\n";
    fs::write(project.join(".codanna/settings.toml"), config).unwrap();
    fs::write(project.join(".codannaignore"), ignores).unwrap();
    let client = connect(&home, &project).await;
    let result = wait_for_ready(&client).await;
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.contains("INCLUDED_SOURCE"), "{text}");
    assert!(!text.contains("EXCLUDED_SOURCE"), "{text}");
    assert_eq!(fs::read(project.join(".codanna/settings.toml")).unwrap(), config);
    assert_eq!(fs::read(project.join(".codannaignore")).unwrap(), ignores);
    client.cancel().await.unwrap();
}
