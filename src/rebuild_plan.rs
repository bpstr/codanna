//! Offline rebuild inventory. Never constructs an index, embedding backend, or model.
//!
//! Prepared inputs use the configured source policy and the runtime segmenter.
//! Cache matches are snapshot opportunities, not a prediction of rebuild ordering.

use crate::embedding_cache::EmbeddingCache;
use crate::embedding_input::{InputBudget, backend_identity};
use crate::indexing::FileWalker;
use crate::indexing::facade::resolve_remote_model_name;
use crate::indexing::file_info::calculate_hash;
use crate::indexing::pipeline::stages::parse::{ParseStage, init_parser_cache};
use crate::symbol_representation::CodeEmbeddingPolicy;
use crate::{IndexError, IndexResult, Settings};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_ENTRIES: usize = 200_000;
const MAX_FILES: usize = 25_000;
const MAX_SOURCE_BYTES: usize = 128 * 1024 * 1024;
const MAX_UNIQUE_INPUTS: usize = 100_000;

/// Counts cover parsed files only. Unknown segment-dependent fields are null.
#[derive(Debug, Serialize)]
pub struct RebuildPlan {
    pub schema_version: u32,
    pub status: &'static str,
    pub scope: &'static str,
    pub code_representation: CodeEmbeddingPolicy,
    pub source_input_policy: &'static str,
    pub input_inventory_complete: bool,
    pub roots: Vec<PathBuf>,
    pub source_fingerprint: String,
    pub files_discovered: usize,
    pub files_parsed: usize,
    pub files_requiring_generic_parser: usize,
    pub source_bytes: usize,
    pub symbols: usize,
    pub eligible_symbols: usize,
    pub symbols_without_embedding_input: usize,
    /// Parent symbols submitted to the embedding stage, not segment inputs.
    pub embedding_candidates: usize,
    pub body_sources: usize,
    pub body_sources_header_only: usize,
    pub missing_body_sources: usize,
    /// Captured headers plus source fragments, before segment header repetition.
    pub retained_representation_bytes: usize,
    /// Prepared inference-input occurrences, including repeated inputs/headers.
    pub embedding_inputs: Option<usize>,
    pub unique_embedding_inputs: Option<usize>,
    pub duplicate_embedding_inputs: Option<usize>,
    pub embedding_input_bytes: Option<usize>,
    pub unique_embedding_input_bytes: Option<usize>,
    pub backend: &'static str,
    pub configured_identity_sha256: Option<String>,
    pub cache_lookup: &'static str,
    pub cache_present: Option<bool>,
    pub snapshot_hit_inputs: Option<usize>,
    pub snapshot_miss_inputs: Option<usize>,
    pub snapshot_unique_miss_inputs: Option<usize>,
    pub snapshot_miss_input_bytes: Option<usize>,
    /// Parent inputs rejected during complete-input validation or segmentation.
    pub input_policy_rejections: Option<usize>,
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
        lookup.reason = "local_model_identity_not_loaded";
        return Ok(lookup);
    };
    if let Some(path) = &cfg.tokenizer_path {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| failure(format!("Cannot inspect configured tokenizer: {error}")))?;
        if !metadata.is_file() {
            return Err(failure("Offline tokenizer must be a regular file"));
        }
    }
    let budget = InputBudget::remote(cfg.max_input_tokens, cfg.tokenizer_path.as_deref())
        .map_err(failure)?;
    let identity = cfg.code_representation.bind_identity(backend_identity(
        "remote",
        &resolve_remote_model_name(cfg),
        Some(&calculate_hash(url.trim_end_matches('/'))),
        cfg.model_revision.as_deref(),
        &budget,
    ));
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
    lookup.cache = Some(EmbeddingCache::load(&path, &identity, dimension));
    lookup.reason = "configured_remote_identity_snapshot";
    Ok(lookup)
}

fn add_count(count: &mut Option<usize>, amount: usize) {
    if let Some(count) = count {
        *count += amount;
    }
}

fn record_input(
    report: &mut RebuildPlan,
    lookup: &Lookup,
    unique: &mut HashSet<String>,
    input: &str,
) -> IndexResult<()> {
    add_count(&mut report.embedding_inputs, 1);
    add_count(&mut report.embedding_input_bytes, input.len());
    let first = unique.insert(calculate_hash(input));
    if unique.len() > MAX_UNIQUE_INPUTS {
        return Err(failure(
            "Offline plan exceeded 100000 unique embedding inputs",
        ));
    }
    if first {
        add_count(&mut report.unique_embedding_input_bytes, input.len());
    }
    if let Some(cache) = &lookup.cache {
        if cache.get(input).is_some() {
            add_count(&mut report.snapshot_hit_inputs, 1);
        } else {
            add_count(&mut report.snapshot_miss_inputs, 1);
            add_count(&mut report.snapshot_miss_input_bytes, input.len());
            if first {
                add_count(&mut report.snapshot_unique_miss_inputs, 1);
            }
        }
    }
    Ok(())
}

/// Inventory source-policy inputs without opening an index or preparing a provider.
/// Generic grammars are never loaded; their inputs remain unmeasured. Body inputs
/// use the same capture and complete-input segmentation as runtime indexing.
/// `settings.index_path` must already be resolved relative to the chosen config.
/// No source text, endpoint URL, credential or per-input hash is returned.
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
    let policy = settings.semantic_search.code_representation;
    let inputs_known = policy == CodeEmbeddingPolicy::DocComment || lookup.budget.is_some();
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
        schema_version: 2,
        status: "complete",
        scope: match policy {
            CodeEmbeddingPolicy::DocComment => "current_source_doc_comment_inputs",
            CodeEmbeddingPolicy::SymbolBodyV1 => "current_source_symbol_body_inputs",
        },
        code_representation: policy,
        source_input_policy: policy.source_policy(),
        input_inventory_complete: inputs_known,
        roots,
        source_fingerprint: fingerprint,
        files_discovered: files.len(),
        source_bytes: files.iter().map(|file| file.content.len()).sum(),
        files_parsed: 0,
        files_requiring_generic_parser: 0,
        symbols: 0,
        eligible_symbols: 0,
        symbols_without_embedding_input: 0,
        embedding_candidates: 0,
        body_sources: 0,
        body_sources_header_only: 0,
        missing_body_sources: 0,
        retained_representation_bytes: 0,
        embedding_inputs: inputs_known.then_some(0),
        unique_embedding_inputs: inputs_known.then_some(0),
        duplicate_embedding_inputs: inputs_known.then_some(0),
        embedding_input_bytes: inputs_known.then_some(0),
        unique_embedding_input_bytes: inputs_known.then_some(0),
        backend: lookup.backend,
        configured_identity_sha256: lookup.identity.clone(),
        cache_lookup: lookup.reason,
        cache_present: lookup.present,
        snapshot_hit_inputs: lookup.cache.as_ref().map(|_| 0),
        snapshot_miss_inputs: lookup.cache.as_ref().map(|_| 0),
        snapshot_unique_miss_inputs: lookup.cache.as_ref().map(|_| 0),
        snapshot_miss_input_bytes: lookup.cache.as_ref().map(|_| 0),
        input_policy_rejections: lookup.budget.as_ref().map(|_| 0),
        exact_provider_tokens: None,
        provider_requests_made: 0,
        future_probe_may_cost_tokens: lookup.backend == "remote",
        warnings: vec![
            "Cache hits are snapshot opportunities, not guaranteed future hits; eviction, batching, retries and source changes can increase cost.",
            "UTF-8 bytes and input-item counts are not billed token counts; no price estimate is made.",
            "No backend was contacted. Configured model identity and dimensions have not been confirmed against a live provider.",
            "Input availability is not a relevance or graph-completeness score. This report does not authorize a rebuild.",
        ],
    };
    // A disabled backend still permits an inventory of potential source inputs.
    // Enable only ParseStage's capture switch in an isolated settings copy. No
    // IndexFacade/backend is constructed, and the caller's config is not changed.
    let parsing_settings = if policy == CodeEmbeddingPolicy::SymbolBodyV1
        && !settings.semantic_search.enabled
    {
        let mut parsing = (*settings).clone();
        parsing.semantic_search.enabled = true;
        report.warnings.push("Semantic indexing is disabled; captured parents are potential inputs only, and inference segment counts remain unknown.");
        Arc::new(parsing)
    } else {
        Arc::clone(&settings)
    };
    init_parser_cache(Arc::clone(&parsing_settings));
    let parser = ParseStage::new(parsing_settings);
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
            if !policy.eligible(symbol.kind, symbol.doc_comment.is_some()) {
                report.symbols_without_embedding_input += 1;
                continue;
            }
            report.eligible_symbols += 1;
            match policy {
                CodeEmbeddingPolicy::DocComment => {
                    if let Some(input) = symbol.doc_comment {
                        report.embedding_candidates += 1;
                        if let Some(budget) = &lookup.budget {
                            if budget.validate([input.as_ref()]).is_err() {
                                add_count(&mut report.input_policy_rejections, 1);
                            }
                        }
                        record_input(&mut report, &lookup, &mut unique, &input)?;
                    }
                }
                CodeEmbeddingPolicy::SymbolBodyV1 => {
                    let Some(source) = symbol.embedding_source else {
                        report.missing_body_sources += 1;
                        report.symbols_without_embedding_input += 1;
                        continue;
                    };
                    report.embedding_candidates += 1;
                    report.body_sources += 1;
                    report.body_sources_header_only += usize::from(source.fragments.is_empty());
                    report.retained_representation_bytes += source.retained_bytes();
                    let Some(budget) = &lookup.budget else {
                        continue;
                    };
                    match source.inputs(budget) {
                        Ok(inputs) => {
                            for input in inputs {
                                record_input(&mut report, &lookup, &mut unique, &input.text)?;
                            }
                        }
                        Err(_) => add_count(&mut report.input_policy_rejections, 1),
                    }
                }
            }
        }
    }
    if let Some(inputs) = report.embedding_inputs {
        report.unique_embedding_inputs = Some(unique.len());
        report.duplicate_embedding_inputs = Some(inputs - unique.len());
    }
    let partial = !inputs_known
        || report.files_requiring_generic_parser > 0
        || report.missing_body_sources > 0;
    let blocked = report
        .input_policy_rejections
        .is_some_and(|count| count > 0);
    report.input_inventory_complete = !partial && !blocked;
    report.status = match (partial, blocked) {
        (false, false) => "complete",
        (true, false) => "partial",
        (false, true) => "blocked",
        (true, true) => "partial_blocked",
    };
    if !inputs_known {
        report.warnings.push("The backend input budget/tokenizer is unavailable offline. Body segment counts, repeated-header bytes and segment cache matches are unknown, not zero.");
    }
    if report.files_requiring_generic_parser > 0 {
        report.warnings.push("Generic grammar files were not parsed, even if cached, to prohibit implicit downloads; their embedding inputs are unknown, not zero.");
    }
    if report.missing_body_sources > 0 {
        report.warnings.push("Some policy-eligible symbols lack captured representations, for example after the per-file budget is exhausted. They are not silently replaced by comment inputs.");
    }
    if report.body_sources_header_only > 0 {
        report.warnings.push("Some captured representations contain headers only; complete implementation coverage is not implied.");
    }
    if blocked {
        report.warnings.push("At least one parent input failed budget, tokenizer or segment preparation; counts cover prepared inputs only and do not predict a successful full rebuild.");
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

#[cfg(test)]
#[path = "rebuild_plan_tests.rs"]
mod tests;
