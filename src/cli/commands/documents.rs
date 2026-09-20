//! Documents management command.

use std::path::PathBuf;
use std::time::Instant;

use crate::cli::DocumentAction;
use crate::config::Settings;
use crate::documents::{
    CollectionConfig, CollectionStats, DocumentStore, EmbeddingDiagnostics, IndexProgress,
    SearchQuery,
};
use crate::io::EnvelopeEntityType;
use crate::io::envelope::{Envelope, ResultCode};
use crate::io::status_line::StatusLine;
use crate::io::{ProgressBar, ProgressBarOptions, ProgressBarStyle};
use crate::vector::VectorDimension;

/// Print JSON or error message if serialization fails.
fn print_json<T: serde::Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("JSON serialization error: {e}");
            std::process::exit(2);
        }
    }
}

#[derive(serde::Serialize)]
struct DocumentStatistics {
    #[serde(flatten)]
    collection: CollectionStats,
    /// Vectors belong to the shared document index, across all collections.
    embedding_index: EmbeddingDiagnostics,
}

/// Run documents management command.
pub fn run(action: DocumentAction, config: &Settings, cli_config: Option<&PathBuf>) {
    let doc_path = config.index_path.join("documents");
    let dimension = VectorDimension::dimension_384();

    // Helper to create store with optional embeddings
    let create_store_with_embeddings = || -> Result<DocumentStore, String> {
        crate::documents::open_from_settings(config)
            .map_err(|error| format!("Failed to open document store: {error}"))
    };

    match action {
        DocumentAction::Status { json } => match crate::documents::status::read_runs(&doc_path) {
            Ok(runs) => {
                if json {
                    print_json(&runs);
                } else if runs.is_empty() {
                    println!(
                        "No indexing run status recorded. Older indexes have unknown completion status."
                    );
                } else {
                    for run in runs {
                        println!("{}", run.summary());
                    }
                }
            }
            Err(error) => {
                eprintln!("Cannot read indexing status: {error}");
                std::process::exit(1);
            }
        },
        DocumentAction::Index {
            collection,
            all,
            force,
            no_progress,
        } => {
            // Progress enabled by default from settings, --no-progress overrides
            let show_progress = config.indexing.show_progress && !no_progress;
            run_index(
                config,
                collection,
                all,
                force,
                show_progress,
                create_store_with_embeddings,
            );
        }

        DocumentAction::Search {
            args,
            collection,
            limit,
            json,
            fields,
        } => {
            use crate::io::args::parse_positional_args;

            // Parse positional arguments for query and key:value pairs
            let (positional_query, params) = parse_positional_args(&args);

            // Determine query source (priority: positional > key:value)
            let final_query = positional_query
                .or_else(|| params.get("query").cloned())
                .unwrap_or_else(|| {
                    eprintln!("Error: search requires a query");
                    eprintln!("Usage: codanna documents search \"query\" [options]");
                    eprintln!("   or: codanna documents search query:\"search text\" [options]");
                    std::process::exit(1);
                });

            // Merge parameters (flags take precedence over key:value)
            let final_limit = limit.unwrap_or_else(|| {
                params
                    .get("limit")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(10)
            });

            // Collection can come from --collection flag or collection:name
            let final_collection = collection.or_else(|| params.get("collection").cloned());

            let mut store = match create_store_with_embeddings() {
                Ok(s) => s,
                Err(e) => {
                    if json {
                        let envelope: Envelope<()> = Envelope::error(ResultCode::IndexError, &e)
                            .with_hint("Run 'codanna documents index' to create the index");
                        print_json(&envelope);
                        std::process::exit(2);
                    }
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };

            let query_text = final_query.clone();
            let search_query = SearchQuery {
                text: final_query,
                collection: final_collection,
                document: None,
                limit: final_limit,
                preview_config: Some(config.documents.search.clone()),
            };

            let start = Instant::now();
            match store.search(search_query) {
                Ok(results) => {
                    let duration_ms = start.elapsed().as_millis() as u64;
                    let count = results.len();

                    if json {
                        let envelope = if results.is_empty() {
                            Envelope::<Vec<_>>::not_found("No documents matched the query")
                                .with_query(&query_text)
                                .with_duration_ms(duration_ms)
                                .with_entity_type(EnvelopeEntityType::Document)
                                .with_hint("Try a different query or check indexed collections with 'codanna documents list'")
                        } else {
                            Envelope::success(results)
                                .with_message(format!("Found {count} matching documents"))
                                .with_entity_type(EnvelopeEntityType::Document)
                                .with_count(count)
                                .with_query(&query_text)
                                .with_duration_ms(duration_ms)
                                .with_hint(
                                    "Use the file paths and byte ranges to read specific sections",
                                )
                        };
                        let output = super::mcp::render_envelope_json(&envelope, fields.as_ref());
                        println!("{output}");
                    } else if results.is_empty() {
                        eprintln!("No results found.");
                    } else {
                        for (i, result) in results.iter().enumerate() {
                            println!(
                                "\n{}. {} (score: {:.3})",
                                i + 1,
                                crate::parsing::paths::render_absolute_path(&result.source_path)
                                    .display(),
                                result.similarity
                            );
                            if !result.heading_context.is_empty() {
                                println!("   Context: {}", result.heading_context.join(" > "));
                            }
                            println!("   Preview: {}", result.content_preview);
                        }
                    }
                }
                Err(e) => {
                    if json {
                        let envelope: Envelope<()> = Envelope::error(
                            ResultCode::InternalError,
                            format!("Search failed: {e}"),
                        );
                        print_json(&envelope);
                        std::process::exit(2);
                    }
                    eprintln!("Search failed: {e}");
                    std::process::exit(1);
                }
            }
        }

        DocumentAction::List { json } => {
            let store = match DocumentStore::new(&doc_path, dimension) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };

            let collections = store.list_collections();
            if json {
                print_json(&collections);
            } else if collections.is_empty() {
                eprintln!("No collections indexed.");
                eprintln!("\nConfigured collections in settings.toml:");
                for name in config.documents.collections.keys() {
                    eprintln!("  - {name}");
                }
            } else {
                println!("Indexed collections:");
                for name in collections {
                    println!("  - {name}");
                }
            }
        }

        DocumentAction::Stats { collection, json } => {
            let store = match DocumentStore::new(&doc_path, dimension) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };

            match store.collection_stats(&collection) {
                Ok(stats) => {
                    let embedding_index = store.embedding_diagnostics();
                    if json {
                        print_json(&DocumentStatistics {
                            collection: stats,
                            embedding_index,
                        });
                    } else {
                        println!("Collection: {}", stats.name);
                        println!("  Chunks: {}", stats.chunk_count);
                        println!("  Files: {}", stats.file_count);
                        println!("Embedding index (all collections):");
                        println!(
                            "  Vectors: {} live / {} stored in {} segments",
                            embedding_index.live_vectors,
                            embedding_index.physical_vectors,
                            embedding_index.vector_segments,
                        );
                        println!(
                            "  Chunks without embeddings: {}",
                            embedding_index.unembedded_chunks,
                        );
                        println!(
                            "  Last publication: {} new vector bytes, {} compacted vector bytes",
                            embedding_index.new_vector_bytes,
                            embedding_index.compacted_vector_bytes,
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Failed to get stats for '{collection}': {e}");
                    std::process::exit(1);
                }
            }
        }

        DocumentAction::AddCollection {
            name,
            path,
            pattern,
        } => {
            run_add_collection(config, cli_config, name, path, pattern);
        }

        DocumentAction::RemoveCollection { name } => {
            run_remove_collection(config, cli_config, name);
        }
    }
}

fn run_index<F>(
    config: &Settings,
    collection: Option<String>,
    all: bool,
    force: bool,
    progress: bool,
    create_store_with_embeddings: F,
) where
    F: Fn() -> Result<DocumentStore, String>,
{
    use std::cell::RefCell;
    use std::sync::Arc;

    let monitor = match crate::documents::status::RunMonitor::start(
        &config.index_path.join("documents"),
        config.semantic_search.enabled,
        force,
    ) {
        Ok(monitor) => monitor,
        Err(error) => {
            eprintln!("Cannot create indexing status: {error}");
            std::process::exit(1);
        }
    };
    // Create or open document store with embeddings
    let mut store = match create_store_with_embeddings() {
        Ok(s) => s,
        Err(e) => {
            monitor.fail(&e);
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // Force flag: clear file states to treat all files as new
    if force {
        eprintln!("Force re-indexing selected collections; preserving source ownership");
        store.clear_file_states();
    }

    // Determine which collections to index
    let collections_to_index: Vec<(String, CollectionConfig)> = if let Some(name) = collection {
        match config.documents.collections.get(&name) {
            Some(col_config) => vec![(name, col_config.clone())],
            None => {
                monitor.fail(format!("Collection '{name}' not found in settings.toml"));
                eprintln!("Collection '{name}' not found in settings.toml");
                eprintln!("\nConfigured collections:");
                for name in config.documents.collections.keys() {
                    eprintln!("  - {name}");
                }
                std::process::exit(1);
            }
        }
    } else if all || config.documents.enabled {
        config
            .documents
            .collections
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    } else {
        monitor.fail("Document indexing is disabled");
        eprintln!("Document indexing is disabled. Enable with:");
        eprintln!("  [documents]");
        eprintln!("  enabled = true");
        eprintln!("\nOr specify a collection: --collection <name>");
        std::process::exit(1);
    };

    // Sync: remove collections that are indexed but not in config
    let indexed_collections: std::collections::HashSet<String> =
        store.list_collections().into_iter().collect();
    let configured_collections: std::collections::HashSet<String> =
        config.documents.collections.keys().cloned().collect();

    let stale_collections: Vec<String> = indexed_collections
        .difference(&configured_collections)
        .cloned()
        .collect();

    if !stale_collections.is_empty() {
        eprintln!(
            "Sync: removing {} stale collections",
            stale_collections.len()
        );
        for name in &stale_collections {
            eprintln!("  - {name}");
            if let Err(e) = store.delete_collection(name) {
                monitor.fail(format!("Removing stale collection '{name}': {e}"));
                eprintln!("    Failed to remove: {e}");
                std::process::exit(1);
            }
        }
    }

    // Now check if there's anything to index
    if collections_to_index.is_empty() {
        monitor.finish("completed: no collections to index");
        if stale_collections.is_empty() {
            eprintln!("No collections configured in settings.toml");
            eprintln!("\nTo add a collection:");
            eprintln!("  codanna documents add-collection <name> <path>");
        } else {
            eprintln!("Cleaned stale collections. No collections to index.");
        }
        return;
    }

    // Progress state for two-phase display
    type ProgressState = Option<(Arc<ProgressBar>, StatusLine<Arc<ProgressBar>>)>;
    let phase1_bar: RefCell<ProgressState> = RefCell::new(None);
    let phase2_bar: RefCell<ProgressState> = RefCell::new(None);

    let mut total_files = 0usize;
    let mut total_chunks = 0usize;
    let mut failed = false;

    for (name, col_config) in collections_to_index {
        if let Some((bar, status)) = phase1_bar.borrow_mut().take() {
            drop(status);
            eprintln!("{bar}");
        }
        if let Some((bar, status)) = phase2_bar.borrow_mut().take() {
            drop(status);
            eprintln!("{bar}");
        }
        let chunking = col_config.effective_chunking(&config.documents.defaults);

        if !progress {
            eprintln!("Indexing collection: {name}");
        }

        // Index with two-phase progress callback
        let result = store.index_collection_with_progress(&name, &col_config, &chunking, |prog| {
            monitor.update(&name, &prog);
            if !progress {
                return;
            }
            match prog {
                IndexProgress::Phase { .. } => {}
                IndexProgress::ProcessingFile { current, total, .. } => {
                    let mut p1 = phase1_bar.borrow_mut();
                    if p1.is_none() && total > 0 {
                        let options = ProgressBarOptions::default()
                            .with_style(ProgressBarStyle::VerticalSolid)
                            .with_width(28);
                        let bar = Arc::new(ProgressBar::with_options(
                            total as u64,
                            "files",
                            "chunked",
                            "failed",
                            options,
                        ));
                        let status = StatusLine::new(Arc::clone(&bar));
                        *p1 = Some((bar, status));
                    }
                    if let Some((bar, _)) = p1.as_ref() {
                        bar.set_progress(current as u64);
                    }
                }
                IndexProgress::GeneratingEmbeddings { current, total } => {
                    // Finish Phase 1 if active
                    let mut p1 = phase1_bar.borrow_mut();
                    if let Some((bar, status)) = p1.take() {
                        drop(status);
                        eprintln!("{bar}");
                    }

                    let mut p2 = phase2_bar.borrow_mut();
                    if p2.is_none() && total > 0 {
                        let options = ProgressBarOptions::default()
                            .with_style(ProgressBarStyle::VerticalSolid)
                            .with_width(28);
                        let bar = Arc::new(ProgressBar::with_options(
                            total as u64,
                            "chunks",
                            "embedded",
                            "failed",
                            options,
                        ));
                        let status = StatusLine::new(Arc::clone(&bar));
                        *p2 = Some((bar, status));
                    }
                    if let Some((bar, _)) = p2.as_ref() {
                        bar.set_progress(current as u64);
                    }
                }
            }
        });

        match result {
            Ok(stats) => {
                total_files += stats.files_processed;
                total_chunks += stats.chunks_created;
                if !progress {
                    eprintln!("  Files processed: {}", stats.files_processed);
                    eprintln!("  Files skipped: {}", stats.files_skipped);
                    eprintln!("  Chunks created: {}", stats.chunks_created);
                    eprintln!("  Chunks removed: {}", stats.chunks_removed);
                }
            }
            Err(e) => {
                failed = true;
                monitor.fail(format!("Collection '{name}': {e}"));
                eprintln!("Failed to index collection '{name}': {e}");
                break;
            }
        };
    }

    // Print final progress bars and summary
    if let Some((bar, status)) = phase1_bar.borrow_mut().take() {
        drop(status);
        eprintln!("{bar}");
    }
    if let Some((bar, status)) = phase2_bar.borrow_mut().take() {
        drop(status);
        eprintln!("{bar}");
    }
    if progress {
        eprintln!("Total: {total_files} files, {total_chunks} chunks");
    }
    monitor.finish(if failed { "failed" } else { "completed" });
    if failed {
        std::process::exit(1);
    }
}

fn run_add_collection(
    _config: &Settings,
    cli_config: Option<&PathBuf>,
    name: String,
    path: PathBuf,
    pattern: Option<String>,
) {
    // Find config file
    let config_path = if let Some(custom_path) = cli_config {
        custom_path.clone()
    } else {
        Settings::find_workspace_config().unwrap_or_else(|| {
            eprintln!("Error: No configuration file found. Run 'codanna init' first.");
            std::process::exit(1);
        })
    };

    // Canonicalize the path
    let canonical_path = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: Invalid path '{}': {e}", path.display());
            std::process::exit(1);
        }
    };

    // Check if path exists
    if !canonical_path.exists() {
        eprintln!(
            "Error: Path does not exist: {}",
            crate::parsing::paths::render_absolute_path(&canonical_path).display()
        );
        std::process::exit(1);
    }

    // Load current settings
    let mut settings = match Settings::load() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error loading settings: {e}");
            std::process::exit(1);
        }
    };

    // Check if collection already exists
    if settings.documents.collections.contains_key(&name) {
        // Add path to existing collection
        let collection = settings.documents.collections.get_mut(&name).unwrap();
        if collection.paths.contains(&canonical_path) {
            eprintln!(
                "Path already in collection '{name}': {}",
                crate::parsing::paths::render_absolute_path(&canonical_path).display()
            );
            std::process::exit(1);
        }
        collection.paths.push(canonical_path.clone());
        if let Some(pat) = pattern {
            if !collection.patterns.contains(&pat) {
                collection.patterns.push(pat);
            }
        }
        println!(
            "Added path to existing collection '{name}': {}",
            crate::parsing::paths::render_absolute_path(&canonical_path).display()
        );
    } else {
        // Create new collection
        let patterns = pattern.map(|p| vec![p]).unwrap_or_default();
        let collection_config = CollectionConfig {
            paths: vec![canonical_path.clone()],
            patterns,
            strategy: None,
            min_chunk_chars: None,
            max_chunk_chars: None,
            overlap_chars: None,
        };
        settings
            .documents
            .collections
            .insert(name.clone(), collection_config);
        println!(
            "Created collection '{name}' with path: {}",
            crate::parsing::paths::render_absolute_path(&canonical_path).display()
        );
    }

    // Enable documents if not already
    if !settings.documents.enabled {
        settings.documents.enabled = true;
        println!("Enabled document indexing (documents.enabled = true)");
    }

    // Save settings
    if let Err(e) = settings.save(&config_path) {
        eprintln!("Error saving settings: {e}");
        std::process::exit(1);
    }

    println!("Configuration saved to: {}", config_path.display());
    println!("\nTo index this collection:");
    println!("  codanna documents index --collection {name}");
    println!("  codanna documents index  # indexes all collections");
}

fn run_remove_collection(_config: &Settings, cli_config: Option<&PathBuf>, name: String) {
    // Find config file
    let config_path = if let Some(custom_path) = cli_config {
        custom_path.clone()
    } else {
        Settings::find_workspace_config().unwrap_or_else(|| {
            eprintln!("Error: No configuration file found. Run 'codanna init' first.");
            std::process::exit(1);
        })
    };

    // Load current settings
    let mut settings = match Settings::load() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error loading settings: {e}");
            std::process::exit(1);
        }
    };

    // Check if collection exists
    if !settings.documents.collections.contains_key(&name) {
        eprintln!("Collection '{name}' not found in settings.toml");
        if settings.documents.collections.is_empty() {
            eprintln!("\nNo collections configured.");
        } else {
            eprintln!("\nConfigured collections:");
            for coll_name in settings.documents.collections.keys() {
                eprintln!("  - {coll_name}");
            }
        }
        std::process::exit(1);
    }

    // Remove collection
    settings.documents.collections.remove(&name);
    println!("Removed collection '{name}' from settings.toml");

    // Save settings
    if let Err(e) = settings.save(&config_path) {
        eprintln!("Error saving settings: {e}");
        std::process::exit(1);
    }

    println!("Configuration saved to: {}", config_path.display());

    if settings.documents.collections.is_empty() {
        println!("\nNo collections remaining.");
    } else {
        println!("\nRemaining collections:");
        for coll_name in settings.documents.collections.keys() {
            println!("  - {coll_name}");
        }
    }

    println!("\nTo clean the index, run:");
    println!("  codanna documents index");
}
