//! Hash-based drift inspection does not mutate stored ingestion state.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore};
use codanna::vector::VectorDimension;
use std::collections::BTreeMap;

#[test]
fn source_drift_distinguishes_changed_missing_new_and_unchanged_without_ingestion() {
    let temp = tempfile::tempdir().unwrap();
    let unchanged = temp.path().join("a.md");
    let changed = temp.path().join("b.md");
    let missing = temp.path().join("c.md");
    let new = temp.path().join("d.md");
    for path in [&unchanged, &changed, &missing] {
        std::fs::write(path, "Original source policy").unwrap();
    }
    let index = temp.path().join("index");
    let mut store = DocumentStore::new(&index, VectorDimension::new(2).unwrap()).unwrap();
    store
        .index_collection(
            "fixture",
            &CollectionConfig {
                paths: vec![unchanged.clone(), changed.clone(), missing.clone()],
                ..Default::default()
            },
            &ChunkingConfig {
                min_chunk_chars: 1,
                ..Default::default()
            },
        )
        .unwrap();
    std::fs::write(&changed, "Changed source policy").unwrap();
    std::fs::remove_file(&missing).unwrap();
    std::fs::write(&new, "New source policy").unwrap();
    let before = store.get_indexed_paths();
    let report = store.source_drift("fixture", &[new], 10, 4096);
    let statuses: BTreeMap<_, _> = report
        .files
        .iter()
        .map(|entry| {
            (
                entry.path.file_name().unwrap().to_str().unwrap(),
                entry.status,
            )
        })
        .collect();
    assert_eq!(
        statuses,
        BTreeMap::from([
            ("a.md", "unchanged"),
            ("b.md", "changed"),
            ("c.md", "missing"),
            ("d.md", "new")
        ])
    );
    assert!(!report.truncated);
    assert_eq!(store.get_indexed_paths(), before);
    let bounded = store.source_drift("fixture", &[], 1, 1);
    assert!(bounded.truncated);
    assert_eq!(bounded.files.len(), 1);
    assert_eq!(bounded.files[0].status, "byte_budget_exceeded");
    let indexed_hash = report
        .files
        .iter()
        .find(|entry| entry.status == "changed")
        .unwrap()
        .indexed_sha256
        .clone();
    drop(store);
    let reopened = DocumentStore::new(&index, VectorDimension::new(2).unwrap()).unwrap();
    let report = reopened.source_drift("fixture", &[], 10, 4096);
    assert_eq!(
        report
            .files
            .iter()
            .find(|entry| entry.status == "changed")
            .unwrap()
            .indexed_sha256,
        indexed_hash
    );
}

fn settings_fixture() -> (tempfile::TempDir, codanna::Settings) {
    let temp = tempfile::tempdir().unwrap();
    let docs = temp.path().join("docs");
    std::fs::create_dir(&docs).unwrap();
    std::fs::write(docs.join("changed.md"), "Original content").unwrap();
    std::fs::write(docs.join("missing.md"), "Original content").unwrap();
    std::fs::write(docs.join("unchanged.md"), "Original content").unwrap();
    let mut settings = codanna::Settings::default();
    settings.index_path = temp.path().join("index");
    settings.workspace_root = Some(temp.path().to_path_buf());
    settings.semantic_search.enabled = false;
    settings.documents.enabled = true;
    settings.documents.collections.insert(
        "docs".into(),
        CollectionConfig {
            paths: vec![docs.clone()],
            ..Default::default()
        },
    );
    let mut store = DocumentStore::new(
        settings.index_path.join("documents"),
        VectorDimension::new(2).unwrap(),
    )
    .unwrap();
    store
        .index_collection(
            "docs",
            &settings.documents.collections["docs"],
            &ChunkingConfig {
                min_chunk_chars: 1,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    drop(store);
    std::fs::write(docs.join("changed.md"), "Changed content").unwrap();
    std::fs::remove_file(docs.join("missing.md")).unwrap();
    std::fs::write(docs.join("new.md"), "New content").unwrap();
    std::fs::write(docs.join("ignored.md"), "Ignored content").unwrap();
    std::fs::write(docs.join(".codannaignore"), "ignored.md\n").unwrap();
    std::fs::write(docs.join("other.txt"), "Wrong extension").unwrap();
    (temp, settings)
}

fn request() -> codanna::mcp::DocumentDriftRequest {
    serde_json::from_value(serde_json::json!({"collection":"docs"})).unwrap()
}

fn persisted_files(root: &std::path::Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            (
                entry.path().strip_prefix(root).unwrap().to_path_buf(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn configured_drift_discovers_sources_without_repairing_or_mutating_persistence() {
    let (_temp, settings) = settings_fixture();
    let base = settings.index_path.join("documents");
    std::fs::remove_file(base.join("state.json")).unwrap();
    let before = persisted_files(&base);
    let report = codanna::documents::inspect_source_drift(&settings, &request()).unwrap();
    assert!(report.generation.is_some());
    assert!(!report.truncated);
    let states: BTreeMap<_, _> = report
        .files
        .iter()
        .map(|entry| {
            (
                entry.path.file_name().unwrap().to_str().unwrap(),
                entry.status,
            )
        })
        .collect();
    assert_eq!(
        states,
        BTreeMap::from([
            ("changed.md", "changed"),
            ("missing.md", "missing"),
            ("new.md", "new"),
            ("unchanged.md", "unchanged")
        ])
    );
    assert_eq!(persisted_files(&base), before);
    assert!(
        report
            .files
            .iter()
            .filter(|e| e.status == "changed")
            .all(|e| e.indexed_sha256 != e.current_sha256)
    );
}

#[test]
fn discovery_and_reads_report_limits_instead_of_claiming_complete_freshness() {
    let (_temp, settings) = settings_fixture();
    let mut request = request();
    request.max_entries = 1;
    request.max_files = 1;
    request.max_bytes = 1;
    let report = codanna::documents::inspect_source_drift(&settings, &request).unwrap();
    assert!(report.truncated);
    assert!(report.discovery_truncated);
    assert!(report.entries_visited <= 1);
    assert!(report.files.len() <= 1);
    assert!(report.bytes_read <= 1);
    assert_eq!(report.max_files, 1);
    assert_eq!(report.max_bytes, 1);
    assert_eq!(report.max_entries, 1);
}

#[test]
fn invalid_limits_unknown_collections_and_missing_indexes_create_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = codanna::Settings::default();
    settings.index_path = temp.path().join("missing");
    settings
        .documents
        .collections
        .insert("docs".into(), CollectionConfig::default());
    for input in [
        serde_json::json!({"collection":"docs","max_files":0}),
        serde_json::json!({"collection":"docs","max_bytes":67108865}),
        serde_json::json!({"collection":"unknown"}),
        serde_json::json!({"collection":"docs"}),
    ] {
        let req: codanna::mcp::DocumentDriftRequest = serde_json::from_value(input).unwrap();
        assert!(codanna::documents::inspect_source_drift(&settings, &req).is_err());
    }
    assert!(!settings.index_path.exists());
}

#[tokio::test]
async fn mcp_drift_works_without_a_document_model_and_returns_structured_evidence() {
    let (_temp, settings) = settings_fixture();
    let before = persisted_files(&settings.index_path.join("documents"));
    let facade =
        codanna::indexing::facade::IndexFacade::new(std::sync::Arc::new(settings.clone())).unwrap();
    let server = codanna::mcp::CodeIntelligenceServer::new(facade);
    let response = server
        .document_drift(rmcp::handler::server::wrapper::Parameters(request()))
        .await
        .unwrap();
    let report = response.structured_content.unwrap();
    assert_eq!(report["collection"], "docs");
    assert_eq!(report["truncated"], false);
    assert_eq!(report["files"].as_array().unwrap().len(), 4);
    assert_eq!(
        persisted_files(&settings.index_path.join("documents")),
        before
    );
}

fn run_drift_cli(
    temp: &std::path::Path,
    settings: &codanna::Settings,
    mcp: bool,
) -> std::process::Output {
    use std::process::{Command, Stdio};
    let config = temp.join("settings.toml");
    std::fs::write(&config, toml::to_string(settings).unwrap()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_codanna"));
    command
        .arg("--config")
        .arg(&config)
        .current_dir(temp)
        .env_clear()
        .env("HOME", temp);
    if mcp {
        command.args(["mcp", "document_drift", "collection:docs", "--json"]);
    } else {
        command.args(["documents", "drift", "docs", "--json"]);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("drift diagnostic exceeded its deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn cli_drift_is_model_free_and_does_not_repair_persistence() {
    let (temp, mut settings) = settings_fixture();
    // An invalid provider endpoint is deliberately never initialized by drift.
    settings.semantic_search.enabled = true;
    let base = settings.index_path.join("documents");
    std::fs::remove_file(base.join("state.json")).unwrap();
    let before = persisted_files(&base);
    let output = run_drift_cli(temp.path(), &settings, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["collection"], "docs");
    assert_eq!(report["files"].as_array().unwrap().len(), 4);
    assert_eq!(persisted_files(&base), before);
}

#[cfg(unix)]
#[test]
fn replaced_fifo_is_reported_without_blocking_the_cli() {
    let (temp, settings) = settings_fixture();
    let path = temp.path().join("docs/unchanged.md");
    std::fs::remove_file(&path).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let output = run_drift_cli(temp.path(), &settings, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let source = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"].as_str().unwrap().ends_with("unchanged.md"))
        .unwrap();
    assert_eq!(source["status"], "unsupported_source");
    assert!(source["current_sha256"].is_null());
}
