//! Candidate retrieval must not certify support for an absent requirement.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore, SearchQuery};
use codanna::vector::{EmbeddingGenerator, VectorDimension, VectorError};

struct Prepared;
impl EmbeddingGenerator for Prepared {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
    fn dimension(&self) -> VectorDimension { VectorDimension::new(2).unwrap() }
    fn cache_identity(&self) -> String { "support-fixture@1".into() }
}

#[tokio::test]
async fn nearest_neighbors_can_return_candidates_without_literal_support() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("policy.md");
    std::fs::write(&path, "Account preferences control calendar display.").unwrap();
    let mut store = DocumentStore::new(temp.path().join("index"), VectorDimension::new(2).unwrap())
        .unwrap().with_embeddings(Box::new(Prepared)).unwrap();
    store.index_collection("fixture", &CollectionConfig {
        paths: vec![path], ..Default::default()
    }, &ChunkingConfig { min_chunk_chars: 1, ..Default::default() }).unwrap();
    assert_eq!(store.retrieval_mode(), "semantic_nearest_neighbors");
    let results = store.search(SearchQuery {
        text: "Every violet narwhal must dance seventeen polkas".into(),
        ..Default::default()
    }).unwrap();
    assert!(!results.is_empty(), "prepared vectors deliberately return unrelated candidates");
    assert!(results.iter().all(|result| !result.content_preview.contains("narwhal")));
    let mut settings = codanna::Settings { index_path: temp.path().join("code"), ..Default::default() };
    settings.semantic_search.enabled = false;
    let facade = codanna::indexing::facade::IndexFacade::new(std::sync::Arc::new(settings)).unwrap();
    let server = codanna::mcp::CodeIntelligenceServer::new(facade).with_document_store(store);
    for (collection, expected) in [(None, "returned"), (Some("absent".to_string()), "none")] {
        let response = server.search_documents(rmcp::handler::server::wrapper::Parameters(
            codanna::mcp::SearchDocumentsRequest {
                query: "Every violet narwhal must dance seventeen polkas".into(),
                collection, limit: 5,
            }
        )).await.unwrap();
        let metadata = response.structured_content.unwrap();
        assert_eq!(metadata["retrieval"]["support_status"], "not_assessed");
        assert_eq!(metadata["retrieval"]["candidate_status"], expected);
        assert_eq!(metadata["retrieval"]["scores_are_probabilities"], false);
    }
}
