//! Deterministic public-API document edit probe; no model or provider calls.
//! Compile this unchanged file against each compared codanna library.
use codanna::documents::{ChunkingConfig, CollectionConfig, DocumentStore};
use codanna::vector::{EmbeddingGenerator, VectorDimension, VectorError};
use std::error::Error;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Instant;

const DIMENSION: usize = 384;
const DOCUMENTS: usize = 8;
const CYCLES: usize = 100;

struct FixtureGenerator;

impl EmbeddingGenerator for FixtureGenerator {
    fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        Ok(texts
            .iter()
            .map(|text| {
                let bucket = text.bytes().fold(0usize, |state, byte| {
                    (state * 31 + byte as usize) % DIMENSION
                });
                let mut vector = vec![0.0; DIMENSION];
                vector[bucket] = 1.0;
                vector
            })
            .collect())
    }

    fn dimension(&self) -> VectorDimension {
        VectorDimension::new(DIMENSION).unwrap()
    }

    fn cache_identity(&self) -> String {
        "document-churn-fixture@1".into()
    }
}

#[derive(Default)]
struct VectorFiles {
    files: u64,
    bytes: u64,
    allocated_bytes: u64,
    records: u64,
}

fn inspect_vector_files(root: &Path, measured: &mut VectorFiles) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            inspect_vector_files(&path, measured)?;
        } else if path.extension().is_some_and(|extension| extension == "vec") {
            let mut header = [0u8; 16];
            File::open(&path)?.read_exact(&mut header)?;
            assert_eq!(&header[..4], b"CVEC");
            let value = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
            assert_eq!(value(4), 1, "Only the existing CVEC v1 format is measured");
            assert_eq!(value(8) as usize, DIMENSION);
            let records = value(12) as u64;
            let metadata = entry.metadata()?;
            assert_eq!(metadata.len(), 16 + records * (4 + 4 * DIMENSION as u64));
            measured.files += 1;
            measured.bytes += metadata.len();
            measured.allocated_bytes += metadata.blocks() * 512;
            measured.records += records;
        }
    }
    Ok(())
}

fn body(document: usize, cycle: usize) -> String {
    format!(
        "Synthetic guide {document:02}, revision {cycle:03}: retain the complete local policy record."
    )
}

fn sample(
    cycle: usize,
    store: &DocumentStore,
    index: &Path,
    start: Instant,
) -> Result<(), Box<dyn Error>> {
    let mut vectors = VectorFiles::default();
    inspect_vector_files(&index.join("vectors"), &mut vectors)?;
    let live = store.collection_stats("docs")?;
    assert_eq!(live.file_count, DOCUMENTS);
    assert_eq!(live.chunk_count, DOCUMENTS);
    assert!(vectors.records >= live.chunk_count as u64);
    println!(
        "{}{{\"cycle\":{cycle},\"live_files\":{},\"live_chunks\":{},\"vector_files\":{},\"vector_file_bytes\":{},\"allocated_vector_bytes\":{},\"physical_vector_records\":{},\"elapsed_ms\":{}}}",
        if cycle == 0 { "" } else { "," },
        live.file_count,
        live.chunk_count,
        vectors.files,
        vectors.bytes,
        vectors.allocated_bytes,
        vectors.records,
        start.elapsed().as_millis(),
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let out = std::env::args_os()
        .nth(1)
        .ok_or("Supply a new output directory")?;
    let out = Path::new(&out);
    fs::create_dir(out)?;
    let docs = out.join("docs");
    fs::create_dir(&docs)?;
    for document in 0..DOCUMENTS {
        fs::write(
            docs.join(format!("guide-{document:02}.md")),
            body(document, 0),
        )?;
    }
    let index = out.join("index");
    let config = ChunkingConfig {
        min_chunk_chars: 1,
        max_chunk_chars: 256,
        overlap_chars: 0,
        ..Default::default()
    };
    let mut store = DocumentStore::new(&index, FixtureGenerator.dimension())?
        .with_embeddings(Box::new(FixtureGenerator))?;
    let start = Instant::now();
    store.index_collection(
        "docs",
        &CollectionConfig {
            paths: vec![docs.clone()],
            ..Default::default()
        },
        &config,
    )?;
    println!(
        "{{\"schema_version\":1,\"fixture\":\"document-churn-v1\",\"documents\":{DOCUMENTS},\"dimensions\":{DIMENSION},\"edit_cycles\":{CYCLES},\"mock_embeddings\":true,\"samples\":["
    );
    sample(0, &store, &index, start)?;
    for cycle in 1..=CYCLES {
        let document = (cycle - 1) % DOCUMENTS;
        let path = docs.join(format!("guide-{document:02}.md"));
        fs::write(&path, body(document, cycle))?;
        assert_eq!(store.reindex_file(&path, &config)?, Some(1));
        if [1, 10, 25, 50, 75, 100].contains(&cycle) {
            sample(cycle, &store, &index, start)?;
        }
    }
    let edit_elapsed_ms = start.elapsed().as_millis();
    drop(store);
    let reopened = DocumentStore::new(&index, FixtureGenerator.dimension())?
        .with_embeddings(Box::new(FixtureGenerator))?;
    let reopened_live_chunks = reopened.collection_stats("docs")?.chunk_count;
    assert_eq!(reopened_live_chunks, DOCUMENTS);
    println!(
        "] ,\"edit_elapsed_ms\":{edit_elapsed_ms},\"reopened_live_chunks\":{reopened_live_chunks}}}"
    );
    Ok(())
}
