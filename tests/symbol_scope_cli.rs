//! Process-boundary scope contracts. All sources, HOME, config and indexes are
//! disposable, with semantic indexing, document search and recall disabled.

use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, Output};

struct Workspace {
    temp: tempfile::TempDir,
    config: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        for (directory, name) in [("active", "activeCalendar"), ("reference", "referenceCalendar")] {
            let source = temp.path().join("src").join(directory);
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(source.join("calendar.rs"), format!(
                "/// Calendar settings for {directory}.\npub fn {name}() {{}}\n"
            )).unwrap();
        }
        std::fs::create_dir_all(temp.path().join("home")).unwrap();
        std::fs::create_dir_all(temp.path().join(".codanna")).unwrap();
        let config = temp.path().join(".codanna/settings.toml");
        std::fs::write(&config, r#"
version = 1
index_path = ".codanna/index"
[semantic_search]
enabled = false
[documents]
enabled = false
[indexing]
indexed_paths = ["src"]
"#).unwrap();
        let workspace = Self { temp, config };
        let indexed = workspace.run(&["index", "src", "--force"]);
        assert!(indexed.status.success(), "{}", diagnostics(&indexed));
        workspace
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_codanna"))
            .env_clear()
            .env("HOME", self.temp.path().join("home"))
            .env("XDG_CONFIG_HOME", self.temp.path().join("home/config"))
            .env("XDG_CACHE_HOME", self.temp.path().join("home/cache"))
            .env("NO_COLOR", "1")
            .current_dir(self.temp.path())
            .arg("--config").arg(&self.config)
            .args(args).output().unwrap()
    }

    fn mcp(&self, tool: &str, request: &Value, as_json: bool) -> Output {
        let encoded = request.to_string();
        let mut args = vec!["mcp", tool, "--args", &encoded];
        if as_json { args.push("--json"); }
        self.run(&args)
    }
}

fn diagnostics(output: &Output) -> String {
    format!("status={}\nstdout={}\nstderr={}", output.status,
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("invalid envelope: {error}; {}", diagnostics(output))
    })
}

#[test]
fn mcp_and_retrieve_json_do_not_drop_the_requested_scope() {
    let workspace = Workspace::new();
    let request = json!({"query": "calendar settings", "limit": 5, "path_prefix": "src/active"});
    let output = workspace.mcp("search_symbols", &request, true);
    assert!(output.status.success(), "{}", diagnostics(&output));
    let data = envelope(&output);
    let hits = data["data"].as_array().expect("symbol results");
    assert_eq!(hits.len(), 1, "{data}");
    assert_eq!(hits[0]["symbol"]["name"], "activeCalendar");
    assert_eq!(hits[0]["symbol"]["file_path"], "src/active/calendar.rs");

    let text = workspace.mcp("search_symbols", &request, false);
    assert!(text.status.success(), "{}", diagnostics(&text));
    let rendered = String::from_utf8_lossy(&text.stdout);
    assert!(rendered.contains("activeCalendar"), "{rendered}");
    assert!(!rendered.contains("referenceCalendar"), "{rendered}");

    let retrieve = workspace.run(&["retrieve", "search", "calendar settings", "--path-prefix", "src/active", "--json"]);
    assert!(retrieve.status.success(), "{}", diagnostics(&retrieve));
    let result = envelope(&retrieve);
    let hits = result["data"].as_array().expect("retrieve results");
    assert_eq!(hits.len(), 1, "{result}");
    assert_eq!(hits[0]["symbol"]["name"], "activeCalendar");
}

#[test]
fn context_json_and_text_share_the_code_subtree() {
    let workspace = Workspace::new();
    let request = json!({
        "query": "calendar settings", "code_limit": 5,
        "document_limit": 1, "conversation_limit": 1,
        "code_path_prefix": "src/reference",
    });
    for as_json in [false, true] {
        let output = workspace.mcp("search_context", &request, as_json);
        assert!(output.status.success(), "{}", diagnostics(&output));
        let rendered = if as_json {
            envelope(&output)["data"]["text"].as_str().expect("context text").to_owned()
        } else {
            String::from_utf8(output.stdout).unwrap()
        };
        let code = rendered.split("## Documents").next().unwrap();
        assert!(code.contains("referenceCalendar"), "{code}");
        assert!(!code.contains("activeCalendar"), "{code}");
    }
}

#[test]
fn malformed_scopes_fail_instead_of_becoming_global_searches_or_success_envelopes() {
    let workspace = Workspace::new();
    for (tool, field) in [("search_symbols", "path_prefix"), ("search_context", "code_path_prefix")] {
        for invalid in [json!("../outside"), json!(""), json!(17), json!([]), json!({})] {
            let mut request = json!({"query": "calendar settings"});
            request[field] = invalid;
            for as_json in [false, true] {
                let output = workspace.mcp(tool, &request, as_json);
                assert_eq!(output.status.code(), Some(2), "{tool}: {}", diagnostics(&output));
                if as_json {
                    let result = envelope(&output);
                    assert_eq!(result["exit_code"], 2, "{result}");
                    assert_eq!(result["code"], "INVALID_QUERY", "{result}");
                }
            }
        }
    }
}

#[test]
fn absent_scope_does_not_fall_back_to_the_other_implementation() {
    let workspace = Workspace::new();
    let output = workspace.mcp("search_symbols", &json!({
        "query": "calendar settings", "path_prefix": "src/missing",
    }), true);
    assert_eq!(output.status.code(), Some(1), "{}", diagnostics(&output));
    let result = envelope(&output);
    assert_eq!(result["code"], "NOT_FOUND", "{result}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("activeCalendar"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("referenceCalendar"));
}
