//! Broad syntax tier backed by tree-sitter-language-pack.
//!
//! Rich Codanna parsers remain authoritative. This module is a fallback for
//! languages that do not have a registered Codanna parser. It deliberately
//! extracts only high-confidence structure and imports; it does not invent
//! call, inheritance, receiver-type, or implementation edges.

use crate::Settings;
use crate::SymbolKind as CodannaSymbolKind;
use crate::indexing::pipeline::types::{
    ParsedFile, PipelineError, PipelineResult, RawImport, RawSymbol,
};
use crate::parsing::LanguageId;
use crate::types::Range;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tree_sitter_language_pack::{ProcessConfig, StructureItem, StructureKind, SymbolInfo};

const MAX_GENERIC_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const GENERIC_PARSE_TIMEOUT_MS: u64 = 5_000;

/// Number of language names/aliases currently registered by the generic pack.
///
/// This is not the size of the downloadable grammar catalogue: the pack can
/// recognize paths for grammars that have not yet been loaded into its runtime
/// registry.
pub fn language_count() -> usize {
    tree_sitter_language_pack::language_count()
}

/// Sorted language names/aliases currently registered by the pack.
pub fn available_languages() -> Vec<String> {
    tree_sitter_language_pack::available_languages()
}

/// Detect a generic language for a path without loading or downloading a parser.
///
/// A per-language entry in `settings.languages` can disable a generic language.
/// Languages absent from that map are enabled by default; rich registered
/// languages are filtered by the caller before this fallback is reached.
pub fn detect_path(path: &Path, settings: &Settings) -> Option<LanguageId> {
    let path = path.to_str()?;
    let name = tree_sitter_language_pack::detect_language_from_path(path)?;
    if settings
        .languages
        .get(name)
        .is_some_and(|config| !config.enabled)
    {
        return None;
    }
    Some(LanguageId::new(name))
}

/// Ensure the parser for a generic language is available in the persistent
/// language-pack cache. The first miss may download a prebuilt parser; cache
/// hits are local and cheap.
pub fn prewarm(language_id: LanguageId) -> PipelineResult<()> {
    tree_sitter_language_pack::get_language(language_id.as_str())
        .map(|_| ())
        .map_err(|error| PipelineError::ParserConstruction {
            language: language_id,
            reason: format!("generic language parser unavailable: {error}"),
        })
}

/// Parse one file through the broad syntax tier.
pub fn parse(
    path: PathBuf,
    content_hash: String,
    content: &str,
    language_id: LanguageId,
    module_root: Option<&Path>,
) -> PipelineResult<ParsedFile> {
    let mut config = ProcessConfig::new(language_id.as_str()).minimal();
    config.structure = true;
    config.imports = true;
    config.symbols = true;
    config.docstrings = true;
    config.max_source_bytes = Some(MAX_GENERIC_SOURCE_BYTES);
    config.parse_timeout_ms = Some(GENERIC_PARSE_TIMEOUT_MS);

    let result = tree_sitter_language_pack::process(content, &config).map_err(|error| {
        PipelineError::Parse {
            path: path.clone(),
            reason: format!("generic {} parser failed: {error}", language_id.as_str()),
        }
    })?;

    let mut raw_symbols = Vec::new();
    let mut seen = HashSet::new();
    for item in &result.structure {
        collect_structure(item, &mut raw_symbols, &mut seen);
    }
    for symbol in &result.symbols {
        if let Some(raw) = map_symbol(symbol) {
            let key = symbol_key(&raw);
            if seen.insert(key) {
                raw_symbols.push(raw);
            }
        }
    }

    let raw_imports = result
        .imports
        .into_iter()
        .filter(|import| !import.source.trim().is_empty())
        .map(|import| {
            let mut raw = RawImport::new(import.source);
            if let Some(alias) = import.alias.filter(|alias| !alias.is_empty()) {
                raw = raw.with_alias(alias);
            }
            if import.is_wildcard {
                raw = raw.as_glob();
            }
            raw
        })
        .collect();

    let module_path = generic_module_path(&path, module_root);
    Ok(ParsedFile {
        path,
        content_hash,
        language_id,
        module_path,
        raw_symbols,
        raw_imports,
        raw_relationships: Vec::new(),
        variable_bindings: Vec::new(),
        this_barrier_spans: Vec::new(),
    })
}

fn collect_structure(
    item: &StructureItem,
    output: &mut Vec<RawSymbol>,
    seen: &mut HashSet<(String, CodannaSymbolKind, u32, u16)>,
) {
    if let (Some(name), Some(kind)) = (item.name.as_deref(), map_structure_kind(&item.kind)) {
        if !name.trim().is_empty() {
            let mut raw = RawSymbol::new(name, kind, map_span(&item.span));
            if let Some(signature) = item.signature.as_deref().filter(|s| !s.trim().is_empty()) {
                raw = raw.with_signature(signature);
            }
            if let Some(doc) = item.doc_comment.as_deref().filter(|s| !s.trim().is_empty()) {
                raw = raw.with_doc_comment(doc);
            }
            let key = symbol_key(&raw);
            if seen.insert(key) {
                output.push(raw);
            }
        }
    }
    for child in &item.children {
        collect_structure(child, output, seen);
    }
}

fn map_symbol(symbol: &SymbolInfo) -> Option<RawSymbol> {
    if symbol.name.trim().is_empty() {
        return None;
    }
    let kind = match &symbol.kind {
        tree_sitter_language_pack::SymbolKind::Variable => CodannaSymbolKind::Variable,
        tree_sitter_language_pack::SymbolKind::Constant => CodannaSymbolKind::Constant,
        tree_sitter_language_pack::SymbolKind::Function => CodannaSymbolKind::Function,
        tree_sitter_language_pack::SymbolKind::Class => CodannaSymbolKind::Class,
        tree_sitter_language_pack::SymbolKind::Type => CodannaSymbolKind::TypeAlias,
        tree_sitter_language_pack::SymbolKind::Interface => CodannaSymbolKind::Interface,
        tree_sitter_language_pack::SymbolKind::Enum => CodannaSymbolKind::Enum,
        tree_sitter_language_pack::SymbolKind::Module => CodannaSymbolKind::Module,
        tree_sitter_language_pack::SymbolKind::Other(_) => return None,
    };
    Some(RawSymbol::new(
        symbol.name.as_str(),
        kind,
        map_span(&symbol.span),
    ))
}

fn map_structure_kind(kind: &StructureKind) -> Option<CodannaSymbolKind> {
    match kind {
        StructureKind::Function => Some(CodannaSymbolKind::Function),
        StructureKind::Method => Some(CodannaSymbolKind::Method),
        StructureKind::Class => Some(CodannaSymbolKind::Class),
        StructureKind::Struct => Some(CodannaSymbolKind::Struct),
        StructureKind::Interface => Some(CodannaSymbolKind::Interface),
        StructureKind::Enum => Some(CodannaSymbolKind::Enum),
        StructureKind::Module | StructureKind::Namespace => Some(CodannaSymbolKind::Module),
        StructureKind::Trait => Some(CodannaSymbolKind::Trait),
        StructureKind::Impl | StructureKind::Other(_) => None,
    }
}

fn map_span(span: &tree_sitter_language_pack::Span) -> Range {
    Range::new(
        saturating_u32(span.start_line),
        saturating_u16(span.start_column),
        saturating_u32(span.end_line),
        saturating_u16(span.end_column),
    )
}

fn saturating_u32(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

fn saturating_u16(value: usize) -> u16 {
    value.min(u16::MAX as usize) as u16
}

fn symbol_key(raw: &RawSymbol) -> (String, CodannaSymbolKind, u32, u16) {
    (
        raw.name.to_string(),
        raw.kind,
        raw.range.start_line,
        raw.range.start_column,
    )
}

fn generic_module_path(path: &Path, module_root: Option<&Path>) -> Option<String> {
    let relative = module_root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let mut without_extension = relative.to_path_buf();
    without_extension.set_extension("");
    let module = without_extension
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    (!module.is_empty()).then_some(module)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_broad_generic_catalogue_without_loading_parsers() {
        let settings = Settings::default();
        for (path, expected) in [
            ("lib/app.ex", "elixir"),
            ("src/main.zig", "zig"),
            ("lib/main.dart", "dart"),
            ("src/Main.scala", "scala"),
            ("infra/main.tf", "terraform"),
        ] {
            assert_eq!(
                detect_path(Path::new(path), &settings).map(|id| id.as_str()),
                Some(expected),
                "unexpected generic language detection for {path}"
            );
        }
    }

    #[test]
    fn generic_language_can_be_disabled_through_existing_language_settings() {
        use crate::config::LanguageConfig;
        let mut settings = Settings::default();
        settings.languages.insert(
            "zig".into(),
            LanguageConfig {
                enabled: false,
                extensions: vec!["zig".into()],
                parser_options: Default::default(),
                config_files: Vec::new(),
                projects: Vec::new(),
            },
        );
        assert!(detect_path(Path::new("main.zig"), &settings).is_none());
    }
}
