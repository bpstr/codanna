//! Deterministic query regressions with explicitly injected local embeddings.
use super::*;
use crate::mcp::service::{FindSymbolTarget, page_symbols, try_resolve_find_symbol_target};
use crate::mcp::{
    CodeIntelligenceServer, FindSymbolRequest, GetIndexInfoRequest,
    SemanticSearchWithContextRequest,
};
use crate::{Range, ScopeContext};
use rmcp::handler::server::wrapper::Parameters;

fn fixture() -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let settings = Settings {
        index_path: temp.path().join("index"),
        semantic_search: crate::config::SemanticSearchConfig {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let facade = IndexFacade::new(Arc::new(settings)).unwrap();
    (temp, facade)
}

fn named_symbol(id: u32, name: &str, kind: SymbolKind) -> Symbol {
    Symbol::new(
        SymbolId::new(id).unwrap(),
        name,
        kind,
        FileId::new(1).unwrap(),
        Range::new(id, 0, id, 20),
    )
}

fn symbols(target: FindSymbolTarget) -> Vec<Symbol> {
    match target {
        FindSymbolTarget::Symbols { symbols, .. } => symbols,
        FindSymbolTarget::InvalidId(id) => panic!("invalid fixture ID: {id}"),
    }
}

#[test]
fn exact_lookup_boundaries_filter_before_pagination() {
    for count in [99u32, 100, 101, 1001] {
        let (_temp, facade) = fixture();
        let index = facade.document_index();
        index.start_batch().unwrap();
        for id in 1..=count {
            let mut symbol = named_symbol(id, "save", SymbolKind::Method).with_scope(
                ScopeContext::ClassMember {
                    class_name: Some(format!("Store{id}").into()),
                },
            );
            symbol.language_id = Some(crate::parsing::registry::LanguageId::new("python"));
            index.index_symbol(&symbol, "stores.py").unwrap();
        }
        let mut decoy = named_symbol(count + 1, "save", SymbolKind::Function);
        decoy.language_id = Some(crate::parsing::registry::LanguageId::new("rust"));
        index.index_symbol(&decoy, "decoy.rs").unwrap();
        index.commit_batch().unwrap();
        let all = symbols(try_resolve_find_symbol_target(&facade, "save", Some("python")).unwrap());
        assert_eq!(all.len(), count as usize);
        let (first, page) = page_symbols(all.clone(), 0, 100);
        assert_eq!(page.total, count as usize);
        assert_eq!(first.len(), count.min(100) as usize);
        assert_eq!(page.next_offset, (count > 100).then_some(100));
        let mut visited = Vec::new();
        let mut offset = 0;
        loop {
            let (items, page) = page_symbols(all.clone(), offset, 100);
            visited.extend(items.into_iter().map(|symbol| symbol.id));
            match page.next_offset {
                Some(next) => offset = next as u32,
                None => break,
            }
        }
        assert_eq!(
            visited,
            all.iter().map(|symbol| symbol.id).collect::<Vec<_>>()
        );
        let qualified = symbols(
            try_resolve_find_symbol_target(&facade, &format!("Store{count}.save"), Some("python"))
                .unwrap(),
        );
        assert_eq!(
            qualified.len(),
            1,
            "owner filter must see the last candidate at {count} matches"
        );
        let (_, empty) = page_symbols(all, count + 1, 100);
        assert_eq!(empty.total, count as usize);
        assert_eq!(empty.returned, 0);
        assert_eq!(empty.next_offset, None);
    }
}

#[tokio::test]
async fn symbol_listing_reports_total_and_empty_page_without_false_absence() {
    let (_temp, facade) = fixture();
    let index = facade.document_index();
    index.start_batch().unwrap();
    for id in 1..=101 {
        index
            .index_symbol(&named_symbol(id, "save", SymbolKind::Function), "stores.rs")
            .unwrap();
    }
    index.commit_batch().unwrap();
    let server = CodeIntelligenceServer::new(facade);
    for (offset, returned, next) in [(0, 100, Some(100)), (100, 1, None), (101, 0, None)] {
        let response = server
            .find_symbol(Parameters(FindSymbolRequest {
                name: "save".into(),
                lang: None,
                limit: 100,
                offset,
            }))
            .await
            .unwrap();
        assert_ne!(response.is_error, Some(true));
        let page = &response.structured_content.unwrap()["pagination"];
        assert_eq!(page["total"], 101);
        assert_eq!(page["returned"], returned);
        assert_eq!(page["next_offset"], serde_json::json!(next));
    }
}

#[test]
fn invalid_retrieve_search_limit_remains_an_error() {
    let (_temp, facade) = fixture();
    for format in [crate::io::OutputFormat::Text, crate::io::OutputFormat::Json] {
        for limit in [0, 1001] {
            assert_eq!(
                crate::retrieve::retrieve_search(
                    &facade, "anything", limit, None, None, None, format, None
                ),
                crate::io::ExitCode::GeneralError
            );
        }
        assert_eq!(
            crate::retrieve::retrieve_search(&facade, "absent", 10, None, None, None, format, None),
            crate::io::ExitCode::NotFound
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn semantic_context_preserves_results_when_one_impact_exceeds_budget() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let transport = tokio::spawn(async move {
        for _ in 0..2 {
            // Explicit backend probe and the one semantic query.
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 2048];
            loop {
                let read = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    socket.read(&mut buffer),
                )
                .await
                .unwrap()
                .unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
                if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    assert!(!headers.contains("authorization:"));
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = r#"{"data":[{"index":0,"embedding":[1.0,0.0]}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    // Construct the backend directly: provider environment variables cannot
    // redirect this fixture or attach a provider credential.
    let backend = RemoteEmbedder::new(&format!("http://{address}"), "fixture", Some(2), None)
        .await
        .unwrap();
    let (_temp, mut facade) = fixture();
    let index = facade.document_index();
    index.start_batch().unwrap();
    for id in 1..=1003 {
        let name = match id {
            1 => "popular_helper".into(),
            2 => "isolated_helper".into(),
            _ => format!("caller{id}"),
        };
        index
            .index_symbol(&named_symbol(id, &name, SymbolKind::Function), "fixture.rs")
            .unwrap();
        if id > 2 {
            index
                .store_relationship(
                    SymbolId::new(id).unwrap(),
                    SymbolId::new(1).unwrap(),
                    &Relationship::new(RelationKind::Calls),
                )
                .unwrap();
        }
    }
    index.commit_batch().unwrap();
    let mut semantic = SimpleSemanticSearch::new_empty(2, "fixture");
    semantic
        .set_embedding_identity(backend.identity(None))
        .unwrap();
    semantic.store_embeddings(vec![
        (SymbolId::new(1).unwrap(), vec![1., 0.], "rust".into()),
        (SymbolId::new(2).unwrap(), vec![0.8, 0.6], "rust".into()),
    ]);
    facade.semantic_search = Some(Arc::new(Mutex::new(semantic)));
    assert!(
        facade
            .embedding_pool
            .set(Arc::new(EmbeddingBackend::Remote(Arc::new(backend))))
            .is_ok()
    );
    let server = CodeIntelligenceServer::new(facade);
    let response = server
        .semantic_search_with_context(Parameters(SemanticSearchWithContextRequest {
            query: "helper".into(),
            limit: 2,
            threshold: Some(0.),
            lang: None,
        }))
        .await
        .unwrap();
    assert_ne!(response.is_error, Some(true));
    let text = response
        .content
        .iter()
        .filter_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("popular_helper") && text.contains("isolated_helper"));
    let metadata = response.structured_content.unwrap();
    let impacts = metadata["impact"].as_array().unwrap();
    assert_eq!(impacts.len(), 2);
    let popular = impacts
        .iter()
        .find(|impact| impact["symbol_id"] == 1)
        .unwrap();
    assert_eq!(popular["status"], "budget_exceeded");
    assert!(
        popular.get("count").is_none(),
        "an unavailable traversal is not an empty graph"
    );
    let isolated = impacts
        .iter()
        .find(|impact| impact["symbol_id"] == 2)
        .unwrap();
    assert_eq!(isolated["status"], "complete");
    assert_eq!(isolated["count"], 0);
    transport.await.unwrap();
}

#[tokio::test]
async fn reference_context_surfaces_remain_distinct_from_calls() {
    let (_temp, facade) = fixture();
    let index = facade.document_index();
    let handle = named_symbol(1, "handle", SymbolKind::Function);
    let install = named_symbol(2, "install", SymbolKind::Function);
    let bootstrap = named_symbol(3, "bootstrap", SymbolKind::Function);
    index.start_batch().unwrap();
    for symbol in [&handle, &install, &bootstrap] {
        index.index_symbol(symbol, "routes.ts").unwrap();
    }
    index
        .store_relationship(
            install.id,
            handle.id,
            &Relationship::new(RelationKind::References).with_metadata(
                crate::relationship::RelationshipMetadata::new()
                    .at_position(7, 12)
                    .with_context("argument_reference"),
            ),
        )
        .unwrap();
    index
        .store_relationship(
            bootstrap.id,
            install.id,
            &Relationship::new(RelationKind::Calls),
        )
        .unwrap();
    index.commit_batch().unwrap();

    assert_eq!(
        facade.get_dependencies(install.id)[&RelationKind::References][0].id,
        handle.id
    );
    assert_eq!(
        facade.get_dependents(handle.id)[&RelationKind::References][0].id,
        install.id
    );
    assert!(facade.get_calling_functions(handle.id).is_empty());
    assert_eq!(
        facade.get_impact_radius(handle.id, Some(2)),
        vec![install.id, bootstrap.id]
    );
    assert_eq!(
        facade.get_relationships_for_symbol(handle.id).unwrap()[0]
            .2
            .kind,
        RelationKind::References
    );
    let context = facade
        .get_symbol_context(handle.id, ContextIncludes::SYMBOL_CARD)
        .unwrap();
    assert!(context.relationships.called_by.is_none());
    let json = serde_json::to_value(&context).unwrap();
    assert_eq!(json["relationships"]["referenced_by"][0][1]["line"], 8);
    assert_eq!(json["relationships"]["referenced_by"][0][1]["column"], 12);
    let description = context.to_string();
    assert!(description.contains("Referenced by 1 symbol(s)"));
    assert!(description.contains("routes.ts:8") && description.contains("argument_reference"));

    let server = CodeIntelligenceServer::new(facade);
    let response = server
        .analyze_impact(Parameters(crate::mcp::AnalyzeImpactRequest {
            symbol_name: Some("handle".into()),
            symbol_id: None,
            max_depth: 2,
        }))
        .await
        .unwrap();
    let text = response
        .content
        .iter()
        .filter_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("direct callers: 0"));
    assert!(text.contains("referenced by: 1"));
    assert!(text.contains("install") && text.contains("bootstrap"));
    let response = server
        .find_symbol(Parameters(FindSymbolRequest {
            name: "handle".into(),
            lang: None,
            limit: 100,
            offset: 0,
        }))
        .await
        .unwrap();
    let text = response
        .content
        .iter()
        .filter_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Referenced by: 1 symbol(s)"));
}

#[test]
fn semantic_coverage_distinguishes_eligibility_missing_and_orphan_vectors() {
    let (_temp, mut facade) = fixture();
    let index = facade.document_index();
    index.start_batch().unwrap();

    let mut eligible_with_vector = named_symbol(1, "eligible_with_vector", SymbolKind::Function);
    eligible_with_vector.doc_comment = Some("calendar preference documentation".into());
    let mut eligible_without_vector =
        named_symbol(2, "eligible_without_vector", SymbolKind::Function);
    eligible_without_vector.doc_comment = Some("workspace settings documentation".into());
    let ineligible = named_symbol(3, "ineligible", SymbolKind::Function);
    for symbol in [&eligible_with_vector, &eligible_without_vector, &ineligible] {
        index.index_symbol(symbol, "coverage.rs").unwrap();
    }
    index.commit_batch().unwrap();

    let identity = serde_json::json!({
        "backend": "remote",
        "model": "fixture",
        "endpoint_sha256": "fixture-digest",
        "model_revision": null,
        "input_policy": "complete-input-v2:utf8-byte-budget-proxy:8192",
    })
    .to_string();
    let mut semantic = SimpleSemanticSearch::new_empty(2, "fixture");
    semantic.set_embedding_identity(identity).unwrap();
    semantic.store_embeddings(vec![
        (eligible_with_vector.id, vec![1.0, 0.0], "rust".into()),
        (SymbolId::new(99).unwrap(), vec![0.0, 1.0], "rust".into()),
    ]);
    facade.semantic_search = Some(Arc::new(Mutex::new(semantic)));

    let symbols = facade.get_all_symbols();
    let status = facade.semantic_coverage_status(&symbols);
    assert_eq!(status.state, "live");
    assert_eq!(status.total_symbols, 3);
    assert_eq!(status.eligible_symbols, 2);
    assert_eq!(status.vector_count, Some(2));
    assert_eq!(status.eligible_with_vector, Some(1));
    assert_eq!(status.eligible_without_vector, Some(1));
    assert_eq!(status.vector_without_current_symbol, Some(1));
    assert_eq!(status.skipped_symbols, None);
    assert_eq!(status.pending_symbols, None);
    assert_eq!(
        status.embedding_input_policy.as_deref(),
        Some("complete-input-v2:utf8-byte-budget-proxy:8192")
    );
    assert!(status.embedding_identity_sha256.is_some());
    assert_eq!(status.vector_code_generation, None);
    assert_eq!(status.generation_alignment, "unknown_untracked");
    assert_eq!(status.freshness, "unknown");
}

#[tokio::test]
async fn index_info_structures_semantic_coverage_without_initializing_a_provider() {
    let (_temp, mut facade) = fixture();
    let index = facade.document_index();
    index.start_batch().unwrap();
    let mut documented = named_symbol(1, "documented", SymbolKind::Function);
    documented.doc_comment = Some("documented symbol".into());
    index.index_symbol(&documented, "coverage.rs").unwrap();
    index
        .index_symbol(
            &named_symbol(2, "plain", SymbolKind::Function),
            "coverage.rs",
        )
        .unwrap();
    index.commit_batch().unwrap();

    let mut metadata = crate::semantic::SemanticMetadata::new_remote("fixture".into(), 2, 1);
    metadata.embedding_identity = Some(
        serde_json::json!({
            "backend": "remote",
            "model": "fixture",
            "endpoint_sha256": "fixture-digest",
            "model_revision": null,
            "input_policy": "complete-input-v2:utf8-byte-budget-proxy:8192",
        })
        .to_string(),
    );
    facade.semantic_metadata_snapshot = Some(metadata);

    let server = CodeIntelligenceServer::new(facade);
    let response = server
        .get_index_info(Parameters(GetIndexInfoRequest {}))
        .await
        .unwrap();
    assert_ne!(response.is_error, Some(true));
    let structured = response.structured_content.expect("structured index info");
    assert_eq!(structured["index"]["symbols"], 2);
    assert_eq!(structured["semantic"]["state"], "metadata_only");
    assert_eq!(structured["semantic"]["eligible_symbols"], 1);
    assert_eq!(structured["semantic"]["vector_count"], 1);
    assert!(structured["semantic"]["eligible_with_vector"].is_null());
    assert!(structured["semantic"]["eligible_without_vector"].is_null());
    assert!(structured["semantic"]["vector_code_generation"].is_null());
    assert_eq!(
        structured["semantic"]["generation_alignment"],
        "unknown_untracked"
    );

    let text = response
        .content
        .iter()
        .filter_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Source eligibility policy: doc_comment_present_v1"));
    assert!(text.contains("Eligible symbols: 1"));
    assert!(text.contains("Eligible with vector: unknown"));
    assert!(text.contains("Generation alignment: unknown_untracked"));
}

#[test]
fn semantic_coverage_disabled_is_not_reported_as_zero_vectors() {
    let (_temp, facade) = fixture();
    let status = facade.semantic_coverage_status(&[]);
    assert_eq!(status.state, "disabled");
    assert_eq!(status.vector_count, None);
    assert_eq!(status.eligible_with_vector, None);
    assert_eq!(status.vector_code_generation, None);
}

#[tokio::test]
async fn unavailable_semantic_query_names_lexical_fallback_without_rebuilding() {
    let (_temp, facade) = fixture();
    let server = CodeIntelligenceServer::new(facade);
    let response = server
        .semantic_search_docs(Parameters(SemanticSearchRequest {
            query: "calendar settings".into(),
            limit: 5,
            threshold: None,
            lang: None,
        }))
        .await
        .unwrap();
    assert_eq!(response.is_error, Some(true));
    let text = response
        .content
        .iter()
        .filter_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("No code or semantic index rebuild was attempted"));
    assert!(text.contains("search_symbols") && text.contains("search_context"));
}
