//! Deterministic replacements for the four historical print-only shell scripts.
//! Test the actual binary and persisted index; never index the developer's repo.

use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        let fixture = Self(tempfile::tempdir().unwrap());
        let root = fixture.0.path();
        std::fs::create_dir_all(root.join(".codanna")).unwrap();
        std::fs::create_dir_all(root.join(".home")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join(".codanna/settings.toml"), format!(
            "index_path = \".codanna/index\"\n[indexing]\nindexed_paths = [{}]\n[semantic_search]\nenabled = false\n",
            crate::common::toml_path_literal(&root.join("src")),
        )).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            r#"
pub trait ReviewTrait { fn execute(&self); }
pub struct ReviewWorker;
impl ReviewWorker { pub fn review_method() -> usize { review_leaf() } }
impl ReviewTrait for ReviewWorker { fn execute(&self) { review_leaf(); } }
pub fn review_leaf() -> usize { 1 }
pub fn review_entry() -> usize { review_leaf() }
pub fn review_other() -> usize { 2 }
"#,
        )
        .unwrap();
        let seeded = fixture.output(&["index", "src", "--no-progress"]);
        assert!(
            seeded.status.success(),
            "fixture index: {}",
            String::from_utf8_lossy(&seeded.stderr)
        );
        fixture
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codanna"));
        command
            .current_dir(self.0.path())
            .env("HOME", self.0.path().join(".home"));
        command
    }

    fn output(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run fixture CLI")
    }

    fn json(&self, args: &[&str], exit: i32) -> Value {
        let output = self.output(args);
        assert_eq!(
            output.status.code(),
            Some(exit),
            "{args:?}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // Parsing the entire stdout rejects extra debug text and multiple JSON values.
        let value: Value = serde_json::from_slice(&output.stdout)
            .expect("stdout must be exactly one JSON envelope");
        assert_eq!(value["exit_code"], exit);
        value
    }
}

fn names(data: &Value) -> Vec<&str> {
    let mut names: Vec<_> = data
        .as_array()
        .expect("array payload")
        .iter()
        .map(|item| item["symbol"]["name"].as_str().expect("symbol name"))
        .collect();
    names.sort();
    names
}

#[test]
fn hardening_review_cli_clean_json_and_exit_codes() {
    let fixture = Fixture::new();
    for (command, subject) in [
        ("symbol", "review_entry"),
        ("calls", "review_entry"),
        ("callers", "review_leaf"),
        ("describe", "ReviewWorker"),
        ("implementations", "ReviewTrait"),
        ("search", "review"),
    ] {
        let envelope = fixture.json(&["retrieve", command, subject, "--json"], 0);
        assert!(
            envelope["meta"]["count"].as_u64().expect("result count") > 0,
            "{command} must exercise a real result"
        );
        assert!(!envelope["data"].is_null());
    }
    for command in [
        "symbol",
        "calls",
        "callers",
        "describe",
        "implementations",
        "search",
    ] {
        let missing = fixture.json(&["retrieve", command, "no_such_review_symbol", "--json"], 3);
        assert_eq!(missing["status"], "not_found");
        assert!(missing["data"].is_null());
    }
    for command in ["symbol", "calls", "callers", "describe"] {
        let invalid = fixture.json(&["retrieve", command, "symbol_id:invalid", "--json"], 1);
        assert_eq!(invalid["code"], "INVALID_QUERY");
    }
    let projected = fixture.json(
        &[
            "retrieve",
            "symbol",
            "review_entry",
            "--fields",
            "invalid_field",
            "--json",
        ],
        2,
    );
    assert_eq!(projected["code"], "INVALID_QUERY");
    // MCP deliberately uses a different not-found process code; preserve it.
    let missing = fixture.json(
        &["mcp", "find_symbol", "name:no_such_review_symbol", "--json"],
        1,
    );
    assert_eq!(missing["status"], "not_found");
}

#[test]
fn hardening_review_cli_dual_formats_and_flag_precedence() {
    let fixture = Fixture::new();
    for (command, positional, keyword) in [
        ("symbol", "review_entry", "name:review_entry"),
        ("calls", "review_entry", "function:review_entry"),
        ("callers", "review_leaf", "function:review_leaf"),
        ("implementations", "ReviewTrait", "trait:ReviewTrait"),
        ("describe", "ReviewWorker", "symbol:ReviewWorker"),
        ("search", "review", "query:review"),
    ] {
        let a = fixture.json(&["retrieve", command, positional, "--json"], 0);
        let b = fixture.json(&["retrieve", command, keyword, "--json"], 0);
        if command == "describe" {
            assert_eq!(a["data"]["symbol"], b["data"]["symbol"]);
        } else {
            assert_eq!(names(&a["data"]), names(&b["data"]), "{command}");
        }
    }
    let limited = fixture.json(
        &[
            "retrieve", "search", "review", "limit:10", "--limit", "1", "--json",
        ],
        0,
    );
    assert_eq!(limited["meta"]["count"], 1);
    assert_eq!(limited["data"].as_array().unwrap().len(), 1);
    let positional = fixture.json(
        &[
            "retrieve",
            "symbol",
            "review_entry",
            "name:no_such_review_symbol",
            "--json",
        ],
        0,
    );
    assert_eq!(names(&positional["data"]), ["review_entry"]);
}

#[test]
fn hardening_review_cli_describe_relationships_are_populated() {
    let fixture = Fixture::new();
    let worker = fixture.json(&["retrieve", "describe", "ReviewWorker", "--json"], 0);
    assert!(
        worker["data"]["relationships"]["defines"]
            .as_array()
            .expect("method definitions")
            .iter()
            .any(|symbol| symbol["name"] == "review_method")
    );
    let entry = fixture.json(&["retrieve", "describe", "review_entry", "--json"], 0);
    let calls = entry["data"]["relationships"]["calls"]
        .as_array()
        .expect("call edges");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][0]["name"], "review_leaf");
    let leaf = fixture.json(&["retrieve", "describe", "review_leaf", "--json"], 0);
    assert!(
        leaf["data"]["relationships"]["called_by"]
            .as_array()
            .expect("caller edges")
            .iter()
            .any(|edge| edge[0]["name"] == "review_entry")
    );
    let implementation = fixture.json(&["retrieve", "describe", "ReviewTrait", "--json"], 0);
    assert!(
        implementation["data"]["relationships"]["implemented_by"]
            .as_array()
            .expect("implementations")
            .iter()
            .any(|symbol| symbol["name"] == "ReviewWorker")
    );
}

#[test]
fn hardening_review_cli_json_chain_uses_actual_symbol_identity() {
    let fixture = Fixture::new();
    let symbol = fixture.json(&["retrieve", "symbol", "review_entry", "--json"], 0);
    let found = names(&symbol["data"]);
    assert_eq!(found, ["review_entry"]);
    let calls = fixture.json(&["retrieve", "calls", found[0], "--json"], 0);
    let callees = names(&calls["data"]);
    assert_eq!(callees, ["review_leaf"]);
    let callers = fixture.json(&["retrieve", "callers", callees[0], "--json"], 0);
    assert!(names(&callers["data"]).contains(&"review_entry"));
}

#[test]
fn hardening_review_cli_missing_arguments_fail() {
    let fixture = Fixture::new();
    for command in [
        "symbol",
        "calls",
        "callers",
        "describe",
        "implementations",
        "search",
    ] {
        let result = fixture.output(&["retrieve", command]);
        assert_eq!(result.status.code(), Some(1), "{command}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("requires"),
            "{command} must explain missing input"
        );
    }
}

#[test]
fn hardening_review_legacy_shell_entrypoints_are_fail_closed_wrappers() {
    // The wrappers all delegate to the same authoritative Cargo suite; they
    // cannot report success by echoing a checkmark after a failed command.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for filename in [
        "test_clean_json.sh",
        "test_pipes.sh",
        "test_describe_relationships.sh",
        "test_dual_format_all.sh",
    ] {
        let script = std::fs::read_to_string(root.join("tests/cli").join(filename)).unwrap();
        assert!(script.contains("set -euo pipefail"));
        assert!(script.contains("exec bash contributing/scripts/review-cli-regressions.sh"));
    }
}
