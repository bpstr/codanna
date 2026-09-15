//! One automatically derived recall scope for the importer and workspace MCP.
//! Explicit synthetic transcripts only: no private-history discovery or inference.
#![cfg(unix)]
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

fn command(binary: &str, home: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("PATH", "/usr/bin:/bin")
        .current_dir(cwd)
        .stdin(Stdio::null());
    for key in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.kill_on_drop(true);
    command
}

async fn recall(home: &Path, root: &Path, args: &[&str]) -> std::process::Output {
    tokio::time::timeout(
        Duration::from_secs(10),
        command(env!("CARGO_BIN_EXE_codanna-recall"), home, root)
            .args(args)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_recall_import_and_mcp_share_scope_without_names_or_environment() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let a = temp.path().join("one/project");
    let b = temp.path().join("two/project");
    for (root, provider, marker) in [
        (&a, "codex", "ONLY_HISTORY_A"),
        (&b, "claude", "ONLY_HISTORY_B"),
    ] {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join("lib.rs"), "pub fn recall_topic() {}\n").unwrap();
        let row = if provider == "codex" {
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":format!("recall_topic {marker}")}]}})
        } else {
            json!({"type":"user","uuid":"fixture","sessionId":"fixture","message":{"role":"user","content":format!("recall_topic {marker}")}})
        };
        let file = root.join("transcript.jsonl");
        fs::write(&file, format!("{row}\n")).unwrap();
        let imported = recall(
            &home,
            root,
            &[
                "import",
                "--provider",
                provider,
                "--file",
                file.to_str().unwrap(),
            ],
        )
        .await;
        assert!(
            imported.status.success(),
            "{}",
            String::from_utf8_lossy(&imported.stderr)
        );
        assert!(
            !root.join(".codanna").exists(),
            "import must not initialize the code index"
        );
    }
    let lookup = recall(&home, &a, &["search", "recall_topic"]).await;
    assert!(lookup.status.success());
    let hits: Value = serde_json::from_slice(&lookup.stdout).unwrap();
    let id = hits["results"][0]["message"]["id"].as_str().unwrap();
    assert!(!recall(&home, &b, &["read", id]).await.status.success());

    for (root, expected, forbidden) in [
        (&a, "ONLY_HISTORY_A", "ONLY_HISTORY_B"),
        (&b, "ONLY_HISTORY_B", "ONLY_HISTORY_A"),
    ] {
        let mut process = command(env!("CARGO_BIN_EXE_codanna"), &home, root);
        process
            .args(["workspace", "serve"])
            .env("CODANNA_RECALL_WORKSPACE", "wrong-inherited-namespace")
            .env("CODANNA_RECALL_INDEX", temp.path().join("wrong-index"))
            .stderr(Stdio::null());
        let client = tokio::time::timeout(
            Duration::from_secs(15),
            ().serve(TokioChildProcess::new(process).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let request = CallToolRequestParams::new("search_context")
                    .with_arguments(json!({"query":"recall_topic"}).as_object().unwrap().clone());
                let result = client.call_tool(request).await.unwrap();
                assert_ne!(result.is_error, Some(true));
                if result
                    .structured_content
                    .as_ref()
                    .is_none_or(|value| value["result"]["ready"] != false)
                {
                    break serde_json::to_string(&result).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
        assert!(result.contains(expected), "{result}");
        assert!(!result.contains(forbidden), "{result}");
        client.cancel().await.unwrap();
    }
}
