//! CLI statistics retain collection scope and expose shared embedding health.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore};
use codanna::vector::{EmbeddingGenerator, VectorDimension, VectorError};

struct FixedGenerator;

impl EmbeddingGenerator for FixedGenerator {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }

    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(2).unwrap()
    }

    fn cache_identity(&self) -> String {
        "diagnostics-fixture@1".into()
    }
}

#[test]
fn document_stats_reports_shared_vector_health_without_loading_a_model() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut settings = codanna::Settings {
        index_path: root.join(".codanna/index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.documents.enabled = true;
    std::fs::create_dir(root.join(".codanna")).unwrap();
    std::fs::write(
        root.join(".codanna/settings.toml"),
        toml::to_string(&settings).unwrap(),
    )
    .unwrap();
    let index = settings.index_path.join("documents");
    let dimension = VectorDimension::new(2).unwrap();
    let chunking = ChunkingConfig {
        min_chunk_chars: 1,
        max_chunk_chars: 256,
        overlap_chars: 0,
        ..Default::default()
    };
    let mut store = DocumentStore::new(&index, dimension)
        .unwrap()
        .with_embeddings(Box::new(FixedGenerator))
        .unwrap();
    for collection in ["alpha", "beta"] {
        let source = root.join(format!("{collection}.md"));
        std::fs::write(&source, format!("{collection} current source evidence")).unwrap();
        store
            .index_collection(
                collection,
                &CollectionConfig {
                    paths: vec![source],
                    ..Default::default()
                },
                &chunking,
            )
            .unwrap();
    }
    drop(store);

    let mut lexical_store = DocumentStore::new(&index, dimension).unwrap();
    let source = root.join("lexical.md");
    std::fs::write(&source, "Document waiting for embedding backfill").unwrap();
    lexical_store
        .index_collection(
            "lexical",
            &CollectionConfig {
                paths: vec![source],
                ..Default::default()
            },
            &chunking,
        )
        .unwrap();
    drop(lexical_store);

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_codanna"))
        .args(["documents", "stats", "alpha", "--json"])
        .current_dir(&root)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["name"], "alpha");
    assert_eq!(value["chunk_count"], 1);
    assert_eq!(value["file_count"], 1);
    let embeddings = &value["embedding_index"];
    assert_eq!(embeddings["live_vectors"], 2);
    assert_eq!(embeddings["physical_vectors"], 2);
    assert_eq!(embeddings["unembedded_chunks"], 1);
    assert!(embeddings["vector_segments"].as_u64().unwrap() > 0);
    assert!(
        embeddings["generation"]
            .as_str()
            .unwrap()
            .starts_with("gen-")
    );
    assert!(!embeddings["identity"].as_str().unwrap().is_empty());
}
