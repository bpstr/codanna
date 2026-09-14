//! Real binary witnesses. Temporary directories and cleared provider environment
//! ensure these tests never read user projects or spend embedding credits.
#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn cli(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codanna"));
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("NO_COLOR", "1")
        .current_dir(cwd)
        .args(args);
    // Preserve only loader configuration needed by the test binary, never
    // provider credentials, config overlays, or another project's recall.
    for key in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.output().unwrap()
}

fn ok(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
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

fn config(root: &Path, roots: &[&str]) {
    fs::create_dir_all(root.join(".codanna")).unwrap();
    let mut settings = codanna::Settings::default();
    settings.semantic_search.enabled = false;
    settings.indexing.parallelism = 1;
    settings.indexing.indexed_paths = roots.iter().map(Into::into).collect();
    fs::write(
        root.join(".codanna/settings.toml"),
        toml::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();
}

#[test]
fn hardening_workspace_auto_discovery_has_no_side_effects() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let root = temp.path().join("assign");
    checkout(&root.join("assign-core"), "core_identity");
    checkout(&root.join("assign-web"), "web_identity");
    let result: Value =
        serde_json::from_str(&ok(cli(&home, &root, &["workspace", "discover", "--json"]))).unwrap();
    assert_eq!(result["repositories"].as_array().unwrap().len(), 2);
    assert_eq!(result["configured"], false);
    assert!(!home.exists());
    assert!(!root.join(".codanna").exists());
}

#[test]
fn hardening_workspace_auto_assign_and_codanna_index_without_registration_commands() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let assign = temp.path().join("assign");
    let core = assign.join("assign-core");
    let web = assign.join("assign-web");
    let codanna = temp.path().join("codanna");
    checkout(&core, "product_unique_identity");
    checkout(&web, "product_unique_identity");
    checkout(&codanna, "unrelated_unique_identity");
    config(&assign, &["."]);
    config(&core, &["src"]);
    config(&web, &["src"]);
    config(&codanna, &["."]);
    let child_config = fs::read(web.join(".codanna/settings.toml")).unwrap();
    ok(cli(&home, &web, &["index", "--no-progress"]));
    ok(cli(
        &home,
        &codanna.join("src"),
        &["index", "--no-progress"],
    ));
    let registrations: Value =
        serde_json::from_str(&ok(cli(&home, &assign, &["workspace", "list", "--json"]))).unwrap();
    assert_eq!(registrations["workspaces"].as_array().unwrap().len(), 2);
    let results = ok(cli(
        &home,
        &web.join("src"),
        &["retrieve", "search", "product_unique_identity", "--json"],
    ));
    let results: Value = serde_json::from_str(&results).unwrap();
    let items = results["data"]["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        2,
        "Assign query should include both member repositories: {results}"
    );
    let rendered = results.to_string();
    assert!(rendered.contains("assign-core"));
    assert!(rendered.contains("assign-web"));
    assert!(!rendered.contains("unrelated_unique_identity"));
    let unrelated = ok(cli(
        &home,
        &codanna,
        &["retrieve", "search", "unrelated_unique_identity", "--json"],
    ));
    assert!(unrelated.contains("unrelated_unique_identity"));
    assert!(!unrelated.contains("product_unique_identity"));
    assert_eq!(
        fs::read(web.join(".codanna/settings.toml")).unwrap(),
        child_config
    );
    assert!(!web.join(".codanna/index").exists());
    assert!(!core.join(".codanna/index").exists());
}

#[test]
fn hardening_workspace_auto_bare_index_fills_only_fresh_empty_source_list() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let root = temp.path().join("codanna");
    checkout(&root, "initialized_project_identity");
    config(&root, &[]);
    ok(cli(&home, &root.join("src"), &["index", "--no-progress"]));
    let text = fs::read_to_string(root.join(".codanna/settings.toml")).unwrap();
    let settings: codanna::Settings = toml::from_str(&text).unwrap();
    assert!(!settings.indexing.indexed_paths.is_empty());
    assert!(!settings.semantic_search.enabled);
    let query = ok(cli(
        &home,
        &root,
        &[
            "retrieve",
            "search",
            "initialized_project_identity",
            "--json",
        ],
    ));
    assert!(query.contains("initialized_project_identity"));
}

#[test]
fn hardening_workspace_auto_unindexed_serve_does_not_load_models_or_write_state() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let root = temp.path().join("codanna");
    checkout(&root, "identity");
    let result = cli(&home, &root, &["serve"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("codanna index"));
    assert!(!root.join(".codanna").exists());
    assert!(!home.exists());
}

#[test]
fn hardening_workspace_auto_inspect_from_sibling_does_not_create_product_parent() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let parent = temp.path().join("projects");
    let assign = parent.join("assign");
    let codanna = parent.join("codanna");
    checkout(&assign, "assign_identity");
    checkout(&codanna, "codanna_identity");
    for root in [&assign, &codanna] {
        let result: Value = serde_json::from_str(&ok(cli(
            &home,
            &root.join("src"),
            &["workspace", "discover", "--json"],
        )))
        .unwrap();
        assert_eq!(
            result["root"],
            root.canonicalize().unwrap().to_string_lossy().as_ref()
        );
    }
    assert!(!parent.join(".codanna").exists());
    assert!(!home.exists());
}
