//! MCP direct tool invocation command.

use crate::Symbol;
use crate::config::Settings;
use crate::indexing::facade::IndexFacade;
use crate::io::args::parse_positional_args;
use crate::io::envelope::EntityType;
use crate::mcp::catalog::ToolKind;
use crate::mcp::service::{
    FindSymbolPageTarget, SymbolResolution, accepted_params_line, missing_param_message,
    resolve_symbol_or_id, tool_param_spec, try_resolve_find_symbol_page,
};
use serde::Serialize;

/// Print a terminal envelope and exit with the envelope's own exit_code.
/// The single choke point keeping the declared `exit_code` field and the
/// delivered process exit synchronized on every JSON error path.
fn emit_envelope_and_exit<T: Serialize>(envelope: crate::io::envelope::Envelope<T>) -> ! {
    println!("{}", envelope.to_json().expect("envelope serialization"));
    std::process::exit(envelope.exit_code.into());
}

/// Render an envelope as JSON, applying --fields projection when requested.
/// An unknown projection field emits an InvalidQuery error envelope and
/// exits 2, mirroring the unknown-argument rejection register.
pub(crate) fn render_envelope_json<T: Serialize>(
    envelope: &crate::io::envelope::Envelope<T>,
    fields: Option<&Vec<String>>,
) -> String {
    use crate::io::envelope::{Envelope, FieldProjectionError, ResultCode};
    match fields {
        None => envelope.to_json().expect("envelope serialization"),
        Some(f) => match envelope.to_json_with_fields(f) {
            Ok(json) => json,
            Err(FieldProjectionError::UnknownField { field, available }) => {
                let err: Envelope<()> = Envelope::error(
                    ResultCode::InvalidQuery,
                    format!("Unknown field '{field}' in --fields"),
                )
                .with_hint(format!(
                    "Available top-level fields: {}",
                    available.join(", ")
                ));
                emit_envelope_and_exit(err);
            }
            Err(FieldProjectionError::Serde(e)) => panic!("envelope serialization: {e}"),
        },
    }
}

pub(crate) fn render_document_envelope<T: Serialize>(
    envelope: &crate::io::envelope::Envelope<T>,
    retrieval: serde_json::Value,
    fields: Option<&Vec<String>>,
) -> String {
    let mut value: serde_json::Value =
        serde_json::from_str(&render_envelope_json(envelope, fields)).expect("envelope JSON");
    if let Some(meta) = value
        .get_mut("meta")
        .and_then(serde_json::Value::as_object_mut)
    {
        meta.insert("retrieval".into(), retrieval);
    }
    serde_json::to_string_pretty(&value).expect("document envelope serialization")
}

/// Print an INVALID_QUERY envelope for an ambiguous symbol name and exit 2.
/// Mirrors the MCP handlers' refuse-and-list policy: JSON mode must never
/// merge relationships across same-named symbols.
fn exit_ambiguous(entity: EntityType, name: &str, candidates: Vec<Symbol>) -> ! {
    use crate::io::envelope::{Envelope, ResultCode};
    let count = candidates.len();
    let mut envelope = Envelope::error(
        ResultCode::InvalidQuery,
        format!("Ambiguous: found {count} symbol(s) named '{name}'"),
    )
    .with_entity_type(entity)
    .with_query(name)
    .with_count(count)
    .with_hint("Ambiguous name: re-run with symbol_id:<id> using a candidate from data");
    envelope.data = Some(candidates);
    emit_envelope_and_exit(envelope);
}

/// Print an INDEX_ERROR envelope and exit 2. A backend failure must be
/// distinguishable from a legitimate zero-match result.
fn exit_index_error(entity: EntityType, query: &str, error: impl std::fmt::Display) -> ! {
    use crate::io::envelope::{Envelope, ResultCode};
    let envelope: Envelope<()> = Envelope::error(
        ResultCode::IndexError,
        format!("Index query failed: {error}"),
    )
    .with_entity_type(entity)
    .with_query(query);
    emit_envelope_and_exit(envelope);
}

/// Reject an invalid argument set: INVALID_QUERY envelope (JSON) or stderr
/// message (text), exit 2 in both modes. One emitter for missing required
/// params and unknown keys alike — the two failure directions of the same
/// parsing layer.
fn exit_invalid_args(tool: &str, message: &str, accepted: &[&str], json: bool) -> ! {
    let accepted_list = if accepted.is_empty() {
        format!("{tool} accepts no key:value parameters")
    } else {
        accepted_params_line(tool)
    };
    if json {
        use crate::io::envelope::{Envelope, ResultCode};
        let envelope: Envelope<()> =
            Envelope::error(ResultCode::InvalidQuery, message).with_hint(accepted_list);
        emit_envelope_and_exit(envelope);
    } else {
        eprintln!("Error: {message}");
        eprintln!("{accepted_list}");
        std::process::exit(2);
    }
}

// MCP tool JSON output structures
#[derive(Debug, Serialize)]
struct IndexInfo {
    symbol_count: usize,
    file_count: usize,
    relationship_count: usize,
    /// Commit of the binary that last wrote this index, `-dirty` when built
    /// from a modified tree. Absent for indexes written before the stamp
    /// existed and for binaries built without a work tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    builder_commit: Option<String>,
    symbol_kinds: std::collections::BTreeMap<String, usize>,
    /// Per-language symbol counts (schema `indexInfo.languages`)
    languages: std::collections::BTreeMap<String, usize>,
    semantic_search: SemanticSearchInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    documents: Option<DocumentsInfo>,
}

#[derive(Debug, Serialize)]
struct DocumentsInfo {
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    collections: Option<Vec<CollectionInfo>>,
}

#[derive(Debug, Serialize)]
struct CollectionInfo {
    name: String,
    chunk_count: usize,
    file_count: usize,
}

#[derive(Debug, Serialize)]
struct SemanticSearchInfo {
    enabled: bool,
    model_name: Option<String>,
    embeddings: Option<usize>,
    dimensions: Option<usize>,
    created: Option<String>,
    updated: Option<String>,
}

/// Flattened call/caller info combining symbol with call site metadata.
/// Avoids tuple waste like `[[symbol, null], ...]` in JSON output.
#[derive(Debug, Serialize)]
struct CallRelation {
    #[serde(flatten)]
    symbol: Symbol,
    /// Line number of the call site (1-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    call_line: Option<u32>,
    /// Column of the call site
    #[serde(skip_serializing_if = "Option::is_none")]
    call_column: Option<u32>,
}

/// Run the MCP direct tool invocation command.
pub async fn run(
    tool: String,
    positional: Vec<String>,
    args: Option<String>,
    json: bool,
    fields: Option<Vec<String>>,
    mut facade: IndexFacade,
    config: &Settings,
) {
    // Build arguments from both positional and --args
    let mut arguments = if let Some(args_str) = &args {
        // Parse JSON arguments if provided (backward compatibility)
        match serde_json::from_str::<serde_json::Value>(args_str) {
            Ok(serde_json::Value::Object(map)) => Some(map),
            Ok(_) => exit_invalid_args(&tool, "Arguments must be a JSON object", &[], json),
            Err(e) => exit_invalid_args(&tool, &format!("Failed to parse --args: {e}"), &[], json),
        }
    } else {
        // Start with empty map if no --args
        Some(serde_json::Map::new())
    };

    // Process positional arguments using unified parser
    if !positional.is_empty() {
        if let Some(ref mut args_map) = arguments {
            // Use the unified parser from args.rs
            let (first_positional, params) = parse_positional_args(&positional);

            // Handle the first positional argument based on tool type
            if let Some(pos_arg) = first_positional {
                if let Some(key) = ToolKind::parse(&tool).and_then(|kind| kind.positional()) {
                    args_map.insert(key.to_string(), serde_json::Value::String(pos_arg.clone()));
                } else {
                    eprintln!("Warning: tool '{tool}' has no positional argument");
                }
            }

            // Special handling: find_symbol supports symbol_id:XXX as positional
            // If symbol_id is in params but name wasn't set, use it as the name
            if tool == "find_symbol" && !args_map.contains_key("name") {
                if let Some(id) = params.get("symbol_id") {
                    args_map.insert(
                        "name".to_string(),
                        serde_json::Value::String(format!("symbol_id:{id}")),
                    );
                }
            }

            // Add all key:value pairs from params
            for (key, value) in params {
                if tool == "search_documents"
                    && key == "score_floor"
                    && value.parse::<f32>().is_ok_and(|floor| !floor.is_finite())
                {
                    exit_invalid_args(
                        &tool,
                        "score_floor must be finite",
                        tool_param_spec(&tool).0,
                        json,
                    );
                }
                // Try to parse as number first, then boolean, fallback to string
                let json_value = if let Ok(n) = value.parse::<i64>() {
                    serde_json::Value::Number(n.into())
                } else if let Ok(f) = value.parse::<f64>() {
                    serde_json::Value::Number(
                        serde_json::Number::from_f64(f)
                            .unwrap_or_else(|| serde_json::Number::from(0)),
                    )
                } else if let Ok(b) = value.parse::<bool>() {
                    serde_json::Value::Bool(b)
                } else {
                    serde_json::Value::String(value)
                };
                args_map.insert(key, json_value);
            }
        }
    }

    // Convert to Option<Map> only if we have arguments
    let mut arguments = arguments.filter(|map| !map.is_empty());
    // Preserve the legacy CLI render/exit paths while accepting ID-only JSON.
    if tool == "find_symbol"
        && let Some(map) = arguments.as_mut()
        && !map.contains_key("name")
        && let Some(id) = map.get("symbol_id")
    {
        let id = id
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| id.to_string());
        map.insert(
            "name".into(),
            serde_json::Value::String(format!("symbol_id:{id}")),
        );
    }

    // Validate the tool name up front: JSON mode never reaches the dispatch
    // match below, so its unknown-tool arm cannot cover this.
    let Some(tool_kind) = ToolKind::parse(&tool) else {
        let available = format!("Available tools: {}", ToolKind::names());
        if json {
            use crate::io::exit_code::ExitCode;
            use crate::io::format::JsonResponse;
            let response = JsonResponse::error(
                ExitCode::GeneralError,
                &format!("Unknown tool: {tool}"),
                vec![&available],
            );
            println!("{}", serde_json::to_string_pretty(&response).unwrap());
        } else {
            eprintln!("Unknown tool: {tool}");
            eprintln!("{available}");
        }
        std::process::exit(1);
    };

    // One validation surface for both output modes (replaces the per-tool
    // checks that used to sit duplicated in the JSON collection blocks and
    // the text dispatch): unknown keys reject instead of silently dropping,
    // and missing required params error as INVALID_QUERY, exit 2.
    {
        let (accepted, requires_one_of) = tool_param_spec(&tool);

        // `depth:` is a documented alias of `max_depth:` on analyze_impact —
        // the envelope's own meta field is named `depth`, so the surface must
        // accept the key it emits.
        if tool == "analyze_impact" {
            if let Some(map) = arguments.as_mut() {
                if let Some(depth) = map.remove("depth") {
                    if map.contains_key("max_depth") {
                        exit_invalid_args(
                            &tool,
                            "analyze_impact accepts either 'depth' or 'max_depth', not both",
                            accepted,
                            json,
                        );
                    }
                    map.insert("max_depth".to_string(), depth);
                }
            }
        }

        if let Some(map) = arguments.as_ref() {
            for key in map.keys() {
                if !accepted.contains(&key.as_str()) {
                    exit_invalid_args(
                        &tool,
                        &format!("Unknown parameter '{key}' for {tool}"),
                        accepted,
                        json,
                    );
                }
            }
        }

        if !requires_one_of.is_empty() {
            let satisfied = arguments
                .as_ref()
                .is_some_and(|map| requires_one_of.iter().any(|k| map.contains_key(*k)));
            if !satisfied {
                exit_invalid_args(&tool, &missing_param_message(&tool), accepted, json);
            }
        }
    }

    let budgeted_search = matches!(
        tool_kind,
        ToolKind::SearchSymbols
            | ToolKind::SemanticSearchDocs
            | ToolKind::SemanticSearchWithContext
            | ToolKind::SearchDocuments
            | ToolKind::SearchContext
            | ToolKind::SearchTicketContext
            | ToolKind::GetDocumentChunk
    );

    let document_request = if tool_kind == ToolKind::SearchDocuments {
        let request = serde_json::from_value::<crate::mcp::SearchDocumentsRequest>(
            serde_json::Value::Object(arguments.clone().unwrap_or_default()),
        )
        .unwrap_or_else(|error| {
            exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
        });
        let options = crate::documents::DocumentSearchOptions {
            literal: request.literal,
            score_floor: request.score_floor,
            authority_sources: request.authority_sources.clone(),
        };
        if let Err(error) = options.validate() {
            exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json);
        }
        Some(request)
    } else {
        None
    };

    let ticket_context_request = if tool_kind == ToolKind::SearchTicketContext {
        let request =
            serde_json::from_value::<crate::mcp::tools::ticket_context::TicketContextRequest>(
                serde_json::Value::Object(arguments.clone().unwrap_or_default()),
            )
            .unwrap_or_else(|error| {
                exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
            });
        if let Err(error) = crate::mcp::tools::ticket_context::validate(&request) {
            exit_invalid_args(&tool, error, tool_param_spec(&tool).0, json);
        }
        Some(request)
    } else {
        None
    };

    // Share typed defaults and validation with the actual MCP handler. The
    // CLI-only symbol_id alias has already supplied the name string above.
    let find_symbol_request = if tool_kind == ToolKind::FindSymbol {
        let mut map = arguments.clone().unwrap_or_default();
        // Preserve malformed-ID aliases' historical not-found rendering, but
        // validate positive numeric string aliases through the typed contract.
        if let Some(id) = map.get("symbol_id").and_then(serde_json::Value::as_str) {
            let target = format!("symbol_id:{id}");
            if map
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| !name.is_empty() && name != target)
            {
                exit_invalid_args(
                    &tool,
                    "Supply either name or symbol_id, not conflicting targets",
                    tool_param_spec(&tool).0,
                    json,
                );
            }
            if let Ok(id) = id.parse::<u32>() {
                map.insert("symbol_id".into(), serde_json::json!(id));
            } else {
                map.remove("symbol_id");
            }
        }
        Some(
            serde_json::from_value::<crate::mcp::FindSymbolRequest>(serde_json::Value::Object(map))
                .and_then(|request| {
                    let target = request
                        .target_name()
                        .map_err(<serde_json::Error as serde::de::Error>::custom)?
                        .into_owned();
                    // Every rendering must use the target accepted by the
                    // typed MCP contract, including an explicitly empty name.
                    arguments
                        .get_or_insert_with(Default::default)
                        .insert("name".to_owned(), serde_json::Value::String(target));
                    Ok(request)
                })
                .unwrap_or_else(|error| {
                    exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                }),
        )
    } else {
        None
    };

    // Use the same typed request validation in JSON collection and text
    // dispatch. A malformed optional scope must not turn into an omitted filter.
    let typed_validation = match tool_kind {
        ToolKind::SemanticSearchDocs => {
            serde_json::from_value::<crate::mcp::SemanticSearchRequest>(serde_json::Value::Object(
                arguments.clone().unwrap_or_default(),
            ))
            .map(|_| ())
        }
        ToolKind::SemanticSearchWithContext => {
            serde_json::from_value::<crate::mcp::SemanticSearchWithContextRequest>(
                serde_json::Value::Object(arguments.clone().unwrap_or_default()),
            )
            .map(|_| ())
        }
        ToolKind::GetDocumentChunk => {
            serde_json::from_value::<crate::mcp::GetDocumentChunkRequest>(
                serde_json::Value::Object(arguments.clone().unwrap_or_default()),
            )
            .map(|_| ())
        }
        ToolKind::SearchSymbols => serde_json::from_value::<crate::mcp::SearchSymbolsRequest>(
            serde_json::Value::Object(arguments.clone().unwrap_or_default()),
        )
        .map(|_| ()),
        ToolKind::SearchContext => serde_json::from_value::<crate::mcp::SearchContextRequest>(
            serde_json::Value::Object(arguments.clone().unwrap_or_default()),
        )
        .map(|_| ()),
        _ => Ok(()),
    };
    if let Err(error) = typed_validation {
        exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json);
    }

    // Semantic snapshots intentionally load without constructing a second
    // query model. Direct CLI invocations do not pass through the workspace
    // reader's lazy initializer, so initialize the shared backend once before
    // either JSON collection or text dispatch tries to embed the query.
    if matches!(
        tool.as_str(),
        "semantic_search_docs" | "semantic_search_with_context"
    ) {
        if let Err(error) = facade.prepare_semantic_query() {
            let query = arguments
                .as_ref()
                .and_then(|map| map.get("query"))
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            exit_index_error(EntityType::SearchResult, query, error);
        }
    }

    // Collect data for find_symbol if JSON output is requested
    let find_symbol_data = if json && tool == "find_symbol" {
        let name = arguments
            .as_ref()
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str());
        let language = arguments
            .as_ref()
            .and_then(|m| m.get("lang"))
            .and_then(|v| v.as_str());

        if let Some(symbol_name) = name {
            // One resolution policy with the MCP handler: the symbol_id:
            // form resolves by id here exactly as it does in text mode.
            let request = find_symbol_request
                .as_ref()
                .expect("find_symbol request validated");
            let (symbols, page) = match try_resolve_find_symbol_page(
                &facade,
                symbol_name,
                language,
                request.offset,
                request.limit,
            )
            .unwrap_or_else(|error| exit_index_error(EntityType::Symbol, symbol_name, error))
            {
                FindSymbolPageTarget::Symbols { symbols, page, .. } => (symbols, page),
                // Non-numeric id: nothing to look up; renders the
                // not_found envelope exactly like an unmatched name.
                FindSymbolPageTarget::InvalidId(_) => {
                    crate::mcp::service::page_symbols(Vec::new(), request.offset, request.limit)
                }
            };
            let semantic_definitions = facade.semantic_definition_status(&symbols);
            if !symbols.is_empty() {
                let mut results = Vec::new();

                for symbol in symbols {
                    // Get full context with callers using the same approach as MCP
                    let context = crate::mcp::service::selected_symbol_context(&facade, &symbol);

                    // Build result with context if available
                    if let Some(ctx) = context {
                        results.push(ctx);
                    } else {
                        // Fallback: create minimal context
                        let file_path = symbol.file_path.to_string();

                        results.push(crate::symbol::context::SymbolContext {
                            symbol,
                            file_path,
                            relationships: Default::default(),
                        });
                    }
                }
                Some((results, page, semantic_definitions))
            } else {
                Some((Vec::new(), page, semantic_definitions))
            }
        } else {
            None
        }
    } else {
        None
    };

    // Collect data for get_calls if JSON output is requested.
    // Resolution goes through the shared service layer: ambiguous names
    // refuse-and-list (exit 2) exactly like the MCP handler, never aggregate.
    let get_calls_data = if json && tool == "get_calls" {
        let symbol_id = arguments
            .as_ref()
            .and_then(|m| m.get("symbol_id"))
            .and_then(|v| v.as_u64())
            .map(|id| id as u32);
        let function_name = arguments
            .as_ref()
            .and_then(|m| m.get("function_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        use crate::symbol::context::ContextIncludes;
        match resolve_symbol_or_id(&facade, symbol_id, function_name) {
            SymbolResolution::Resolved { symbol, .. } => {
                let mut all_calls = Vec::new();
                if let Some(ctx) = facade.get_symbol_context(symbol.id, ContextIncludes::CALLS) {
                    if let Some(calls) = ctx.relationships.calls {
                        for (called, metadata) in calls {
                            all_calls.push(CallRelation {
                                symbol: called,
                                call_line: metadata.as_ref().and_then(|m| m.line).map(|l| l + 1),
                                call_column: metadata.as_ref().and_then(|m| m.column),
                            });
                        }
                    }
                }
                Some(all_calls)
            }
            SymbolResolution::NotFoundById(_) | SymbolResolution::NotFoundByName(_) => None,
            SymbolResolution::Ambiguous { name, candidates } => {
                exit_ambiguous(EntityType::Calls, &name, candidates)
            }
            SymbolResolution::MissingParam => exit_invalid_args(
                &tool,
                &missing_param_message(&tool),
                tool_param_spec(&tool).0,
                json,
            ),
        }
    } else {
        None
    };

    // Collect data for find_callers if JSON output is requested.
    // Same shared resolution policy as get_calls: refuse-and-list on
    // ambiguity, never merge callers of unrelated same-named symbols.
    let find_callers_data = if json && tool == "find_callers" {
        let symbol_id = arguments
            .as_ref()
            .and_then(|m| m.get("symbol_id"))
            .and_then(|v| v.as_u64())
            .map(|id| id as u32);
        let function_name = arguments
            .as_ref()
            .and_then(|m| m.get("function_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        match resolve_symbol_or_id(&facade, symbol_id, function_name) {
            SymbolResolution::Resolved { symbol, .. } => {
                let callers = facade.get_calling_functions_with_metadata(symbol.id);
                let all_callers: Vec<_> = callers
                    .into_iter()
                    .map(|(caller, metadata)| CallRelation {
                        symbol: caller,
                        call_line: metadata.as_ref().and_then(|m| m.line).map(|l| l + 1),
                        call_column: metadata.as_ref().and_then(|m| m.column),
                    })
                    .collect();
                Some(all_callers)
            }
            SymbolResolution::NotFoundById(_) | SymbolResolution::NotFoundByName(_) => None,
            SymbolResolution::Ambiguous { name, candidates } => {
                exit_ambiguous(EntityType::Callers, &name, candidates)
            }
            SymbolResolution::MissingParam => exit_invalid_args(
                &tool,
                &missing_param_message(&tool),
                tool_param_spec(&tool).0,
                json,
            ),
        }
    } else {
        None
    };

    // Collect data for analyze_impact if JSON output is requested
    let analyze_impact_data = if json && tool == "analyze_impact" {
        let symbol_id = arguments
            .as_ref()
            .and_then(|m| m.get("symbol_id"))
            .and_then(|v| v.as_u64())
            .map(|id| id as u32);
        let symbol_name = arguments
            .as_ref()
            .and_then(|m| m.get("symbol_name"))
            .and_then(|v| v.as_str());

        match resolve_symbol_or_id(&facade, symbol_id, symbol_name.map(|s| s.to_string())) {
            SymbolResolution::Resolved { symbol, .. } => {
                let max_depth = arguments
                    .as_ref()
                    .and_then(|m| m.get("max_depth"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3) as usize;

                let impacted_ids = facade
                    .get_impact_radius_bounded(symbol.id, max_depth)
                    .unwrap_or_else(|error| {
                        exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                    });
                let impacted_symbols = facade.get_symbols(&impacted_ids).unwrap_or_else(|error| {
                    exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                });

                Some(impacted_symbols)
            }
            SymbolResolution::NotFoundById(_) | SymbolResolution::NotFoundByName(_) => None,
            SymbolResolution::Ambiguous { name, candidates } => {
                exit_ambiguous(EntityType::ImpactGraph, &name, candidates)
            }
            SymbolResolution::MissingParam => exit_invalid_args(
                &tool,
                &missing_param_message(&tool),
                tool_param_spec(&tool).0,
                json,
            ),
        }
    } else {
        None
    };

    let guidance_config = facade.settings().guidance.clone();

    // Only load document store for tools that need it.
    // This is expensive (~1s to load ML model) so we skip it for other tools
    let needs_document_store = matches!(
        tool.as_str(),
        "search_documents" | "search_context" | "search_ticket_context"
    );
    let document_store = if needs_document_store
        && !document_request
            .as_ref()
            .is_some_and(|request| request.literal)
    {
        crate::documents::load_from_settings(config)
    } else {
        None
    };

    // If we need JSON output for get_index_info, collect data before moving indexer
    let index_info_data = if json && tool == "get_index_info" {
        let symbol_count = facade.symbol_count();
        let file_count = facade.file_count();
        let relationship_count = facade.relationship_count();

        let (symbol_kinds, languages) = facade.symbol_stats();

        // Get semantic search info
        let semantic_search = if let Some(metadata) = facade.get_semantic_metadata() {
            SemanticSearchInfo {
                enabled: true,
                model_name: Some(metadata.model_name),
                embeddings: Some(metadata.embedding_count),
                dimensions: Some(metadata.dimension),
                created: Some(crate::mcp::format_relative_time(metadata.created_at)),
                updated: Some(crate::mcp::format_relative_time(metadata.updated_at)),
            }
        } else {
            SemanticSearchInfo {
                enabled: false,
                model_name: None,
                embeddings: None,
                dimensions: None,
                created: None,
                updated: None,
            }
        };

        // Document collections info is skipped for performance
        // Loading DocumentStore requires ML model (~1s) which defeats fast index info
        // TODO: Add fast stats-only document store loader
        let documents: Option<DocumentsInfo> = None;

        let builder_commit = crate::storage::IndexMetadata::load(&config.index_path)
            .ok()
            .and_then(|m| m.builder_commit);

        Some(IndexInfo {
            symbol_count,
            file_count: file_count as usize,
            relationship_count,
            builder_commit,
            symbol_kinds,
            languages,
            semantic_search,
            documents,
        })
    } else {
        None
    };

    // Target handlers carry their resolution outcome into the text exit status.
    let mut text_exit = 0;

    // Embedded mode - use already loaded facade directly
    let server = {
        let server = crate::mcp::CodeIntelligenceServer::new(facade);

        // Add DocumentStore if documents are enabled and indexed
        if let Some(store_arc) = document_store {
            server.with_document_store_arc(store_arc)
        } else {
            server
        }
    };

    // Call the tool directly
    use crate::mcp::*;
    use rmcp::handler::server::wrapper::Parameters;

    // Search/read tools share the canonical budgeted MCP result in both modes.
    // Legacy non-search JSON tools still use their pre-collected service data.
    let result = if json && !budgeted_search && tool_kind != ToolKind::DocumentDrift {
        Ok(rmcp::model::CallToolResult::success(vec![]))
    } else {
        match tool_kind {
            ToolKind::FindSymbol => {
                let name = arguments
                    .as_ref()
                    .and_then(|m| m.get("name"))
                    .and_then(|v| v.as_str())
                    .expect("required param validated upstream");
                let lang = arguments
                    .as_ref()
                    .and_then(|m| m.get("lang"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                server
                    .find_symbol_with_outcome(Parameters(FindSymbolRequest {
                        name: name.to_string(),
                        symbol_id: None,
                        lang,
                        limit: find_symbol_request
                            .as_ref()
                            .expect("validated request")
                            .limit,
                        offset: find_symbol_request
                            .as_ref()
                            .expect("validated request")
                            .offset,
                    }))
                    .await
                    .map(|response| {
                        text_exit = response.outcome.exit_code();
                        response.result
                    })
            }
            ToolKind::GetCalls => {
                let function_name = arguments
                    .as_ref()
                    .and_then(|m| m.get("function_name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let symbol_id = arguments
                    .as_ref()
                    .and_then(|m| m.get("symbol_id"))
                    .and_then(|v| v.as_u64())
                    .map(|id| id as u32);

                server
                    .get_calls_with_outcome(Parameters(GetCallsRequest {
                        function_name,
                        symbol_id,
                    }))
                    .await
                    .map(|response| {
                        text_exit = response.outcome.exit_code();
                        response.result
                    })
            }
            ToolKind::FindCallers => {
                let function_name = arguments
                    .as_ref()
                    .and_then(|m| m.get("function_name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let symbol_id = arguments
                    .as_ref()
                    .and_then(|m| m.get("symbol_id"))
                    .and_then(|v| v.as_u64())
                    .map(|id| id as u32);

                server
                    .find_callers_with_outcome(Parameters(FindCallersRequest {
                        function_name,
                        symbol_id,
                    }))
                    .await
                    .map(|response| {
                        text_exit = response.outcome.exit_code();
                        response.result
                    })
            }
            ToolKind::AnalyzeImpact => {
                let symbol_name = arguments
                    .as_ref()
                    .and_then(|m| m.get("symbol_name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let symbol_id = arguments
                    .as_ref()
                    .and_then(|m| m.get("symbol_id"))
                    .and_then(|v| v.as_u64())
                    .map(|id| id as u32);

                let max_depth = arguments
                    .as_ref()
                    .and_then(|m| m.get("max_depth"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3) as u32;
                server
                    .analyze_impact_with_outcome(Parameters(AnalyzeImpactRequest {
                        symbol_name,
                        symbol_id,
                        max_depth,
                    }))
                    .await
                    .map(|response| {
                        text_exit = response.outcome.exit_code();
                        response.result
                    })
            }
            ToolKind::GetIndexInfo => {
                use crate::mcp::GetIndexInfoRequest;
                use rmcp::handler::server::wrapper::Parameters;
                server
                    .get_index_info(Parameters(GetIndexInfoRequest {}))
                    .await
            }
            ToolKind::SearchSymbols => {
                let request = serde_json::from_value::<crate::mcp::SearchSymbolsRequest>(
                    serde_json::Value::Object(arguments.clone().unwrap_or_default()),
                )
                .unwrap_or_else(|error| {
                    exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                });
                server.search_symbols(Parameters(request)).await
            }
            ToolKind::SemanticSearchDocs => {
                let request = serde_json::from_value::<crate::mcp::SemanticSearchRequest>(
                    serde_json::Value::Object(arguments.clone().unwrap_or_default()),
                )
                .unwrap_or_else(|error| {
                    exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                });
                server.semantic_search_docs(Parameters(request)).await
            }
            ToolKind::SemanticSearchWithContext => {
                let request =
                    serde_json::from_value::<crate::mcp::SemanticSearchWithContextRequest>(
                        serde_json::Value::Object(arguments.clone().unwrap_or_default()),
                    )
                    .unwrap_or_else(|error| {
                        exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                    });
                server
                    .semantic_search_with_context(Parameters(request))
                    .await
            }
            ToolKind::DocumentDrift => {
                let request = serde_json::from_value::<crate::mcp::DocumentDriftRequest>(
                    serde_json::Value::Object(arguments.clone().unwrap_or_default()),
                )
                .unwrap_or_else(|error| {
                    exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                });
                server.document_drift(Parameters(request)).await
            }
            ToolKind::GetDocumentChunk => {
                let request = serde_json::from_value::<crate::mcp::GetDocumentChunkRequest>(
                    serde_json::Value::Object(arguments.clone().unwrap_or_default()),
                )
                .unwrap_or_else(|error| {
                    exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                });
                server.get_document_chunk(Parameters(request)).await
            }
            ToolKind::SearchDocuments => {
                server
                    .search_documents(Parameters(
                        document_request.expect("document request validated upstream"),
                    ))
                    .await
            }
            ToolKind::SearchTicketContext => {
                server
                    .search_ticket_context(Parameters(
                        ticket_context_request.expect("ticket context request validated upstream"),
                    ))
                    .await
            }
            ToolKind::SearchContext => {
                let request = serde_json::from_value::<crate::mcp::SearchContextRequest>(
                    serde_json::Value::Object(arguments.clone().unwrap_or_default()),
                )
                .unwrap_or_else(|error| {
                    exit_invalid_args(&tool, &error.to_string(), tool_param_spec(&tool).0, json)
                });
                server.search_context(Parameters(request)).await
            }
        }
    };

    // Print result
    match result {
        Ok(call_result) => {
            if json && budgeted_search {
                use crate::io::envelope::{Envelope, ResultCode};
                let data = call_result
                    .structured_content
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({"content":call_result.content}));
                let mut envelope = if call_result.is_error == Some(true) {
                    let mut error: Envelope<serde_json::Value> = Envelope::error(
                        ResultCode::InvalidQuery,
                        "Tool could not return evidence within this request",
                    );
                    error.error = Some(crate::io::envelope::ErrorDetails {
                        suggestions: vec![],
                        context: Some(data),
                    });
                    error
                } else if matches!(
                    tool_kind,
                    ToolKind::SearchSymbols
                        | ToolKind::SemanticSearchDocs
                        | ToolKind::SemanticSearchWithContext
                        | ToolKind::SearchDocuments
                ) && data["results"].as_array().is_some_and(Vec::is_empty)
                    && data["output"]["partial"] == false
                {
                    let message = if tool_kind == ToolKind::SearchDocuments {
                        "No matching indexed documents"
                    } else {
                        "No matching indexed symbols"
                    };
                    let mut not_found = Envelope::not_found(message);
                    not_found.data = Some(data);
                    not_found
                } else {
                    Envelope::success(data)
                };
                // The search payload changed from legacy arrays to the shared
                // structured result; advertise the migration, never masquerade as v1.
                envelope.meta.schema_version = "2.0.0".to_owned();
                let rendered = render_envelope_json(&envelope, fields.as_ref());
                // Minify the export envelope; the shared MCP packer already counts
                // both representations. Enforce this final CLI surface as well.
                let encoded = serde_json::from_str::<serde_json::Value>(&rendered)
                    .expect("serialized envelope")
                    .to_string();
                let budget = arguments
                    .as_ref()
                    .and_then(|map| map.get("max_output_bytes"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(crate::mcp::output::DEFAULT_OUTPUT_BYTES as u64)
                    as usize;
                if encoded.len() + 1 > budget {
                    let error: Envelope<()> = Envelope::error(
                        ResultCode::InvalidQuery,
                        "Export envelope exceeds max_output_bytes; narrow the request or raise its budget",
                    );
                    emit_envelope_and_exit(error);
                }
                println!("{encoded}");
                if envelope.exit_code != 0 {
                    std::process::exit(envelope.exit_code.into());
                }
                return;
            }
            if json && tool == "document_drift" {
                let data = call_result
                    .structured_content
                    .unwrap_or(serde_json::Value::Null);
                let envelope = crate::io::envelope::Envelope::success(data)
                    .with_message("Document source drift inspection completed");
                println!("{}", render_envelope_json(&envelope, fields.as_ref()));
            } else if json && tool == "get_index_info" {
                use crate::io::envelope::Envelope;
                use crate::io::guidance_engine::generate_guidance_from_config;

                if let Some(index_info) = index_info_data {
                    let mut envelope =
                        Envelope::success(index_info).with_message("Index statistics");

                    if let Some(hint) =
                        generate_guidance_from_config(&guidance_config, "get_index_info", None, 1)
                    {
                        envelope = envelope.with_hint(hint);
                    }

                    let output = render_envelope_json(&envelope, fields.as_ref());
                    println!("{output}");
                    if envelope.exit_code != 0 {
                        std::process::exit(envelope.exit_code.into());
                    }
                }
            } else if json && tool == "find_symbol" {
                // Use pre-collected data for JSON output
                if let Some((symbol_contexts, page, semantic_definitions)) = find_symbol_data {
                    use crate::io::envelope::{EntityType, Envelope};
                    use crate::io::guidance_engine::generate_guidance_from_config;

                    let name = arguments
                        .as_ref()
                        .and_then(|m| m.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let language = arguments
                        .as_ref()
                        .and_then(|m| m.get("lang"))
                        .and_then(|v| v.as_str());

                    if page.total == 0 {
                        let mut envelope: Envelope<()> =
                            Envelope::not_found(format!("Symbol '{name}' not found"))
                                .with_entity_type(EntityType::Symbol)
                                .with_query(name);
                        envelope.meta.semantic_definitions =
                            Some(serde_json::json!(semantic_definitions));
                        envelope.meta.total = Some(page.total);
                        envelope.meta.offset = Some(page.offset);
                        envelope.meta.limit = Some(page.limit);
                        envelope.meta.truncated = Some(false);

                        if let Some(lang) = language {
                            envelope = envelope.with_lang(lang);
                        }

                        if let Some(hint) = generate_guidance_from_config(
                            &guidance_config,
                            "find_symbol",
                            Some(name),
                            0,
                        ) {
                            envelope = envelope.with_hint(hint);
                        }

                        // Envelope serialization is infallible for simple types
                        emit_envelope_and_exit(envelope);
                    } else {
                        let count = symbol_contexts.len();
                        let mut envelope = Envelope::success(symbol_contexts)
                            .with_entity_type(EntityType::Symbol)
                            .with_count(count)
                            .with_query(name)
                            .with_message(format!("Found {count} symbol(s)"));
                        envelope.meta.semantic_definitions =
                            Some(serde_json::json!(semantic_definitions));
                        envelope.meta.total = Some(page.total);
                        envelope.meta.offset = Some(page.offset);
                        envelope.meta.limit = Some(page.limit);
                        envelope.meta.next_offset = page.next_offset;
                        envelope.meta.truncated = Some(page.returned < page.total);

                        if let Some(lang) = language {
                            envelope = envelope.with_lang(lang);
                        }

                        if let Some(hint) = generate_guidance_from_config(
                            &guidance_config,
                            "find_symbol",
                            Some(name),
                            count,
                        ) {
                            envelope = envelope.with_hint(hint);
                        }

                        let output = render_envelope_json(&envelope, fields.as_ref());
                        println!("{output}");
                    }
                }
            } else if json && tool == "get_calls" {
                use crate::io::envelope::{EntityType, Envelope};
                use crate::io::guidance_engine::generate_guidance_from_config;

                let identifier = if let Some(id) = arguments
                    .as_ref()
                    .and_then(|m| m.get("symbol_id"))
                    .and_then(|v| v.as_u64())
                {
                    format!("symbol_id:{id}")
                } else {
                    arguments
                        .as_ref()
                        .and_then(|m| m.get("function_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string()
                };
                let language = arguments
                    .as_ref()
                    .and_then(|m| m.get("lang"))
                    .and_then(|v| v.as_str());

                if let Some(calls) = get_calls_data {
                    let count = calls.len();
                    let mut envelope = Envelope::success(calls)
                        .with_entity_type(EntityType::Calls)
                        .with_count(count)
                        .with_query(&identifier)
                        .with_message(format!("Calls {count} function(s)"));

                    if let Some(lang) = language {
                        envelope = envelope.with_lang(lang);
                    }

                    if let Some(hint) = generate_guidance_from_config(
                        &guidance_config,
                        "get_calls",
                        Some(&identifier),
                        count,
                    ) {
                        envelope = envelope.with_hint(hint);
                    }

                    let output = render_envelope_json(&envelope, fields.as_ref());
                    println!("{output}");
                    if envelope.exit_code != 0 {
                        std::process::exit(envelope.exit_code.into());
                    }
                } else {
                    let mut envelope: Envelope<()> =
                        Envelope::not_found(format!("Function '{identifier}' not found"))
                            .with_entity_type(EntityType::Calls)
                            .with_query(&identifier);

                    if let Some(lang) = language {
                        envelope = envelope.with_lang(lang);
                    }

                    if let Some(hint) = generate_guidance_from_config(
                        &guidance_config,
                        "get_calls",
                        Some(&identifier),
                        0,
                    ) {
                        envelope = envelope.with_hint(hint);
                    }

                    emit_envelope_and_exit(envelope);
                }
            } else if json && tool == "find_callers" {
                use crate::io::envelope::{EntityType, Envelope};
                use crate::io::guidance_engine::generate_guidance_from_config;

                let identifier = if let Some(id) = arguments
                    .as_ref()
                    .and_then(|m| m.get("symbol_id"))
                    .and_then(|v| v.as_u64())
                {
                    format!("symbol_id:{id}")
                } else {
                    arguments
                        .as_ref()
                        .and_then(|m| m.get("function_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string()
                };
                let language = arguments
                    .as_ref()
                    .and_then(|m| m.get("lang"))
                    .and_then(|v| v.as_str());

                if let Some(callers) = find_callers_data {
                    let count = callers.len();
                    let mut envelope = Envelope::success(callers)
                        .with_entity_type(EntityType::Callers)
                        .with_count(count)
                        .with_query(&identifier)
                        .with_message(format!("Called by {count} function(s)"));

                    if let Some(lang) = language {
                        envelope = envelope.with_lang(lang);
                    }

                    if let Some(hint) = generate_guidance_from_config(
                        &guidance_config,
                        "find_callers",
                        Some(&identifier),
                        count,
                    ) {
                        envelope = envelope.with_hint(hint);
                    }

                    let output = render_envelope_json(&envelope, fields.as_ref());
                    println!("{output}");
                    if envelope.exit_code != 0 {
                        std::process::exit(envelope.exit_code.into());
                    }
                } else {
                    let mut envelope: Envelope<()> =
                        Envelope::not_found(format!("Function '{identifier}' not found"))
                            .with_entity_type(EntityType::Callers)
                            .with_query(&identifier);

                    if let Some(lang) = language {
                        envelope = envelope.with_lang(lang);
                    }

                    if let Some(hint) = generate_guidance_from_config(
                        &guidance_config,
                        "find_callers",
                        Some(&identifier),
                        0,
                    ) {
                        envelope = envelope.with_hint(hint);
                    }

                    emit_envelope_and_exit(envelope);
                }
            } else if json && tool == "analyze_impact" {
                use crate::io::envelope::{EntityType, Envelope};
                use crate::io::guidance_engine::generate_guidance_from_config;

                // Get identifier for messages
                let identifier = if let Some(id) = arguments
                    .as_ref()
                    .and_then(|m| m.get("symbol_id"))
                    .and_then(|v| v.as_u64())
                {
                    format!("symbol_id:{id}")
                } else {
                    arguments
                        .as_ref()
                        .and_then(|m| m.get("symbol_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string()
                };

                let max_depth = arguments
                    .as_ref()
                    .and_then(|m| m.get("max_depth"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3) as u32;

                if let Some(impacted) = analyze_impact_data {
                    let count = impacted.len();
                    let mut envelope = Envelope::success(impacted)
                        .with_entity_type(EntityType::ImpactGraph)
                        .with_count(count)
                        .with_query(&identifier)
                        .with_depth(max_depth)
                        .with_message(format!("{count} symbol(s) would be impacted"));

                    if let Some(hint) = generate_guidance_from_config(
                        &guidance_config,
                        "analyze_impact",
                        Some(&identifier),
                        count,
                    ) {
                        envelope = envelope.with_hint(hint);
                    }

                    let output = render_envelope_json(&envelope, fields.as_ref());
                    println!("{output}");
                    if envelope.exit_code != 0 {
                        std::process::exit(envelope.exit_code.into());
                    }
                } else {
                    // Symbol not found
                    let mut envelope: Envelope<()> =
                        Envelope::not_found(format!("Symbol '{identifier}' not found"))
                            .with_entity_type(EntityType::ImpactGraph)
                            .with_query(&identifier);

                    if let Some(hint) = generate_guidance_from_config(
                        &guidance_config,
                        "analyze_impact",
                        Some(&identifier),
                        0,
                    ) {
                        envelope = envelope.with_hint(hint);
                    }

                    emit_envelope_and_exit(envelope);
                }
            } else {
                // Default text output
                for content in &call_result.content {
                    match content {
                        rmcp::model::ContentBlock::Text(text_content) => {
                            println!("{}", text_content.text);
                        }
                        _ => {
                            eprintln!("Warning: Non-text content returned");
                        }
                    }
                }
                if call_result.is_error == Some(true) && budgeted_search {
                    std::process::exit(2);
                }
                if text_exit != 0 {
                    std::process::exit(text_exit);
                }
            }
        }
        Err(e) => {
            if json {
                use crate::io::envelope::{Envelope, ResultCode};
                let code = if e.code == rmcp::model::ErrorCode::INVALID_PARAMS {
                    ResultCode::InvalidQuery
                } else {
                    ResultCode::InternalError
                };
                let envelope: Envelope<()> = Envelope::error(code, e.message.to_string())
                    .with_hint("Check the tool name and arguments");

                emit_envelope_and_exit(envelope);
            } else {
                eprintln!("Error calling tool: {}", e.message);
                std::process::exit(2);
            }
        }
    }
}
