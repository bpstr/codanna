//! Search and info tools: get_index_info, semantic_search_docs,
//! semantic_search_with_context, search_symbols, search_documents.

use rmcp::model::ErrorData as McpError;
use rmcp::model::*;
use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};

use crate::documents::SearchQuery as DocSearchQuery;

use crate::mcp::requests::{
    GetIndexInfoRequest, SearchDocumentsRequest, SearchSymbolsRequest, SemanticSearchRequest,
    SemanticSearchWithContextRequest,
};
use crate::mcp::server::{CodeIntelligenceServer, format_relative_time, generate_mcp_guidance};

#[tool_router(router = search_router, vis = "pub(crate)")]
impl CodeIntelligenceServer {
    #[tool(description = "Get information about the indexed codebase")]
    pub async fn get_index_info(
        &self,
        Parameters(_params): Parameters<GetIndexInfoRequest>,
    ) -> Result<CallToolResult, McpError> {
        crate::runtime::read(&self.facade, move |indexer| {
            let symbols = indexer.get_all_symbols();
            let symbol_count = symbols.len();
            let file_count = indexer.file_count();
            let relationship_count = indexer.relationship_count();

            let mut kind_counts = std::collections::BTreeMap::new();
            let mut language_counts = std::collections::BTreeMap::new();
            for symbol in &symbols {
                *kind_counts.entry(format!("{:?}", symbol.kind)).or_insert(0usize) += 1;
                if let Some(language) = symbol.language_id.as_ref() {
                    *language_counts
                        .entry(language.as_str().to_string())
                        .or_insert(0usize) += 1;
                }
            }

            let mut kinds_display = String::new();
            for (kind, count) in &kind_counts {
                kinds_display.push_str(&format!("\n  - {kind}s: {count}"));
            }
            let mut languages_display = String::new();
            for (language, count) in &language_counts {
                languages_display.push_str(&format!("\n  - {language}: {count}"));
            }

            let semantic = indexer.semantic_coverage_status(&symbols);
            let metadata = indexer.get_semantic_metadata();
            let display = |value: Option<usize>| {
                value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            };
            let model = semantic.model_name.as_deref().unwrap_or("unknown");
            let dimension = semantic
                .dimension
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let input_policy = semantic
                .embedding_input_policy
                .as_deref()
                .unwrap_or("unknown");
            let timestamp_info = metadata.as_ref().map_or_else(
                String::new,
                |metadata| {
                    format!(
                        "\n  - Created: {}\n  - Updated: {}",
                        format_relative_time(metadata.created_at),
                        format_relative_time(metadata.updated_at)
                    )
                },
            );
            let semantic_info = format!(
                "\n\nSemantic Search:\n  - Status: {}\n  - Model: {}\n  - Dimensions: {}\n  - Source eligibility policy: {}\n  - Eligible symbols: {}\n  - Vectors: {}\n  - Eligible with vector: {}\n  - Eligible without vector: {}\n  - Orphan vectors: {}\n  - Skipped symbols: unknown\n  - Pending symbols: unknown\n  - Embedding input policy: {}\n  - Code generation: {}\n  - Vector code generation: unknown\n  - Generation alignment: {}\n  - Freshness: {}{}",
                semantic.state,
                model,
                dimension,
                semantic.source_input_policy,
                semantic.eligible_symbols,
                display(semantic.vector_count),
                display(semantic.eligible_with_vector),
                display(semantic.eligible_without_vector),
                display(semantic.vector_without_current_symbol),
                input_policy,
                semantic.code_generation,
                semantic.generation_alignment,
                semantic.freshness,
                timestamp_info,
            );

            let representation_info = format!("\n  - Recorded source policy: {}\n  - Representation status: {}\n  - Action: {}\n  - Source coverage: {}", semantic.recorded_source_policy.as_deref().unwrap_or("unknown"), semantic.representation_status, semantic.representation_action, semantic.source_coverage);
            let result = format!(
                "Index contains {symbol_count} symbols across {file_count} files.\n\nBreakdown:\n  - Symbols: {symbol_count}\n  - Relationships: {relationship_count}\n\nSymbol Kinds:{kinds_display}\n\nLanguages:{languages_display}{semantic_info}{representation_info}"
            );

            let mut response = CallToolResult::success(vec![ContentBlock::text(result)]);
            response.structured_content = Some(serde_json::json!({
                "index": {
                    "symbols": symbol_count,
                    "files": file_count,
                    "relationships": relationship_count,
                    "code_generation": semantic.code_generation,
                    "symbol_kinds": kind_counts,
                    "languages": language_counts,
                },
                "semantic": semantic,
            }));
            Ok(response)
        })
        .await
        .map_err(|error| McpError::internal_error(error.to_string(), None))?
    }

    #[tool(description = "Search documentation using natural language semantic search")]
    pub async fn semantic_search_docs(
        &self,
        Parameters(SemanticSearchRequest {
            query,
            limit,
            threshold,
            lang,
        }): Parameters<SemanticSearchRequest>,
    ) -> Result<CallToolResult, McpError> {
        crate::mcp::requests::validate_search_limit(limit)?;
        if let Err(error) = self.prepare_semantic_query().await {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Semantic search failed: {error}\nNo code or semantic index rebuild was attempted by this query. Use search_symbols or search_context for lexical code discovery."
            ))]));
        }
        crate::runtime::read(&self.facade, move |indexer| {

        tracing::debug!(
            target: "mcp",
            "semantic_search_docs called - symbols: {}, semantic: {}",
            indexer.symbol_count(),
            indexer.has_semantic_search()
        );

        if !indexer.has_semantic_search() {
            // Check if semantic files exist
            let semantic_path = indexer.settings().index_path.join("semantic");
            let metadata_exists = semantic_path.join("metadata.json").exists();
            let vectors_exist = semantic_path.join("metadata.json").exists();
            let symbol_count = indexer.symbol_count();

            // Get current working directory for debugging
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "unknown".to_string());

            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Semantic search is not enabled. No code or semantic index rebuild was attempted by this query. Use search_symbols or search_context for lexical code discovery. Enabling semantic indexing is a separate explicit operation.\n\nDEBUG INFO:\n- Index path: {}\n- Symbol count: {}\n- Semantic files exist: {}\n- Has semantic search: {}\n- Working dir: {}",
                crate::parsing::paths::render_absolute_path(&indexer.settings().index_path)
                    .display(),
                symbol_count,
                metadata_exists && vectors_exist,
                indexer.has_semantic_search(),
                cwd
            ))]));
        }

        let results = match threshold {
            Some(t) => indexer.semantic_search_docs_with_threshold_and_language(
                &query,
                limit as usize,
                t,
                lang.as_deref(),
            ),
            None => {
                indexer.semantic_search_docs_with_language(&query, limit as usize, lang.as_deref())
            }
        };

        match results {
            Ok(results) => {
                if results.is_empty() {
                    let mut output =
                        format!("No semantically similar documentation found for: {query}");
                    // Add guidance for no results
                    if let Some(guidance) =
                        generate_mcp_guidance(indexer.settings(), "semantic_search_docs", 0)
                    {
                        output.push_str("\n\n---\nGuidance: ");
                        output.push_str(&guidance);
                        output.push('\n');
                    }
                    return Ok(CallToolResult::success(vec![ContentBlock::text(output)]));
                }

                let mut result = format!(
                    "Found {} semantically similar result(s) for '{}':\n\n",
                    results.len(),
                    query
                );

                for (i, (symbol, score)) in results.iter().enumerate() {
                    result.push_str(&format!(
                        "{}. {} ({:?}) - Similarity: {:.3}\n",
                        i + 1,
                        symbol.name,
                        symbol.kind,
                        score
                    ));
                    result.push_str(&format!(
                        "   File: {}:{}\n",
                        symbol.file_path,
                        symbol.range.start_line + 1
                    ));

                    if let Some(ref doc) = symbol.doc_comment {
                        // Show first 3 lines of doc
                        let preview: Vec<&str> = doc.lines().take(3).collect();
                        let doc_preview = if doc.lines().count() > 3 {
                            format!("{}...", preview.join(" "))
                        } else {
                            preview.join(" ")
                        };
                        result.push_str(&format!("   Doc: {doc_preview}\n"));
                    }

                    if let Some(ref sig) = symbol.signature {
                        result.push_str(&format!("   Signature: {sig}\n"));
                    }

                    result.push('\n');
                }

                // Add system guidance
                if let Some(guidance) =
                    generate_mcp_guidance(indexer.settings(), "semantic_search_docs", results.len())
                {
                    result.push_str("\n---\nGuidance: ");
                    result.push_str(&guidance);
                    result.push('\n');
                }

                Ok(CallToolResult::success(vec![ContentBlock::text(result)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Semantic search failed: {e}"
            ))])),
        }
        }).await.map_err(|error| McpError::internal_error(error.to_string(), None))?
    }

    #[tool(
        description = "Search by natural language with documentation, dependency previews, callers and bounded impact. Returns each matching symbol even when its impact expansion exceeds the graph budget. Structured impact metadata reports complete, budget_exceeded or unavailable for each function/method; a missing count is not zero impact."
    )]
    pub async fn semantic_search_with_context(
        &self,
        Parameters(SemanticSearchWithContextRequest {
            query,
            limit,
            threshold,
            lang,
        }): Parameters<SemanticSearchWithContextRequest>,
    ) -> Result<CallToolResult, McpError> {
        crate::mcp::requests::validate_search_limit(limit)?;
        if let Err(error) = self.prepare_semantic_query().await {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Semantic search failed: {error}\nNo code or semantic index rebuild was attempted by this query. Use search_symbols or search_context for lexical code discovery."
            ))]));
        }
        crate::runtime::read(&self.facade, move |indexer| {

        if !indexer.has_semantic_search() {
            tracing::debug!(
                target: "mcp",
                "semantic search not available - index_path: {}, has_semantic: {}",
                crate::parsing::paths::render_absolute_path(&indexer.settings().index_path).display(),
                indexer.has_semantic_search()
            );
            // Check if semantic files exist
            let semantic_path = indexer.settings().index_path.join("semantic");
            let metadata_exists = semantic_path.join("metadata.json").exists();
            let vectors_exist = semantic_path.join("metadata.json").exists();

            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Semantic search is not enabled. No code or semantic index rebuild was attempted by this query. Use search_symbols or search_context for lexical code discovery. Enabling semantic indexing is a separate explicit operation.\n\nDEBUG INFO:\n- Index path: {}\n- Has semantic search: {}\n- Semantic path: {}\n- Metadata exists: {}\n- Vectors exist: {}",
                crate::parsing::paths::render_absolute_path(&indexer.settings().index_path)
                    .display(),
                indexer.has_semantic_search(),
                crate::parsing::paths::render_absolute_path(&semantic_path).display(),
                metadata_exists,
                vectors_exist
            ))]));
        }

        // First, perform semantic search
        let search_results = match threshold {
            Some(t) => indexer.semantic_search_docs_with_threshold_and_language(
                &query,
                limit as usize,
                t,
                lang.as_deref(),
            ),
            None => {
                indexer.semantic_search_docs_with_language(&query, limit as usize, lang.as_deref())
            }
        };

        match search_results {
            Ok(results) => {
                if results.is_empty() {
                    let mut output = format!("No documentation found matching query: {query}");
                    // Add guidance for no results
                    if let Some(guidance) =
                        generate_mcp_guidance(indexer.settings(), "semantic_search_with_context", 0)
                    {
                        output.push_str("\n\n---\nGuidance: ");
                        output.push_str(&guidance);
                        output.push('\n');
                    }
                    return Ok(CallToolResult::success(vec![ContentBlock::text(output)]));
                }

                let mut output = String::new();
                output.push_str(&format!(
                    "Found {} results for query: '{}'\n\n",
                    results.len(),
                    query
                ));

                let mut impact_contexts = Vec::new();
                // For each result, gather comprehensive context
                for (idx, (symbol, score)) in results.iter().enumerate() {
                    // Basic symbol information - matching find_symbol format
                    output.push_str(&format!(
                        "{}. {} - {:?} at {} [symbol_id:{}]\n",
                        idx + 1,
                        symbol.name,
                        symbol.kind,
                        crate::symbol::context::SymbolContext::symbol_location(symbol),
                        symbol.id.value()
                    ));
                    output.push_str(&format!("   Similarity Score: {score:.3}\n"));

                    // Documentation
                    if let Some(ref doc) = symbol.doc_comment {
                        output.push_str("   Documentation:\n");
                        for line in doc.lines().take(5) {
                            output.push_str(&format!("     {line}\n"));
                        }
                        if doc.lines().count() > 5 {
                            output.push_str("     ...\n");
                        }
                    }

                    // Signature
                    if let Some(ref sig) = symbol.signature {
                        output.push_str(&format!("   Signature: {sig}\n"));
                    }

                    // Only gather additional context for functions/methods
                    if matches!(
                        symbol.kind,
                        crate::SymbolKind::Function | crate::SymbolKind::Method
                    ) {
                        // Dependencies (what this function calls) - using logic from get_calls
                        let (called_with_metadata, called_with_metadata_total) = indexer.graph_neighbor_preview(symbol.id, crate::RelationKind::Calls, false, 10)
                            .map_err(|error|McpError::internal_error(error.to_string(),None))?;
                        if !called_with_metadata.is_empty() {
                            output.push_str(&format!(
                                "\n   {} calls {} function(s):\n",
                                symbol.name,
                                called_with_metadata_total
                            ));
                            for (i, (called, metadata)) in
                                called_with_metadata.iter().take(10).enumerate()
                            {
                                // Parse receiver information from metadata and get call site location
                                let (call_display, call_line) = if let Some(meta) = metadata {
                                    let display = if let Some(context) = &meta.context {
                                        if context.contains("receiver:")
                                            && context.contains("static:")
                                        {
                                            let parts: Vec<&str> = context.split(',').collect();
                                            let mut receiver = None;
                                            let mut is_static = false;

                                            for part in parts {
                                                if let Some(recv) = part.strip_prefix("receiver:") {
                                                    receiver = Some(recv.trim());
                                                } else if let Some(static_val) =
                                                    part.strip_prefix("static:")
                                                {
                                                    is_static = static_val.trim() == "true";
                                                }
                                            }

                                            match (receiver, is_static) {
                                                (Some("self"), false) => {
                                                    format!("(self.{})", called.name)
                                                }
                                                (Some(recv), true) if recv != "self" => {
                                                    format!("({}::{})", recv, called.name)
                                                }
                                                (Some(recv), false) if recv != "self" => {
                                                    format!("({}.{})", recv, called.name)
                                                }
                                                _ => called.name.to_string(),
                                            }
                                        } else {
                                            called.name.to_string()
                                        }
                                    } else {
                                        called.name.to_string()
                                    };

                                    (display, meta.line)
                                } else {
                                    (called.name.to_string(), None)
                                };

                                // Callee def location; the call site lives in
                                // THIS symbol's file, named explicitly.
                                let call_site = call_line
                                    .map(|l| format!(" (called at {}:{})", symbol.file_path, l + 1))
                                    .unwrap_or_default();
                                output.push_str(&format!(
                                    "     -> {:?} {} at {}:{} [symbol_id:{}]{}\n",
                                    called.kind,
                                    call_display,
                                    called.file_path,
                                    called.range.start_line + 1,
                                    called.id.value(),
                                    call_site
                                ));
                                if i == 9 && called_with_metadata_total > 10 {
                                    output.push_str(&format!(
                                        "     ... and {} more\n",
                                        called_with_metadata_total - 10
                                    ));
                                }
                            }
                        }

                        // Callers (who uses this function) - using logic from find_callers
                        let (calling_functions_with_metadata, calling_functions_with_metadata_total) = indexer.graph_neighbor_preview(symbol.id, crate::RelationKind::Calls, true, 10)
                            .map_err(|error|McpError::internal_error(error.to_string(),None))?;
                        if !calling_functions_with_metadata.is_empty() {
                            output.push_str(&format!(
                                "\n   {} function(s) call {}:\n",
                                calling_functions_with_metadata_total,
                                symbol.name
                            ));
                            for (i, (caller, metadata)) in
                                calling_functions_with_metadata.iter().take(10).enumerate()
                            {
                                // Parse metadata to extract receiver info and call site location
                                let (call_info, call_line) = if let Some(meta) = metadata {
                                    let info = if let Some(context) = &meta.context {
                                        if context.contains("receiver:")
                                            && context.contains("static:")
                                        {
                                            // Parse "receiver:{receiver},static:{is_static}"
                                            let parts: Vec<&str> = context.split(',').collect();
                                            let mut receiver = "";
                                            let mut is_static = false;

                                            for part in parts {
                                                if let Some(r) = part.strip_prefix("receiver:") {
                                                    receiver = r;
                                                } else if let Some(s) = part.strip_prefix("static:")
                                                {
                                                    is_static = s == "true";
                                                }
                                            }

                                            if !receiver.is_empty() {
                                                let qualified_name = if is_static {
                                                    format!("{}::{}", receiver, symbol.name)
                                                } else {
                                                    format!("{}.{}", receiver, symbol.name)
                                                };
                                                format!(" (calls {qualified_name})")
                                            } else {
                                                String::new()
                                            }
                                        } else {
                                            String::new()
                                        }
                                    } else {
                                        String::new()
                                    };

                                    // Use call site line if available
                                    let line = meta
                                        .line
                                        .map(|l| l + 1)
                                        .unwrap_or(caller.range.start_line + 1);
                                    (info, line)
                                } else {
                                    (String::new(), caller.range.start_line + 1)
                                };

                                output.push_str(&format!(
                                    "     <- {:?} {} at {}:{}{} [symbol_id:{}]\n",
                                    caller.kind,
                                    caller.name,
                                    caller.file_path,
                                    call_line,
                                    call_info,
                                    caller.id.value()
                                ));
                                if i == 9 && calling_functions_with_metadata_total > 10 {
                                    output.push_str(&format!(
                                        "     ... and {} more\n",
                                        calling_functions_with_metadata_total - 10
                                    ));
                                }
                            }
                        }

                        // Impact analysis - using logic from analyze_impact
                        let (impacted, impact_context) = crate::mcp::service::impact_context(&indexer, symbol.id, 2);
                        if let Some(reason) = &impact_context.reason {
                            output.push_str(&format!(
                                "\n   Impact expansion incomplete: {reason}. Other search matches remain available.\n"
                            ));
                        }
                        impact_contexts.push(impact_context);
                        if !impacted.is_empty() {
                            output.push_str(&format!(
                                "\n   Changing {} would impact {} symbol(s) (max depth: 2):\n",
                                symbol.name,
                                impacted.len()
                            ));

                            // Get details and group by kind
                            let impacted_details = indexer.get_symbols(&impacted)
                                .map_err(|error| McpError::internal_error(error.to_string(), None))?;

                            // Group by kind
                            let mut methods = Vec::new();
                            let mut functions = Vec::new();
                            let mut other = Vec::new();

                            for sym in impacted_details {
                                match sym.kind {
                                    crate::SymbolKind::Method => methods.push(sym),
                                    crate::SymbolKind::Function => functions.push(sym),
                                    _ => other.push(sym),
                                }
                            }

                            if !methods.is_empty() {
                                output.push_str(&format!("\n     methods ({}):\n", methods.len()));
                                for method in methods.iter().take(5) {
                                    output.push_str(&format!(
                                        "       - {} [symbol_id:{}]\n",
                                        method.name,
                                        method.id.value()
                                    ));
                                }
                                if methods.len() > 5 {
                                    output.push_str(&format!(
                                        "       ... and {} more\n",
                                        methods.len() - 5
                                    ));
                                }
                            }

                            if !functions.is_empty() {
                                output.push_str(&format!(
                                    "\n     functions ({}):\n",
                                    functions.len()
                                ));
                                for func in functions.iter().take(5) {
                                    output.push_str(&format!(
                                        "       - {} [symbol_id:{}]\n",
                                        func.name,
                                        func.id.value()
                                    ));
                                }
                                if functions.len() > 5 {
                                    output.push_str(&format!(
                                        "       ... and {} more\n",
                                        functions.len() - 5
                                    ));
                                }
                            }

                            if !other.is_empty() {
                                output.push_str(&format!("\n     other ({}):\n", other.len()));
                                for sym in other.iter().take(3) {
                                    output.push_str(&format!(
                                        "       - {} ({:?}) [symbol_id:{}]\n",
                                        sym.name,
                                        sym.kind,
                                        sym.id.value()
                                    ));
                                }
                            }
                        }
                    }

                    // Show inheritance relationships for classes/structs/enums
                    if matches!(
                        symbol.kind,
                        crate::SymbolKind::Class
                            | crate::SymbolKind::Struct
                            | crate::SymbolKind::Enum
                    ) {
                        // What does this class extend?
                        let (extends, extends_total) = indexer.graph_neighbor_preview(symbol.id, crate::RelationKind::Extends, false, 5)
                            .map_err(|error|McpError::internal_error(error.to_string(),None))?;
                        let extends: Vec<_> = extends.into_iter().map(|(symbol,_)|symbol).collect();
                        if !extends.is_empty() {
                            output.push_str(&format!(
                                "\n   {} extends {} class(es):\n",
                                symbol.name,
                                extends_total
                            ));
                            for (i, base_class) in extends.iter().take(5).enumerate() {
                                output.push_str(&format!(
                                    "     -> {:?} {} at {} [symbol_id:{}]\n",
                                    base_class.kind,
                                    base_class.name,
                                    crate::symbol::context::SymbolContext::symbol_location(
                                        base_class
                                    ),
                                    base_class.id.value()
                                ));
                                if i == 4 && extends_total > 5 {
                                    output.push_str(&format!(
                                        "     ... and {} more\n",
                                        extends_total - 5
                                    ));
                                }
                            }
                        }

                        // What classes extend this class?
                        let (extended_by, extended_by_total) = indexer.graph_neighbor_preview(symbol.id, crate::RelationKind::Extends, true, 5)
                            .map_err(|error|McpError::internal_error(error.to_string(),None))?;
                        let extended_by: Vec<_> = extended_by.into_iter().map(|(symbol,_)|symbol).collect();
                        if !extended_by.is_empty() {
                            output.push_str(&format!(
                                "\n   {} class(es) extend {}:\n",
                                extended_by_total,
                                symbol.name
                            ));
                            for (i, derived_class) in extended_by.iter().take(5).enumerate() {
                                output.push_str(&format!(
                                    "     <- {:?} {} at {} [symbol_id:{}]\n",
                                    derived_class.kind,
                                    derived_class.name,
                                    crate::symbol::context::SymbolContext::symbol_location(
                                        derived_class
                                    ),
                                    derived_class.id.value()
                                ));
                                if i == 4 && extended_by_total > 5 {
                                    output.push_str(&format!(
                                        "     ... and {} more\n",
                                        extended_by_total - 5
                                    ));
                                }
                            }
                        }

                        // What traits does this type implement?
                        let (implements, implements_total) = indexer.graph_neighbor_preview(symbol.id, crate::RelationKind::Implements, false, 5)
                            .map_err(|error|McpError::internal_error(error.to_string(),None))?;
                        let implements: Vec<_> = implements.into_iter().map(|(symbol,_)|symbol).collect();
                        if !implements.is_empty() {
                            output.push_str(&format!(
                                "\n   {} implements {} trait(s):\n",
                                symbol.name,
                                implements_total
                            ));
                            for (i, trait_sym) in implements.iter().take(5).enumerate() {
                                output.push_str(&format!(
                                    "     -> {:?} {} at {} [symbol_id:{}]\n",
                                    trait_sym.kind,
                                    trait_sym.name,
                                    crate::symbol::context::SymbolContext::symbol_location(
                                        trait_sym
                                    ),
                                    trait_sym.id.value()
                                ));
                                if i == 4 && implements_total > 5 {
                                    output.push_str(&format!(
                                        "     ... and {} more\n",
                                        implements_total - 5
                                    ));
                                }
                            }
                        }
                    }

                    // Show what implements this trait/interface
                    if matches!(
                        symbol.kind,
                        crate::SymbolKind::Trait | crate::SymbolKind::Interface
                    ) {
                        let (implementations, implementations_total) = indexer.graph_neighbor_preview(symbol.id, crate::RelationKind::Implements, true, 5)
                            .map_err(|error|McpError::internal_error(error.to_string(),None))?;
                        let implementations: Vec<_> = implementations.into_iter().map(|(symbol,_)|symbol).collect();
                        if !implementations.is_empty() {
                            output.push_str(&format!(
                                "\n   {} type(s) implement {}:\n",
                                implementations_total,
                                symbol.name
                            ));
                            for (i, impl_sym) in implementations.iter().take(5).enumerate() {
                                output.push_str(&format!(
                                    "     <- {:?} {} at {} [symbol_id:{}]\n",
                                    impl_sym.kind,
                                    impl_sym.name,
                                    crate::symbol::context::SymbolContext::symbol_location(
                                        impl_sym
                                    ),
                                    impl_sym.id.value()
                                ));
                                if i == 4 && implementations_total > 5 {
                                    output.push_str(&format!(
                                        "     ... and {} more\n",
                                        implementations_total - 5
                                    ));
                                }
                            }
                        }
                    }

                    // Show uses relationships (for all symbols)
                    let (uses, uses_total) = indexer.graph_neighbor_preview(symbol.id, crate::RelationKind::Uses, false, 5)
                            .map_err(|error|McpError::internal_error(error.to_string(),None))?;
                        let uses: Vec<_> = uses.into_iter().map(|(symbol,_)|symbol).collect();
                    if !uses.is_empty() {
                        output.push_str(&format!(
                            "\n   {} uses {} type(s):\n",
                            symbol.name,
                            uses_total
                        ));
                        for (i, used_type) in uses.iter().take(5).enumerate() {
                            output.push_str(&format!(
                                "     -> {:?} {} at {} [symbol_id:{}]\n",
                                used_type.kind,
                                used_type.name,
                                crate::symbol::context::SymbolContext::symbol_location(used_type),
                                used_type.id.value()
                            ));
                            if i == 4 && uses_total > 5 {
                                output.push_str(&format!("     ... and {} more\n", uses_total - 5));
                            }
                        }
                    }

                    // What symbols use this type?
                    let (used_by, used_by_total) = indexer.graph_neighbor_preview(symbol.id, crate::RelationKind::Uses, true, 5)
                            .map_err(|error|McpError::internal_error(error.to_string(),None))?;
                        let used_by: Vec<_> = used_by.into_iter().map(|(symbol,_)|symbol).collect();
                    if !used_by.is_empty() {
                        output.push_str(&format!(
                            "\n   {} type(s) use {}:\n",
                            used_by_total,
                            symbol.name
                        ));
                        for (i, using_symbol) in used_by.iter().take(5).enumerate() {
                            output.push_str(&format!(
                                "     <- {:?} {} at {} [symbol_id:{}]\n",
                                using_symbol.kind,
                                using_symbol.name,
                                crate::symbol::context::SymbolContext::symbol_location(
                                    using_symbol
                                ),
                                using_symbol.id.value()
                            ));
                            if i == 4 && used_by_total > 5 {
                                output.push_str(&format!(
                                    "     ... and {} more\n",
                                    used_by_total - 5
                                ));
                            }
                        }
                    }

                    output.push('\n');
                }

                // Add system guidance
                if let Some(guidance) = generate_mcp_guidance(
                    indexer.settings(),
                    "semantic_search_with_context",
                    results.len(),
                ) {
                    output.push_str("\n---\nGuidance: ");
                    output.push_str(&guidance);
                    output.push('\n');
                }

                let mut response = CallToolResult::success(vec![ContentBlock::text(output)]);
                response.structured_content = Some(serde_json::json!({ "impact": impact_contexts }));
                Ok(response)
            }
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Semantic search failed: {e}"
            ))])),
        }
        }).await.map_err(|error| McpError::internal_error(error.to_string(), None))?
    }

    #[tool(description = "Search for symbols using full-text search with fuzzy matching")]
    pub async fn search_symbols(
        &self,
        Parameters(SearchSymbolsRequest {
            query,
            limit,
            kind,
            module,
            lang,
            path_prefix,
        }): Parameters<SearchSymbolsRequest>,
    ) -> Result<CallToolResult, McpError> {
        crate::mcp::requests::validate_search_limit(limit)?;
        crate::runtime::read(&self.facade, move |indexer| {
            // One kind vocabulary (SymbolKind::from_str); unknown kinds error
            // instead of silently returning unfiltered results.
            let kind_filter = match kind.as_deref().map(str::parse::<crate::SymbolKind>) {
                None => None,
                Some(Ok(k)) => Some(k),
                Some(Err(e)) => {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                        "Error: {e}"
                    ))]));
                }
            };

            match indexer.search_scoped(
                &query,
                limit as usize,
                kind_filter,
                module.as_deref(),
                lang.as_deref(),
                path_prefix.as_deref(),
            ) {
                Ok(results) => {
                    if results.is_empty() {
                        let mut output = format!("No results found for query: {query}");
                        // Add guidance for no results
                        if let Some(guidance) =
                            generate_mcp_guidance(indexer.settings(), "search_symbols", 0)
                        {
                            output.push_str("\n\n---\nGuidance: ");
                            output.push_str(&guidance);
                            output.push('\n');
                        }
                        return Ok(CallToolResult::success(vec![ContentBlock::text(output)]));
                    }

                    let mut result = format!(
                        "Found {} result(s) for query '{}':\n\n",
                        results.len(),
                        query
                    );

                    for (i, search_result) in results.iter().enumerate() {
                        result.push_str(&format!(
                            "{}. {} ({:?})\n",
                            i + 1,
                            search_result.name,
                            search_result.kind
                        ));
                        result.push_str(&format!(
                            "   File: {}:{}\n",
                            search_result.file_path, search_result.line
                        ));

                        if !search_result.module_path.is_empty() {
                            result.push_str(&format!("   Module: {}\n", search_result.module_path));
                        }

                        if let Some(ref doc) = search_result.doc_comment {
                            // Show first line of doc comment
                            let first_line = doc.lines().next().unwrap_or("");
                            result.push_str(&format!("   Doc: {first_line}\n"));
                        }

                        if let Some(ref sig) = search_result.signature {
                            result.push_str(&format!("   Signature: {sig}\n"));
                        }

                        result.push_str(&format!(
                            "   Score: {:.2} (raw lexical candidate score)\n",
                            search_result.score
                        ));
                        if let Some((matched, total)) =
                            crate::storage::tantivy::discovery_term_coverage(&query, search_result)
                        {
                            result.push_str(&format!(
                                "   Distinct query-term coverage: {matched}/{total}\n"
                            ));
                        }
                        result.push('\n');
                    }

                    // Add system guidance
                    if let Some(guidance) =
                        generate_mcp_guidance(indexer.settings(), "search_symbols", results.len())
                    {
                        result.push_str("\n---\nGuidance: ");
                        result.push_str(&guidance);
                        result.push('\n');
                    }

                    Ok(CallToolResult::success(vec![ContentBlock::text(result)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "Search failed: {e}"
                ))])),
            }
        })
        .await
        .map_err(|error| McpError::internal_error(error.to_string(), None))?
    }

    #[tool(
        description = "Search indexed documents (markdown, text files) using natural language queries. Returns relevant chunks with context and highlighted keywords."
    )]
    pub async fn search_documents(
        &self,
        Parameters(SearchDocumentsRequest {
            query,
            collection,
            limit,
        }): Parameters<SearchDocumentsRequest>,
    ) -> Result<CallToolResult, McpError> {
        crate::mcp::requests::validate_search_limit(limit)?;
        let store = match &self.document_store {
            Some(s) => s,
            None => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "Document search not available. No document collections are indexed.\n\n\
                    To enable:\n\
                    1. Add a collection: codanna documents add-collection docs docs/\n\
                    2. Index it: codanna documents index\n\
                    3. Restart the MCP server",
                )]));
            }
        };

        let preview_config = self.facade.read().await.settings().documents.search.clone();
        // Queries never perform auto-index mutations. A document writer owns its
        // mmap state exclusively; fail fast rather than waiting behind a full sync.
        let mut store = store
            .try_read()
            .map_err(|_| McpError::internal_error("Document index is busy; retry the query", None))?
            .query_snapshot();
        crate::runtime::blocking(move || {
            let search_query = DocSearchQuery {
                text: query.clone(),
                collection,
                document: None,
                limit: limit as usize,
                preview_config: Some(preview_config),
            };

            match store.search(search_query) {
                Ok(results) => {
                    if results.is_empty() {
                        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                            "No documents found for: {query}"
                        ))]));
                    }

                    let mut output = format!(
                        "Found {} document(s) matching '{}':\n\n",
                        results.len(),
                        query
                    );

                    for (i, result) in results.iter().enumerate() {
                        output.push_str(&format!(
                            "{}. {} (score: {:.3})\n",
                            i + 1,
                            crate::parsing::paths::render_absolute_path(&result.source_path)
                                .display(),
                            result.similarity
                        ));

                        if !result.heading_context.is_empty() {
                            output.push_str(&format!(
                                "   Context: {}\n",
                                result.heading_context.join(" > ")
                            ));
                        }

                        // Preview is already KWIC-processed with highlighting
                        output.push_str(&format!("   Preview: {}\n\n", result.content_preview));
                    }

                    Ok(CallToolResult::success(vec![ContentBlock::text(output)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "Document search failed: {e}"
                ))])),
            }
        })
        .await
        .map_err(|error| McpError::internal_error(error.to_string(), None))?
    }
}
