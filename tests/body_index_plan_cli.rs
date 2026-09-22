//! Offline body-policy planning through a real, isolated CLI process.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SOURCE: &str = "/// alpha owner.\npub fn owner() {}\n/// beta consumer.\npub fn consumer() {}\npub fn undocumented() {}\n";

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
        let fixture = Self { temp, endpoint, url };
        for path in ["src", ".home", ".codanna"] {
            std::fs::create_dir_all(fixture.root().join(path)).unwrap();
        }
        std::fs::write(fixture.root().join("src/lib.rs"), SOURCE).unwrap();
        fixture.configure(true, "", true);
        fixture
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn configure(&self, remote: bool, extra: &str, enabled: bool) {
        let root = serde_json::to_string(&self.root().to_string_lossy()).unwrap();
        let source = serde_json::to_string(&self.root().join("src").to_string_lossy()).unwrap();
        let endpoint = if remote {
            format!("remote_url = {:?}\nremote_dim = 2\n", self.url)
        } else {
            String::new()
        };
        std::fs::write(self.root().join(".codanna/settings.toml"), format!(
            "workspace_root = {root}\nindex_path = \".codanna/index\"\n[indexing]\nindexed_paths = [{source}]\n[semantic_search]\nenabled = {enabled}\ncode_representation = \"symbol_body_v1\"\nremote_model = \"offline-body-fixture\"\n{endpoint}{extra}\n"
        )).unwrap();
    }

    fn run(&self) -> (Output, Value) {
        let before = tree(self.root());
        let mut command = Command::new(env!("CARGO_BIN_EXE_codanna-index-plan"));
        command.env_clear().env("HOME", self.root().join(".home"));
        for key in ["PATH", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "SYSTEMROOT", "WINDIR"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let output = command
            .current_dir(self.root())
            .args(["--config", ".codanna/settings.toml"])
            .output()
            .unwrap();
        assert_eq!(tree(self.root()), before, "planner wrote workspace data");
        assert!(matches!(self.endpoint.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock), "planner contacted the provider");
        let report: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        assert!(!output.stdout.windows(self.url.len()).any(|part| part == self.url.as_bytes()));
        assert_eq!(report["provider_requests_made"], 0);
        assert!(report["exact_provider_tokens"].is_null());
        (output, report)
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, Option<std::time::SystemTime>)> {
    let mut result = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            result.insert(path.clone(), (None, metadata.modified().ok()));
            for entry in std::fs::read_dir(path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else {
            result.insert(path.clone(), (Some(std::fs::read(path).unwrap()), metadata.modified().ok()));
        }
    }
    result
}

#[test]
fn body_plan_counts_undocumented_parents_without_creating_an_index() {
    let fixture = Fixture::new();
    let (output, report) = fixture.run();
    assert!(output.status.success());
    assert_eq!(report["symbols"], 3);
    assert_eq!(report["embedding_candidates"], 3);
    assert_eq!(report["eligible_symbols"], 3);
    assert_eq!(report["body_sources"], 3);
    assert_eq!(report["embedding_inputs"], 3);
    assert_eq!(report["snapshot_miss_inputs"], 3);
    assert_eq!(report["code_representation"], "symbol_body_v1");
    assert!(!fixture.root().join(".codanna/index").exists());
}

#[test]
fn body_plan_does_not_accept_comment_only_cache_identity() {
    let fixture = Fixture::new();
    let identity = json!({"backend":"remote", "model":"offline-body-fixture", "endpoint_sha256":hash(&fixture.url), "model_revision":null, "input_policy":"complete-input-v2:utf8-byte-budget-proxy:8192"}).to_string();
    let cache = json!({"format_version":1, "preprocessing_version":2, "model_identity":identity, "dimension":2,
        "entries":[{"input_sha256":hash("alpha owner."),"embedding":[1.0,0.0]}, {"input_sha256":hash("beta consumer."),"embedding":[1.0,0.0]}]});
    let path = fixture.root().join(".codanna/index/semantic/embedding-cache.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, cache.to_string()).unwrap();
    let (output, report) = fixture.run();
    assert!(output.status.success());
    assert_eq!(report["snapshot_hit_inputs"], 0);
    assert_eq!(report["snapshot_unique_miss_inputs"], 3);
}

#[test]
fn body_plan_counts_segment_headers_separately_from_retained_source() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root().join("src/lib.rs"), format!("pub fn body_owner() {{ {} }}\n", "consume(); ".repeat(240))).unwrap();
    fixture.configure(true, "max_input_tokens = 2048", true);
    let (output, report) = fixture.run();
    assert!(output.status.success());
    assert_eq!(report["embedding_candidates"], 1);
    let inputs = report["embedding_inputs"].as_u64().unwrap();
    assert!((2..=8).contains(&inputs));
    assert!(report["embedding_input_bytes"].as_u64().unwrap() > report["retained_representation_bytes"].as_u64().unwrap());
}

#[test]
fn body_plan_unknown_tokenizer_keeps_segment_cost_unknown() {
    let fixture = Fixture::new();
    fixture.configure(false, "", true);
    let (output, report) = fixture.run();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(report["status"], "partial");
    assert_eq!(report["body_sources"], 3);
    assert_eq!(report["embedding_candidates"], 3);
    assert!(report["retained_representation_bytes"].as_u64().unwrap() > 0);
    for field in ["embedding_inputs", "embedding_input_bytes", "unique_embedding_inputs", "snapshot_hit_inputs"] {
        assert!(report[field].is_null(), "{field} must be unknown");
    }
}

#[test]
fn body_plan_header_budget_failure_is_not_a_successful_empty_plan() {
    let fixture = Fixture::new();
    fixture.configure(true, "max_input_tokens = 3", true);
    let (output, report) = fixture.run();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(report["status"], "blocked");
    assert_eq!(report["input_policy_rejections"], 3);
    assert_eq!(report["embedding_candidates"], 3);
    assert_eq!(report["embedding_inputs"], 0);
}
