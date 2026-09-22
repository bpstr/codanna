//! Offline rebuild inventory. Never constructs an index, embedding backend, or model.
//!
//! Cache matches describe a fixed lookup snapshot, not the execution order of a
//! future rebuild. The latter may evict entries and send duplicate batch inputs.

use crate::embedding_cache::EmbeddingCache;
use crate::embedding_input::{InputBudget, backend_identity};
use crate::indexing::FileWalker;
use crate::indexing::facade::resolve_remote_model_name;
use crate::indexing::file_info::calculate_hash;
use crate::indexing::pipeline::stages::parse::{ParseStage, init_parser_cache};
use crate::{IndexError, IndexResult, Settings};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_ENTRIES: usize = 200_000;
const MAX_FILES: usize = 25_000;
const MAX_SOURCE_BYTES: usize = 128 * 1024 * 1024;
const MAX_UNIQUE_INPUTS: usize = 100_000;

/// Counts apply only to parsed files. Unknown fields are null, not successful zeros.
#[derive(Debug, Serialize)]
pub struct RebuildPlan {
    pub schema_version: u32,
    pub status: &'static str,
    pub scope: &'static str,
    pub roots: Vec<PathBuf>,
    pub source_fingerprint: String,
    pub files_discovered: usize,
    pub files_parsed: usize,
    pub files_requiring_generic_parser: usize,
    pub source_bytes: usize,
    pub symbols: usize,
    pub symbols_without_embedding_input: usize,
    pub embedding_candidates: usize,
    pub unique_embedding_inputs: usize,
    pub duplicate_embedding_inputs: usize,
    pub embedding_input_bytes: usize,
    pub unique_embedding_input_bytes: usize,
    pub backend: &'static str,
    pub configured_identity_sha256: Option<String>,
    pub cache_lookup: &'static str,
    pub cache_present: Option<bool>,
    pub snapshot_hit_inputs: Option<usize>,
    pub snapshot_miss_inputs: Option<usize>,
    pub snapshot_unique_miss_inputs: Option<usize>,
    pub snapshot_miss_input_bytes: Option<usize>,
    pub over_budget_inputs: Option<usize>,
    pub exact_provider_tokens: Option<usize>,
    pub provider_requests_made: usize,
    pub future_probe_may_cost_tokens: bool,
    pub warnings: Vec<&'static str>,
}

struct Lookup {
    backend: &'static str,
    identity: Option<String>,
    cache: Option<EmbeddingCache>,
    present: Option<bool>,
    budget: Option<InputBudget>,
    reason: &'static str,
}

fn failure(message: impl Into<String>) -> IndexError {
    IndexError::General(message.into())
}

fn lookup(settings: &Settings) -> IndexResult<Lookup> {
    let cfg = &settings.semantic_search;
    let remote = std::env::var("CODANNA_EMBED_URL")
        .ok()
        .or_else(|| cfg.remote_url.clone());
    let mut lookup = Lookup {
        backend: if !cfg.enabled {
            "disabled"
        } else if remote.is_some() {
            "remote"
        } else {
            "local"
        },
        identity: None,
        cache: None,
        present: None,
        budget: None,
        reason: "not_checked",
    };
    if !cfg.enabled {
        lookup.reason = "semantic_search_disabled";
        return Ok(lookup);
    }
    let Some(url) = remote else {
        // The actual local tokenizer is part of the identity. Never initialize
        // or download a model merely to make an offline estimate look precise.
        lookup.reason = "local_model_identity_not_loaded";
        return Ok(lookup);
    };
    let budget = InputBudget::remote(cfg.max_input_tokens, cfg.tokenizer_path.as_deref())
        .map_err(failure)?;
    let identity = backend_identity(
        "remote",
        &resolve_remote_model_name(cfg),
        Some(&calculate_hash(url.trim_end_matches('/'))),
        cfg.model_revision.as_deref(),
        &budget,
    );
    lookup.identity = Some(calculate_hash(&identity));
    lookup.budget = Some(budget);
    let dimension = match std::env::var("CODANNA_EMBED_DIM") {
        Ok(value) => Some(
            value
                .parse::<usize>()
                .map_err(|_| failure("CODANNA_EMBED_DIM must be a positive integer"))?,
        ),
        Err(_) => cfg.remote_dim,
    };
    let Some(dimension) = dimension else {
        lookup.reason = "remote_dimension_unknown_without_probe";
        return Ok(lookup);
    };
    if dimension == 0 {
        return Err(failure(
            "Remote embedding dimension must be greater than zero",
        ));
    }
    let path = settings.index_path.join("semantic/embedding-cache.json");
    lookup.present = Some(match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => true,
        Ok(_) => {
            return Err(failure(
                "Embedding cache must be a regular file for offline planning",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(failure(format!("Cannot inspect embedding cache: {error}"))),
    });
    // Use the runtime's bounded decoder and identity/dimension checks. Invalid,
    // incompatible and evicted entries are misses, never invented cache hits.
    lookup.cache = Some(EmbeddingCache::load(&path, &identity, dimension));
    lookup.reason = "configured_remote_identity_snapshot";
    Ok(lookup)
}

/// Inspect current source inputs using the regular bounded source snapshot and
/// native parsers. `settings.index_path` must already be resolved for the config.
///
/// Sources are confined to the explicit workspace; overlapping roots are deduped.
/// Discovery/read/parser failures are errors. Generic grammars are not loaded:
/// their files produce a partial report so planning cannot download code or models.
/// No source text, endpoint URL, API key, or individual input hashes are returned.
pub fn inspect(settings: Settings, requested_roots: &[PathBuf]) -> IndexResult<RebuildPlan> {
    let workspace = settings.workspace_root.as_ref()
        .ok_or_else(|| failure("Offline planning requires an explicit workspace_root or a .codanna/settings.toml config"))?
        .canonicalize().map_err(|error| failure(format!("Cannot resolve workspace: {error}")))?;
    let selected = if requested_roots.is_empty() {
        &settings.indexed_paths_cache
    } else {
        requested_roots
    };
    if selected.is_empty() {
        return Err(failure(
            "No source roots selected; provide paths or configure indexing.indexed_paths",
        ));
    }
    let mut roots = Vec::new();
    for root in selected {
        let root = root
            .canonicalize()
            .map_err(|error| failure(format!("Cannot resolve source root: {error}")))?;
        if !root.starts_with(&workspace) {
            return Err(failure(
                "Offline source root is outside the configured workspace",
            ));
        }
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots.sort();
    let lookup = lookup(&settings)?;
    let settings = Arc::new(settings);
    let files = FileWalker::new(Arc::clone(&settings)).snapshot(
        &roots,
        MAX_ENTRIES,
        MAX_FILES,
        MAX_SOURCE_BYTES,
    )?;
    let inventory: Vec<_> = files
        .iter()
        .map(|file| {
            (
                file.path
                    .strip_prefix(&workspace)
                    .unwrap_or(&file.path)
                    .to_string_lossy()
                    .replace('\\', "/"),
                &file.hash,
            )
        })
        .collect();
    let fingerprint = calculate_hash(
        &serde_json::to_string(&inventory).map_err(|error| failure(error.to_string()))?,
    );
    let mut report = RebuildPlan {
        schema_version: 1,
        status: "complete",
        scope: "current_source_doc_comment_inputs",
        roots,
        source_fingerprint: fingerprint,
        files_discovered: files.len(),
        source_bytes: files.iter().map(|file| file.content.len()).sum(),
        files_parsed: 0,
        files_requiring_generic_parser: 0,
        symbols: 0,
        symbols_without_embedding_input: 0,
        embedding_candidates: 0,
        unique_embedding_inputs: 0,
        duplicate_embedding_inputs: 0,
        embedding_input_bytes: 0,
        unique_embedding_input_bytes: 0,
        backend: lookup.backend,
        configured_identity_sha256: lookup.identity,
        cache_lookup: lookup.reason,
        cache_present: lookup.present,
        snapshot_hit_inputs: lookup.cache.as_ref().map(|_| 0),
        snapshot_miss_inputs: lookup.cache.as_ref().map(|_| 0),
        snapshot_unique_miss_inputs: lookup.cache.as_ref().map(|_| 0),
        snapshot_miss_input_bytes: lookup.cache.as_ref().map(|_| 0),
        over_budget_inputs: lookup.budget.as_ref().map(|_| 0),
        exact_provider_tokens: None,
        provider_requests_made: 0,
        future_probe_may_cost_tokens: lookup.backend == "remote",
        warnings: vec![
            "Cache hits are snapshot opportunities, not guaranteed future hits; eviction, batching, retries and source changes can increase cost.",
            "UTF-8 bytes and input-item counts are not billed token counts; no price estimate is made.",
            "The input policy currently embeds documentation comments, not all source code. This is not a relevance or graph-completeness score.",
            "No backend was contacted. Configured model identity and dimensions have not been confirmed against a live provider.",
        ],
    };
    init_parser_cache(Arc::clone(&settings));
    let parser = ParseStage::new(Arc::clone(&settings));
    let mut unique = HashSet::new();
    for file in files {
        if !has_native_parser(&file.path, &settings)? {
            report.files_requiring_generic_parser += 1;
            continue;
        }
        let parsed = parser
            .parse(file)
            .map_err(|error| failure(error.to_string()))?;
        report.files_parsed += 1;
        for symbol in parsed.raw_symbols {
            report.symbols += 1;
            let Some(input) = symbol.doc_comment else {
                report.symbols_without_embedding_input += 1;
                continue;
            };
            // Same Some(doc_comment) eligibility as COLLECT, including duplicates.
            report.embedding_candidates += 1;
            report.embedding_input_bytes += input.len();
            let first = unique.insert(calculate_hash(&input));
            if unique.len() > MAX_UNIQUE_INPUTS {
                return Err(failure(
                    "Offline plan exceeded 100000 unique embedding inputs",
                ));
            }
            if first {
                report.unique_embedding_input_bytes += input.len();
            }
            if let Some(budget) = &lookup.budget {
                if budget.validate([input.as_ref()]).is_err() {
                    *report.over_budget_inputs.as_mut().unwrap() += 1;
                }
            }
            if let Some(cache) = &lookup.cache {
                if cache.get(&input).is_some() {
                    *report.snapshot_hit_inputs.as_mut().unwrap() += 1;
                } else {
                    *report.snapshot_miss_inputs.as_mut().unwrap() += 1;
                    *report.snapshot_miss_input_bytes.as_mut().unwrap() += input.len();
                    if first {
                        *report.snapshot_unique_miss_inputs.as_mut().unwrap() += 1;
                    }
                }
            }
        }
    }
    report.unique_embedding_inputs = unique.len();
    report.duplicate_embedding_inputs = report.embedding_candidates - unique.len();
    if report.files_requiring_generic_parser > 0 {
        report.status = "partial";
        report.warnings.push("Generic grammar files were not parsed, even if cached, to prohibit implicit downloads; their embedding inputs are unknown, not zero.");
    }
    if report.over_budget_inputs.is_some_and(|count| count > 0) {
        report.status = if report.status == "partial" {
            "partial_blocked"
        } else {
            "blocked"
        };
        report.warnings.push("At least one parsed input exceeds the configured embedding budget; a successful full rebuild is not predicted.");
    }
    Ok(report)
}

fn has_native_parser(path: &Path, settings: &Settings) -> IndexResult<bool> {
    let registry = crate::parsing::get_registry()
        .lock()
        .map_err(|_| IndexError::MutexPoisoned)?;
    Ok(path
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(|extension| registry.get_by_extension(extension))
        .is_some_and(|definition| definition.is_enabled(settings)))
}
