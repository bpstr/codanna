//! No provider: both semantic handlers must offer lexical discovery when disabled.

use codanna::indexing::IndexFacade;
use codanna::mcp::{CodeIntelligenceServer, SemanticSearchRequest, SemanticSearchWithContextRequest};
use codanna::Settings;
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;

#[tokio::test]
async fn disabled_semantic_handlers_offer_lexical_search_without_rebuild() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings { index_path: temp.path().join("index"), ..Default::default() };
    settings.semantic_search.enabled = false;
    let facade = IndexFacade::new(Arc::new(settings)).unwrap();
    let server = CodeIntelligenceServer::new(facade);
    let docs = server.semantic_search_docs(Parameters(SemanticSearchRequest {
        query: "calendar settings".into(), limit: 5, threshold: None, lang: None,
    })).await.unwrap();
    let context = server.semantic_search_with_context(Parameters(SemanticSearchWithContextRequest {
        query: "calendar settings".into(), limit: 5, threshold: None, lang: None,
    })).await.unwrap();
    for response in [docs, context] {
        assert_eq!(response.is_error, Some(true));
        let value = serde_json::to_value(response).unwrap();
        let text = value["content"].as_array().unwrap().iter()
            .filter_map(|block| block["text"].as_str()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("No code or semantic index rebuild was attempted"), "{text}");
        assert!(text.contains("search_symbols") && text.contains("search_context"), "{text}");
        assert!(!text.contains("The index needs to be rebuilt"), "{text}");
    }
    assert!(!temp.path().join("index/semantic").exists());
}
