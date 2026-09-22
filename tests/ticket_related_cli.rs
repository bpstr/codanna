//! Disposable process environment; documents, semantic indexing and recall disabled.
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, Output};

struct Workspace { temp: tempfile::TempDir, config: PathBuf }
impl Workspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::create_dir_all(temp.path().join("home")).unwrap();
        std::fs::create_dir_all(temp.path().join(".codanna")).unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "/// Conversation timeout dispatch.\npub fn dispatch_ticket() { timeout_worker(); }\nfn timeout_worker() {}\n").unwrap();
        let config = temp.path().join(".codanna/settings.toml");
        std::fs::write(&config, "version = 1\nindex_path = \".codanna/index\"\n[semantic_search]\nenabled = false\n[documents]\nenabled = false\n[indexing]\nindexed_paths = [\"src\"]\n").unwrap();
        let workspace = Self { temp, config };
        let result = workspace.run(&["index", "src", "--force"]);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        workspace
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_codanna"))
            .env_clear().env("HOME", self.temp.path().join("home"))
            .env("XDG_CONFIG_HOME", self.temp.path().join("home/config"))
            .env("XDG_CACHE_HOME", self.temp.path().join("home/cache"))
            .env("NO_COLOR", "1").current_dir(self.temp.path())
            .arg("--config").arg(&self.config).args(args).output().unwrap()
    }
    fn ticket(&self, value: &Value, json: bool) -> Output {
        let encoded = value.to_string();
        let mut args = vec!["mcp", "search_ticket_context", "--args", &encoded];
        if json { args.push("--json"); }
        self.run(&args)
    }
}

#[test]
fn ticket_related_cli_json_text_and_default_compatibility() {
    let workspace = Workspace::new();
    let mut request = json!({"query":"conversation timeout", "code_limit":1});
    let disabled = workspace.ticket(&request, true);
    assert!(disabled.status.success());
    let disabled: Value = serde_json::from_slice(&disabled.stdout).unwrap();
    assert!(disabled["data"]["code"].get("related_code").is_none());
    request["include_related_code"] = json!(true);
    let enabled = workspace.ticket(&request, true);
    assert!(enabled.status.success(), "{}", String::from_utf8_lossy(&enabled.stderr));
    let enabled: Value = serde_json::from_slice(&enabled.stdout).unwrap();
    assert_eq!(enabled["data"]["code"]["items"], disabled["data"]["code"]["items"]);
    let related = &enabled["data"]["code"]["related_code"];
    assert_eq!(related["status"], "completed_bounded");
    assert_eq!(related["items"][0]["name"], "timeout_worker");
    assert_eq!(related["items"][0]["via"][0]["seed_name"], "dispatch_ticket");
    let text = workspace.ticket(&request, false);
    assert!(text.status.success());
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("Related implementations"));
    assert!(text.contains("timeout_worker"));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn ticket_related_cli_rejects_wrong_types_and_never_broadens_scope() {
    let workspace = Workspace::new();
    for invalid in [json!("true"), json!(1), Value::Null] {
        let result = workspace.ticket(&json!({"query":"conversation timeout", "include_related_code":invalid}), true);
        assert_eq!(result.status.code(), Some(2));
        let envelope: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(envelope["exit_code"], 2);
    }
    let result = workspace.ticket(&json!({
        "query":"conversation timeout", "include_related_code":true, "code_path_prefix":"src",
    }), true);
    assert!(result.status.success());
    let envelope: Value = serde_json::from_slice(&result.stdout).unwrap();
    let report = &envelope["data"]["code"]["related_code"];
    assert_eq!(report["status"], "not_run_scoped_graph_unsupported");
    assert_eq!(report["probes"], json!([]));
    assert_eq!(report["items"], json!([]));
}
