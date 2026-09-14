//! Process-level workspace regressions. Every child uses a temporary HOME and
//! provider-free settings. These tests never index code or load embedding models.

#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        Self { _temp: temp, root, home }
    }

    fn workspace(&self, name: &str) -> PathBuf {
        let root = self.root.join(name);
        fs::create_dir_all(root.join(".codanna")).unwrap();
        fs::write(root.join(".codanna/settings.toml"), "version = 1\n[semantic_search]\nenabled = false\n").unwrap();
        root
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codanna"));
        command.env_clear().env("HOME", &self.home).env("USERPROFILE", &self.home)
            .env("NO_COLOR", "1").current_dir(&self.root).args(args)
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        // Preserve only platform/runtime loader settings, never provider credentials.
        for key in ["SYSTEMROOT", "LD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn add(&self, root: &Path, name: &str) -> Value {
        self.json(&["workspace", "add", root.to_str().unwrap(), "--name", name, "--json"])
    }
}

#[test]
fn workspace_cli_empty_list_and_help_are_side_effect_free() {
    let fixture = Fixture::new();
    assert_eq!(fixture.json(&["workspace", "list", "--json"])["workspaces"], serde_json::json!([]));
    assert!(fixture.run(&["workspace", "--help"]).status.success());
    assert!(fixture.run(&["completions", "bash"]).status.success());
    assert!(!fixture.home.join(".codanna").exists());
    assert!(!fixture.root.join(".codanna").exists());
}

#[test]
fn workspace_cli_registers_products_not_unconfigured_child_directories() {
    let fixture = Fixture::new();
    let assign = fixture.workspace("assign");
    let codanna = fixture.workspace("codanna");
    let web = assign.join("assign-web/src");
    fs::create_dir_all(&web).unwrap();
    let first = fixture.add(&assign, "assign");
    let nested = fixture.json(&["workspace", "add", web.to_str().unwrap(), "--json"]);
    assert_eq!(first["workspace"]["id"], nested["workspace"]["id"]);
    fixture.add(&codanna, "codanna");
    let all = fixture.json(&["workspace", "list", "--json"]);
    assert_eq!(all["workspaces"].as_array().unwrap().len(), 2);
    assert!(!assign.join(".codanna/index").exists());
    assert!(!codanna.join(".codanna/index").exists());
    assert!(!web.join(".codanna").exists());
}

#[test]
fn workspace_cli_parallel_processes_do_not_lose_registrations() {
    let fixture = Fixture::new();
    let mut children = Vec::new();
    for n in 0..8 {
        let name = format!("product-{n}");
        let root = fixture.workspace(&name);
        children.push(fixture.command(&["workspace", "add", root.to_str().unwrap(), "--json"]).spawn().unwrap());
    }
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    let all = fixture.json(&["workspace", "list", "--json"]);
    assert_eq!(all["workspaces"].as_array().unwrap().len(), 8);
    let registry: Value = serde_json::from_slice(&fs::read(fixture.home.join(".codanna/projects.json")).unwrap()).unwrap();
    assert_eq!(registry["version"], 1);
    assert!(registry.get("snapshot_digest").is_none());
}

#[test]
fn workspace_cli_explicit_selection_uses_target_config_not_cwd_or_environment() {
    let fixture = Fixture::new();
    let assign = fixture.workspace("assign");
    let codanna = fixture.workspace("codanna");
    fixture.add(&assign, "assign");
    fixture.add(&codanna, "codanna");
    let output = fixture.command(&["--workspace", "assign", "config"])
        .current_dir(&codanna)
        .env("CI_INDEX_PATH", codanna.join(".codanna/index"))
        .env("CI_WORKSPACE_ROOT", &codanna)
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    let table: toml::Value = toml::from_str(&text.lines().skip(2).collect::<Vec<_>>().join("\n")).unwrap();
    assert_eq!(PathBuf::from(table["workspace_root"].as_str().unwrap()), assign);
    assert_eq!(PathBuf::from(table["index_path"].as_str().unwrap()), assign.join(".codanna/index"));
    assert!(!assign.join(".codanna/index").exists());
    assert!(!codanna.join(".codanna/index").exists());
}

#[test]
fn workspace_cli_rejects_unknown_and_conflicting_selectors_before_auto_init() {
    let fixture = Fixture::new();
    for args in [
        vec!["--workspace", "missing", "index", "--force"],
        vec!["--workspace", "missing", "--config", "other.toml", "index"],
        vec!["--workspace", "missing", "workspace", "list"],
    ] {
        assert!(!fixture.run(&args).status.success());
    }
    assert!(!fixture.root.join(".codanna").exists());
    assert!(!fixture.home.join(".codanna").exists());
}

#[test]
fn workspace_cli_unregistration_keeps_existing_index_bytes() {
    let fixture = Fixture::new();
    let assign = fixture.workspace("assign");
    fs::create_dir_all(assign.join(".codanna/index")).unwrap();
    fs::write(assign.join(".codanna/index/sentinel"), b"not a real index").unwrap();
    let before = fixture.add(&assign, "assign");
    let renamed = fixture.json(&["workspace", "rename", "assign", "product", "--json"]);
    assert_eq!(before["workspace"]["id"], renamed["workspace"]["id"]);
    fixture.json(&["workspace", "remove", "product", "--json"]);
    assert_eq!(fs::read(assign.join(".codanna/index/sentinel")).unwrap(), b"not a real index");
}
