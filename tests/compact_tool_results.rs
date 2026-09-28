//! Offline, temporary-index witnesses for the compact model-facing contract.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore};
use codanna::indexing::facade::IndexFacade;
use codanna::mcp::{CodeIntelligenceServer, GetDocumentChunkRequest, SearchDocumentsRequest};
use codanna::vector::{EmbeddingGenerator, VectorDimension, VectorError};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::RwLock;

struct Prepared(Arc<AtomicUsize>);
impl EmbeddingGenerator for Prepared {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }
    fn cache_identity(&self) -> String {
        "compact-output-offline-fixture@1".into()
    }
}
struct Fixture {
    temp: tempfile::TempDir,
    server: CodeIntelligenceServer,
    store: Arc<RwLock<DocumentStore>>,
    collection: CollectionConfig,
    calls: Arc<AtomicUsize>,
}
fn chunks() -> ChunkingConfig {
    ChunkingConfig {
        min_chunk_chars: 1,
        max_chunk_chars: 20_000,
        overlap_chars: 0,
        ..Default::default()
    }
}
impl Fixture {
    fn new(contents: &str, count: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = codanna::Settings {
            workspace_root: Some(temp.path().to_path_buf()),
            index_path: temp.path().join("index"),
            ..Default::default()
        };
        settings.semantic_search.enabled = false;
        settings.documents.enabled = true;
        let paths: Vec<_> = (0..count)
            .map(|i| {
                let path = temp.path().join(format!("guide-{i}.md"));
                std::fs::write(&path, contents).unwrap();
                path
            })
            .collect();
        let collection = CollectionConfig {
            paths,
            ..Default::default()
        };
        settings
            .documents
            .collections
            .insert("docs".into(), collection.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut store = DocumentStore::new(
            settings.index_path.join("documents"),
            VectorDimension::new(2).unwrap(),
        )
        .unwrap()
        .with_embeddings(Box::new(Prepared(calls.clone())))
        .unwrap();
        store
            .index_collection("docs", &collection, &chunks())
            .unwrap();
        calls.store(0, Ordering::SeqCst);
        let store = Arc::new(RwLock::new(store));
        let facade = IndexFacade::new(Arc::new(settings)).unwrap();
        let server = CodeIntelligenceServer::new(facade).with_document_store_arc(store.clone());
        Self {
            temp,
            server,
            store,
            collection,
            calls,
        }
    }
    async fn search(&self, extra: Value) -> Value {
        let mut args = json!({"query":"reconnect", "literal":true,"collection":"docs"});
        for (key, value) in extra.as_object().unwrap() {
            args[key] = value.clone();
        }
        let request: SearchDocumentsRequest = serde_json::from_value(args).unwrap();
        let budget = request.max_output_bytes.bytes();
        let result = self
            .server
            .search_documents(Parameters(request))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        assert!(serde_json::to_vec(&result).unwrap().len() <= budget);
        result.structured_content.unwrap()
    }
    async fn read(
        &self,
        id: Value,
        generation: Value,
        offset: usize,
        limit: u32,
        budget: usize,
    ) -> Value {
        let request:GetDocumentChunkRequest=serde_json::from_value(json!({
            "chunk_id":id,"document_generation":generation,"line_offset":offset,"line_limit":limit,"max_output_bytes":budget
        })).unwrap();
        let result = self
            .server
            .get_document_chunk(Parameters(request))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        assert!(serde_json::to_vec(&result).unwrap().len() <= budget);
        result.structured_content.unwrap()
    }
}

#[tokio::test]
async fn compact_documents_are_budgeted_addressable_and_keep_candidates_distinct() {
    let f = Fixture::new(&format!("reconnect: {}", "quoted \\\"雪🦀 ".repeat(80)), 30);
    let data = f.search(json!({"limit":30,"max_output_bytes":4096})).await;
    let rows = data["results"].as_array().unwrap();
    assert!(!rows.is_empty() && rows.len() < 30, "{data}");
    assert_eq!(data["output"]["partial"], true);
    assert_eq!(data["retrieval"]["returned_chunks"], rows.len());
    assert_eq!(data["retrieval"]["retrieved_chunks"], 30);
    assert_eq!(data["retrieval"]["support_status"], "not_assessed");
    assert_eq!(data["retrieval"]["source_freshness"], "unchecked");
    assert!(data["retrieval"]["document_generation"].is_string());
    for row in rows {
        assert!(row["chunk_id"].as_u64().unwrap() > 0);
        assert!(row["byte_range"].is_array());
        assert!(row["source_path"].as_str().unwrap().starts_with("guide-"));
        assert!(row["content_preview"].as_str().unwrap().chars().count() <= 280);
    }
    assert_eq!(
        f.calls.load(Ordering::SeqCst),
        0,
        "literal reads never embed"
    );
}

#[tokio::test]
async fn document_filter_restricts_one_source_and_rejects_escapes() {
    let f = Fixture::new("reconnect requirements in a guide", 3);
    let data = f.search(json!({"document":"guide-1.md"})).await;
    let rows = data["results"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["source_path"], "guide-1.md");
    for path in ["../secret.md", "/etc/passwd", "C:\\secret.md"] {
        let args = json!({"query":"reconnect","document":path,"literal":true});
        let result = f
            .server
            .search_documents(Parameters(serde_json::from_value(args).unwrap()))
            .await;
        assert!(result.is_err(), "{path}");
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn exact_chunk_read_pages_without_rewriting_text_or_reembedding() {
    let contents = "reconnect 雪\nKeep this \\\"quoted\\\" evidence.\nDo not merge the lines.\n";
    let f = Fixture::new(contents, 1);
    let search = f.search(json!({})).await;
    let row = &search["results"][0];
    let generation = search["retrieval"]["document_generation"].clone();
    let mut offset = 0;
    let mut recovered = String::new();
    for _ in 0..10 {
        let data = f
            .read(row["chunk_id"].clone(), generation.clone(), offset, 1, 4096)
            .await;
        let chunk = &data["chunk"];
        assert_eq!(chunk["line_numbers"], "relative_to_indexed_chunk");
        assert_eq!(chunk["byte_range"], row["byte_range"]);
        for line in chunk["lines"].as_array().unwrap() {
            recovered.push_str(line["text"].as_str().unwrap());
        }
        match chunk["next_offset"].as_u64() {
            None => break,
            Some(next) => {
                assert!(next as usize > offset);
                offset = next as usize;
            }
        }
    }
    let a = row["byte_range"][0].as_u64().unwrap() as usize;
    let b = row["byte_range"][1].as_u64().unwrap() as usize;
    assert_eq!(recovered, &contents[a..b]);
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn generation_change_rejects_old_handle_and_unindexed_edits_do_not_change_evidence() {
    let f = Fixture::new("reconnect before edit", 1);
    let search = f.search(json!({})).await;
    let id = search["results"][0]["chunk_id"].clone();
    let generation = search["retrieval"]["document_generation"].clone();
    std::fs::write(f.temp.path().join("guide-0.md"), "reconnect after edit").unwrap();
    let read = f.read(id.clone(), generation.clone(), 0, 80, 4096).await;
    assert!(read["chunk"]["lines"].to_string().contains("before edit"));
    assert_eq!(read["source_freshness"], "unchecked");
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    f.store
        .write()
        .await
        .index_collection("docs", &f.collection, &chunks())
        .unwrap();
    f.calls.store(0, Ordering::SeqCst);
    let request =
        serde_json::from_value(json!({"chunk_id":id,"document_generation":generation})).unwrap();
    let error = f
        .server
        .get_document_chunk(Parameters(request))
        .await
        .unwrap_err();
    assert!(error.message.contains("generation"));
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn oversized_source_line_is_not_cut_and_does_not_create_a_looping_cursor() {
    let f = Fixture::new(&format!("reconnect{}", "雪".repeat(3000)), 1);
    let search = f.search(json!({})).await;
    let data = f
        .read(
            search["results"][0]["chunk_id"].clone(),
            search["retrieval"]["document_generation"].clone(),
            0,
            80,
            4096,
        )
        .await;
    assert_eq!(data["chunk"]["lines"], json!([]));
    assert!(data["chunk"]["next_offset"].is_null());
    assert_eq!(data["output"]["partial"], true);
    assert_eq!(data["output"]["lists"]["/chunk/lines"]["omitted"], 1);
}

#[tokio::test]
async fn unified_search_keeps_document_handles_and_disables_recall_by_default() {
    let f = Fixture::new("reconnect requirements from indexed evidence", 1);
    let result = f
        .server
        .search_context(Parameters(
            serde_json::from_value(json!({"query":"reconnect","document":"guide-0.md"})).unwrap(),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert!(serde_json::to_vec(&result).unwrap().len() <= 8192);
    let data = result.structured_content.unwrap();
    assert_eq!(data["conversations"]["requested"], false);
    assert_eq!(data["conversations"]["items"], json!([]));
    assert!(data["documents"]["items"][0]["chunk_id"].is_number());
    assert!(data["documents"]["retrieval"]["document_generation"].is_string());
}

#[tokio::test]
async fn ticket_compact_keeps_document_identity_and_bounds_combined_payload() {
    let f = Fixture::new("reconnect policy `applyChanges`", 1);
    let result = f
        .server
        .search_ticket_context(Parameters(
            serde_json::from_value(json!({"query":"reconnect","document":"guide-0.md"})).unwrap(),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert!(serde_json::to_vec(&result).unwrap().len() <= 8192);
    let data = result.structured_content.unwrap();
    assert!(data["documents"]["items"][0]["chunk_id"].is_number());
    assert!(data["documents"]["items"][0]["byte_range"].is_array());
    assert!(data["documents"]["document_generation"].is_string());
    assert_eq!(data["conversations"]["requested"], false);
    assert_eq!(data["graph"]["freshness"], "unknown");
}
