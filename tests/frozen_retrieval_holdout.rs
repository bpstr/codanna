//! Frozen synthetic retrieval regression corpus. No provider or scaling claims.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore};
use codanna::indexing::facade::IndexFacade;
use codanna::vector::VectorDimension;
use codanna::{IndexPersistence, Settings};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CORPUS: &[(&str, &str)] = &[
    (
        "account.ts",
        include_str!("fixtures/retrieval_findings/f10/account.ts"),
    ),
    (
        "policy.md",
        include_str!("fixtures/retrieval_findings/f10/policy.md"),
    ),
    (
        "reference.ts",
        include_str!("fixtures/retrieval_findings/f10/reference.ts"),
    ),
];
const ORACLE: &str = include_str!("fixtures/retrieval_findings/f10/cases.json");
const MANIFEST: &str = include_str!("fixtures/retrieval_findings/f10/manifest.json");
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn frozen_holdout_manifest_covers_exact_corpus_and_independent_oracle() {
    let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["corpus_version"], "f10-v1");
    let files: BTreeMap<String, String> = CORPUS
        .iter()
        .map(|(path, text)| (path.to_string(), hash(text.as_bytes())))
        .collect();
    assert_eq!(
        manifest["source_sha256"],
        serde_json::to_value(&files).unwrap()
    );
    assert_eq!(manifest["oracle_sha256"], hash(ORACLE.as_bytes()));
    let identity = files
        .iter()
        .map(|(path, digest)| format!("{path}\0{digest}\n"))
        .collect::<String>();
    assert_eq!(manifest["corpus_sha256"], hash(identity.as_bytes()));
    assert_eq!(
        manifest["index_allowlist"],
        serde_json::json!(["account.ts", "policy.md", "reference.ts"])
    );
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/retrieval_findings/f10");
    let present: BTreeSet<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        present,
        BTreeSet::from(
            [
                "account.ts",
                "policy.md",
                "reference.ts",
                "cases.json",
                "manifest.json"
            ]
            .map(String::from)
        )
    );
}

fn identity(value: &Value) -> String {
    let symbol = value.get("symbol").unwrap_or(value);
    let path = symbol
        .get("file_path")
        .or_else(|| value.get("file_path"))
        .and_then(Value::as_str)
        .expect("symbol source path");
    format!(
        "{}:{}",
        Path::new(path).file_name().unwrap().to_string_lossy(),
        symbol["name"].as_str().expect("symbol name")
    )
}

#[test]
fn frozen_holdout_executes_every_oracle_against_reopened_indexes() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    assert_eq!(oracle["schema_version"], 1);
    assert_eq!(oracle["provider_requests"], 0);
    assert_eq!(oracle["corpus_version"], "f10-v1");
    let cases = oracle["cases"].as_array().unwrap();
    let ids: BTreeSet<_> = cases
        .iter()
        .map(|case| case["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        BTreeSet::from([
            "exact",
            "scoped",
            "caller",
            "decoy_callers",
            "callee",
            "positive_literal",
            "negative_literal",
            "negative_symbol"
        ])
    );
    assert_eq!(ids.len(), cases.len());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("corpus");
    std::fs::create_dir(&root).unwrap();
    for (path, text) in CORPUS {
        std::fs::write(root.join(path), text).unwrap();
    }
    // Only the allowlisted sources enter the scratch corpus. Oracle/receipts,
    // including their negative sentinel, are never indexed.
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: root.join(".codanna/index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.clone()).unwrap();
    settings.documents.enabled = true;
    let collection = CollectionConfig {
        paths: vec![root.join("policy.md")],
        ..Default::default()
    };
    settings
        .documents
        .collections
        .insert("policy".into(), collection.clone());
    let mut index = IndexFacade::new(Arc::new(settings.clone())).unwrap();
    index.index_directory(&root, true).unwrap();
    assert_eq!(index.symbol_count(), 3);
    let mut targets = BTreeMap::new();
    for name in ["accountWeekStart", "displayWeek"] {
        for symbol in index.find_symbols_by_name(name, Some("typescript")) {
            let key = format!(
                "{}:{}",
                Path::new(symbol.file_path.as_ref())
                    .file_name()
                    .unwrap()
                    .to_string_lossy(),
                symbol.name
            );
            assert!(targets.insert(key, symbol.id.0).is_none());
        }
    }
    IndexPersistence::new(settings.index_path.clone())
        .save_facade(&index)
        .unwrap();
    drop(index);
    let mut documents = DocumentStore::new(
        settings.index_path.join("documents"),
        VectorDimension::dimension_384(),
    )
    .unwrap();
    documents
        .index_collection(
            "policy",
            &collection,
            &ChunkingConfig {
                min_chunk_chars: 1,
                overlap_chars: 0,
                ..Default::default()
            },
        )
        .unwrap();
    drop(documents);
    let config = temp.path().join("settings.toml");
    std::fs::write(&config, toml::to_string(&settings).unwrap()).unwrap();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let tool = case["tool"].as_str().unwrap();
        let mut args = vec!["mcp".to_string(), tool.to_string()];
        for (key, value) in case["arguments"].as_object().unwrap() {
            args.push(format!(
                "{key}:{}",
                value
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| value.to_string())
            ));
        }
        if let Some(target) = case.get("target") {
            args.push(format!("symbol_id:{}", targets[target.as_str().unwrap()]));
        }
        args.push("--json".into());
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_codanna"))
            .arg("--config")
            .arg(&config)
            .args(args)
            .current_dir(&root)
            .env_clear()
            .env("HOME", temp.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(case["exit_code"].as_i64().unwrap() as i32),
            "{id}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        let data = if case["exit_code"] == 0 {
            assert_eq!(response["code"], "OK", "{id}");
            assert_eq!(response["status"], "success", "{id}");
            response["data"]
                .as_array()
                .expect("successful retrieval has array data")
                .clone()
        } else {
            assert_eq!(response["code"], "NOT_FOUND", "{id}");
            assert_eq!(response["status"], "not_found", "{id}");
            match response.get("data") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(data)) => data.clone(),
                _ => panic!("{id}: malformed not-found data"),
            }
        };
        let actual: BTreeSet<String> = if tool == "search_documents" {
            assert_eq!(response["meta"]["retrieval"]["mode"], "literal", "{id}");
            assert_eq!(
                response["meta"]["retrieval"]["support_status"], "not_assessed",
                "{id}"
            );
            data.iter()
                .map(|result| {
                    PathBuf::from(result["source_path"].as_str().unwrap())
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
        } else {
            data.iter().map(identity).collect()
        };
        let expected: BTreeSet<String> = serde_json::from_value(case["expected"].clone()).unwrap();
        assert_eq!(actual, expected, "{id}");
        assert_eq!(data.len(), expected.len(), "{id}: duplicate results");
        println!(
            "F10 f10-v1 {} case={id} passed",
            serde_json::from_str::<Value>(MANIFEST).unwrap()["corpus_sha256"]
                .as_str()
                .unwrap()
        );
    }
}
