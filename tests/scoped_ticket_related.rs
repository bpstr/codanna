//! Public ticket handler controls for scoped one-hop evidence. No provider/model.
use codanna::indexing::facade::IndexFacade;
use codanna::indexing::pipeline::{FileRegistration, ResolvedRelationship, stages::WriteStage};
use codanna::mcp::CodeIntelligenceServer;
use codanna::parsing::LanguageId;
use codanna::{FileId, Range, RelationKind, Settings, Symbol, SymbolId, SymbolKind};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture(dense: bool) -> (tempfile::TempDir, CodeIntelligenceServer) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    let facade = IndexFacade::new(Arc::new(settings)).unwrap();
    let storage = facade.document_index();
    storage.start_batch().unwrap();
    let paths = [
        "src/active/entry.rs".to_string(),
        "src/active/nested.rs".to_string(),
        "src/active-neighbor/edge.rs".to_string(),
        "src/shared.rs".to_string(),
        temp.path()
            .join("external/src/active/entry.rs")
            .to_string_lossy()
            .into_owned(),
        "src/active/entry.rs.backup".to_string(),
        root.join("src/active/legacy.rs")
            .to_string_lossy()
            .into_owned(),
        "src/active/unregistered.rs".to_string(),
    ];
    for (slot, path) in paths.iter().enumerate().take(7) {
        storage
            .store_file_registration(&FileRegistration {
                path: path.as_str().into(),
                file_id: FileId::new(slot as u32 + 1).unwrap(),
                content_hash: "fixture".into(),
                language_id: LanguageId::new("rust"),
                timestamp: 0,
                mtime: 0,
            })
            .unwrap();
    }
    for (id, file, name) in [
        (1, 1, "scopeRouter"),
        (2, 1, "allowedImplementation"),
        (3, 2, "nestedImplementation"),
        (4, 3, "neighborImplementation"),
        (5, 4, "sharedImplementation"),
        (6, 5, "externalImplementation"),
        (7, 6, "backupImplementation"),
        (8, 7, "legacyImplementation"),
        (9, 8, "unregisteredImplementation"),
    ] {
        let symbol = Symbol::new(
            SymbolId::new(id).unwrap(),
            name,
            SymbolKind::Function,
            FileId::new(file).unwrap(),
            Range::new(id, 0, id, 20),
        );
        storage
            .index_symbol(&symbol, &paths[file as usize - 1])
            .unwrap();
    }
    let edges: Vec<u32> = if dense {
        (100..133).collect()
    } else {
        (2..=9).collect()
    };
    let mut writer = WriteStage::new(storage.clone());
    for id in edges {
        if dense {
            let symbol = Symbol::new(
                SymbolId::new(id).unwrap(),
                format!("outside{id}"),
                SymbolKind::Function,
                FileId::new(3).unwrap(),
                Range::new(id, 0, id, 20),
            );
            storage.index_symbol(&symbol, &paths[2]).unwrap();
        }
        writer
            .write_one(ResolvedRelationship::new(
                SymbolId::new(1).unwrap(),
                SymbolId::new(id).unwrap(),
                RelationKind::Calls,
            ))
            .unwrap();
    }
    writer.flush().unwrap();
    (temp, CodeIntelligenceServer::new(facade))
}

async fn pair(server: &CodeIntelligenceServer, scope: &str) -> Value {
    for attempt in 0..3 {
        let query = json!({"query":"scopeRouter", "code_limit":1, "code_path_prefix":scope});
        let original = server
            .search_ticket_context(Parameters(serde_json::from_value(query.clone()).unwrap()))
            .await
            .unwrap();
        assert_ne!(original.is_error, Some(true));
        let direct = original.structured_content.unwrap();
        assert!(direct["code"].get("related_code").is_none());
        let mut enabled = query;
        enabled["include_related_code"] = true.into();
        let response = server
            .search_ticket_context(Parameters(serde_json::from_value(enabled).unwrap()))
            .await
            .unwrap();
        assert_ne!(response.is_error, Some(true));
        let result = response.structured_content.unwrap();
        let status = result["code"]["related_code"]["status"].as_str().unwrap();
        if matches!(
            status,
            "not_run_generation_mismatch" | "discarded_generation_changed"
        ) {
            assert!(
                result["code"]["related_code"]["items"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            println!("scoped_related_generation_retry={attempt} status={status}");
            continue;
        }
        assert_eq!(direct["code"]["items"], result["code"]["items"]);
        assert_eq!(result["documents"]["status"], "not_configured");
        assert_eq!(result["code"]["semantic_status"], "not_requested");
        assert_eq!(result["conversations"]["requested"], false);
        let text = serde_json::to_value(&response.content).unwrap().to_string();
        assert!(!text.contains("externalImplementation"));
        return result;
    }
    panic!("Reader did not stabilize within the bounded measurement attempts");
}

fn ids(result: &Value) -> Vec<u64> {
    let mut ids: Vec<_> = result["code"]["related_code"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["symbol_id"].as_u64().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn scoped_related_subtree_excludes_external_neighbors_and_unregistered_targets() {
    let (_temp, server) = fixture(false);
    for prefix in ["src/active", r"src\active"] {
        let data = pair(&server, prefix).await;
        assert_eq!(ids(&data), vec![2, 3, 7, 8]);
        assert_eq!(
            data["code"]["related_code"]["probes"][0]["excluded_by_scope"],
            4
        );
        assert_eq!(
            data["code"]["related_code"]["probes"][0]["indexed_edges"],
            8
        );
    }
}

#[tokio::test]
async fn scoped_related_root_keeps_workspace_dependencies_but_not_external_roots() {
    let (_temp, server) = fixture(false);
    assert_eq!(ids(&pair(&server, ".").await), vec![2, 3, 4, 5, 7, 8]);
}

#[tokio::test]
async fn scoped_related_exact_file_excludes_backup_and_does_not_fallback_for_missing_scope() {
    let (_temp, server) = fixture(false);
    assert_eq!(ids(&pair(&server, "src/active/entry.rs").await), vec![2]);
    let missing = pair(&server, "src/missing").await;
    assert!(ids(&missing).is_empty());
    assert!(missing["code"]["items"].as_array().unwrap().is_empty());
    assert_eq!(
        missing["code"]["related_code"]["status"],
        "not_run_no_direct_matches"
    );
}

#[tokio::test]
async fn scoped_related_does_not_relax_edge_budget_even_when_every_target_is_outside_scope() {
    let (_temp, server) = fixture(true);
    let result = pair(&server, "src/active").await;
    assert_eq!(result["code"]["related_code"]["status"], "partial");
    assert_eq!(
        result["code"]["related_code"]["probes"][0]["status"],
        "edge_budget_exceeded"
    );
    assert!(ids(&result).is_empty());
}

#[tokio::test]
async fn scoped_related_invalid_requests_fail_before_optional_retrieval() {
    let (_temp, server) = fixture(false);
    for scope in ["", "../external", "/tmp", r"C:\external"] {
        let response = server
            .search_ticket_context(Parameters(
                serde_json::from_value(json!({
                    "query":"scopeRouter", "include_related_code":true, "code_path_prefix":scope,
                }))
                .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.is_error, Some(true));
        assert!(response.structured_content.is_none());
    }
}
