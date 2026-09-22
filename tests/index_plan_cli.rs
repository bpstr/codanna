//! Offline preflight through an actual CLI child with an empty environment.
//! Synthetic cache bytes only; no model, production store, or paid provider.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SOURCE: &str = "/// alpha calendar preference owner.\npub fn owner() {}\n/// beta calendar preference consumer.\npub fn consumer() {}\npub fn undocumented() {}\n";
const INPUTS: [&str; 2] = [
    "alpha calendar preference owner.",
    "beta calendar preference consumer.",
];

fn hash(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

struct Fixture {
    temp: tempfile::TempDir,
    endpoint: TcpListener,
    url: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = TcpListener::bind("127.0.0.1:0").unwrap();
        endpoint.set_nonblocking(true).unwrap();
        let url = format!("http://{}", endpoint.local_addr().unwrap());
        let fixture = Self {
            temp,
            endpoint,
            url,
        };
        for directory in ["src", ".home", ".codanna"] {
            std::fs::create_dir_all(fixture.root().join(directory)).unwrap();
        }
        std::fs::write(fixture.root().join("src/lib.rs"), SOURCE).unwrap();
        fixture.configure(true, Some(2), "");
        fixture
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn configure(&self, enabled: bool, dimension: Option<usize>, extra: &str) {
        let root = serde_json::to_string(&self.root().to_string_lossy()).unwrap();
        let source = serde_json::to_string(&self.root().join("src").to_string_lossy()).unwrap();
        let dim = dimension
            .map(|value| format!("remote_dim = {value}\n"))
            .unwrap_or_default();
        std::fs::write(self.root().join(".codanna/settings.toml"), format!(
            "workspace_root = {root}\nindex_path = \".codanna/index\"\n[indexing]\nindexed_paths = [{source}]\n[semantic_search]\nenabled = {enabled}\nremote_url = {:?}\nremote_model = \"offline-fixture\"\n{dim}{extra}\n", self.url
        )).unwrap();
    }

    fn seed(&self, revision: Option<&str>) {
        let identity = json!({
            "backend": "remote", "model": "offline-fixture", "endpoint_sha256": hash(&self.url),
            "model_revision": revision, "input_policy": "complete-input-v2:utf8-byte-budget-proxy:8192"
        }).to_string();
        let cache = json!({
            "format_version": 1, "preprocessing_version": 2,
            "model_identity": identity, "dimension": 2,
            "entries": INPUTS.iter().map(|input| json!({
                "input_sha256": hash(input), "embedding": [1.0, 0.0]
            })).collect::<Vec<_>>()
        });
        let path = self
            .root()
            .join(".codanna/index/semantic/embedding-cache.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, cache.to_string()).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        let before = tree(self.root());
        let mut command = Command::new(env!("CARGO_BIN_EXE_codanna-index-plan"));
        command.env_clear().env("HOME", self.root().join(".home"));
        // No real credential enters the child. A forbidden backend startup would
        // hit the listener and fail the test even if it swallowed a network error.
        command.env("CODANNA_EMBED_API_KEY", "synthetic-do-not-send");
        for variable in [
            "PATH",
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
            "SYSTEMROOT",
            "WINDIR",
        ] {
            if let Some(value) = std::env::var_os(variable) {
                command.env(variable, value);
            }
        }
        let output = command
            .current_dir(self.root())
            .args(["--config", ".codanna/settings.toml"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            tree(self.root()),
            before,
            "preflight modified the workspace"
        );
        assert!(
            matches!(self.endpoint.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "preflight contacted the embedding endpoint"
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-do-not-send"));
        output
    }

    fn plan(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, Option<std::time::SystemTime>)> {
    let mut result = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            result.insert(path.clone(), (None, metadata.modified().ok()));
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else {
            let bytes = std::fs::read(&path).unwrap();
            result.insert(path, (Some(bytes), metadata.modified().ok()));
        }
    }
    result
}

#[test]
fn missing_index_is_not_created_and_counts_are_not_token_claims() {
    let fixture = Fixture::new();
    let report = fixture.plan(&[]);
    assert_eq!(report["status"], "complete");
    assert_eq!(report["files_parsed"], 1);
    assert_eq!(report["symbols"], 3);
    assert_eq!(report["symbols_without_embedding_input"], 1);
    assert_eq!(report["embedding_candidates"], 2);
    assert_eq!(report["snapshot_miss_inputs"], 2);
    assert_eq!(report["cache_present"], false);
    assert_eq!(report["provider_requests_made"], 0);
    assert!(report["exact_provider_tokens"].is_null());
    assert!(!fixture.root().join(".codanna/index").exists());
}

#[test]
fn exact_cache_matches_track_current_source_and_not_old_symbol_ids() {
    let fixture = Fixture::new();
    fixture.seed(None);
    let before = fixture.plan(&[]);
    assert_eq!(before["snapshot_hit_inputs"], 2);
    assert_eq!(before["snapshot_miss_inputs"], 0);
    std::fs::write(
        fixture.root().join("src/lib.rs"),
        SOURCE.replace("beta calendar", "updated calendar"),
    )
    .unwrap();
    let after = fixture.plan(&[]);
    assert_eq!(after["snapshot_hit_inputs"], 1);
    assert_eq!(after["snapshot_unique_miss_inputs"], 1);
    assert_ne!(after["source_fingerprint"], before["source_fingerprint"]);
    assert!(
        !serde_json::to_string(&after)
            .unwrap()
            .contains("alpha calendar preference owner")
    );
    assert!(
        !serde_json::to_string(&after)
            .unwrap()
            .contains(&fixture.url)
    );
}

#[test]
fn overlap_ignores_and_duplicate_inputs_do_not_inflate_unique_cost() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root().join("src/copy.rs"), SOURCE).unwrap();
    std::fs::write(fixture.root().join("src/ignored.rs"), SOURCE).unwrap();
    std::fs::write(fixture.root().join("src/.codannaignore"), "ignored.rs\n").unwrap();
    let report = fixture.plan(&["src", "src/copy.rs", "src"]);
    assert_eq!(report["files_parsed"], 2);
    assert_eq!(report["embedding_candidates"], 4);
    assert_eq!(report["unique_embedding_inputs"], 2);
    assert_eq!(report["duplicate_embedding_inputs"], 2);
    assert_eq!(report["snapshot_unique_miss_inputs"], 2);
    assert_eq!(report["snapshot_miss_inputs"], 4);
}

#[test]
fn corrupt_and_incompatible_cache_entries_do_not_become_hits() {
    let fixture = Fixture::new();
    fixture.seed(Some("old-revision"));
    assert_eq!(fixture.plan(&[])["snapshot_hit_inputs"], 0);
    std::fs::write(
        fixture
            .root()
            .join(".codanna/index/semantic/embedding-cache.json"),
        "not json",
    )
    .unwrap();
    assert_eq!(fixture.plan(&[])["snapshot_miss_inputs"], 2);
    fixture.seed(None);
    fixture.configure(true, Some(3), "");
    assert_eq!(fixture.plan(&[])["snapshot_hit_inputs"], 0);
}

#[test]
fn disabled_or_unknown_dimension_does_not_probe_or_pretend_to_know_reuse() {
    let fixture = Fixture::new();
    fixture.configure(false, Some(2), "");
    let disabled = fixture.plan(&[]);
    assert_eq!(disabled["backend"], "disabled");
    assert!(disabled["snapshot_hit_inputs"].is_null());
    fixture.configure(true, None, "");
    let unknown = fixture.plan(&[]);
    assert_eq!(
        unknown["cache_lookup"],
        "remote_dimension_unknown_without_probe"
    );
    assert!(unknown["snapshot_hit_inputs"].is_null());
    assert_eq!(unknown["future_probe_may_cost_tokens"], true);
}

#[test]
fn over_budget_inputs_produce_a_nonzero_blocked_report() {
    let fixture = Fixture::new();
    fixture.configure(true, Some(2), "max_input_tokens = 3");
    let output = fixture.run(&[]);
    assert_eq!(output.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "blocked");
    assert_eq!(report["over_budget_inputs"], 2);
}

#[test]
fn invalid_roots_and_settings_fail_without_a_successful_empty_report() {
    let fixture = Fixture::new();
    assert!(!fixture.run(&["missing"]).status.success());
    let outside = tempfile::tempdir().unwrap();
    assert!(
        !fixture
            .run(&[outside.path().to_str().unwrap()])
            .status
            .success()
    );
    std::fs::write(
        fixture.root().join(".codanna/settings.toml"),
        "not valid = [",
    )
    .unwrap();
    let output = fixture.run(&[]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn local_identity_stays_unknown_without_initializing_a_model() {
    let fixture = Fixture::new();
    let config = fixture.root().join(".codanna/settings.toml");
    let text = std::fs::read_to_string(&config)
        .unwrap()
        .replace(&format!("remote_url = {:?}\n", fixture.url), "");
    std::fs::write(config, text).unwrap();
    let report = fixture.plan(&[]);
    assert_eq!(report["backend"], "local");
    assert_eq!(report["cache_lookup"], "local_model_identity_not_loaded");
    assert!(report["snapshot_hit_inputs"].is_null());
    assert!(report["over_budget_inputs"].is_null());
}

#[test]
fn generic_grammar_files_are_partial_not_silently_downloaded() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.root().join("src/module.lua"),
        "function calendar() return 1 end\n",
    )
    .unwrap();
    let output = fixture.run(&[]);
    assert_eq!(output.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "partial");
    assert_eq!(report["files_discovered"], 2);
    assert_eq!(report["files_parsed"], 1);
    assert_eq!(report["files_requiring_generic_parser"], 1);
}

#[test]
fn corrupt_index_documents_are_not_opened_by_source_planning() {
    let fixture = Fixture::new();
    fixture.seed(None);
    let index = fixture.root().join(".codanna/index/tantivy");
    std::fs::create_dir_all(&index).unwrap();
    std::fs::write(index.join("meta.json"), "deliberately corrupt index").unwrap();
    let report = fixture.plan(&[]);
    assert_eq!(report["snapshot_hit_inputs"], 2);
    assert_eq!(report["status"], "complete");
}
