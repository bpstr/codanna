//! Deterministic source inputs for code embeddings. No model-generated summaries.
//!
//! Byte limits below bound retained source, not billed tokens. The actual backend
//! input validator partitions each retained fragment before inference. Parser
//! ranges are evidence, not a claim of complete implementation extraction.

use crate::embedding_input::InputBudget;
use crate::indexing::pipeline::types::ParsedFile;
use crate::{Range, ScopeContext, SymbolKind};
use serde::{Deserialize, Serialize};
use std::ops::Range as ByteRange;
use std::path::Path;

pub const MAX_SYMBOL_SOURCE_BYTES: usize = 16 * 1024;
pub const MAX_FILE_SOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_SYMBOL_SEGMENTS: usize = 8;
pub const SOURCE_POLICY_ID: &str =
    "symbol-body-v1:source=16384:file=1048576:segments=8:fields=v1:selection=head-tail";

pub const SOURCE_POLICY_V2_ID: &str =
    "symbol-body-v2:source=16384:file=1048576:segments=8:fields=v2:selection=head-middle-tail";

/// Source policy is independent of the model's complete-input/tokenizer policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeEmbeddingPolicy {
    #[default]
    DocComment,
    SymbolBodyV1,
    SymbolBodyV2,
}

impl CodeEmbeddingPolicy {
    pub fn eligible(self, kind: SymbolKind, has_doc: bool) -> bool {
        match self {
            Self::DocComment => has_doc,
            Self::SymbolBodyV1 | Self::SymbolBodyV2 => matches!(
                kind,
                SymbolKind::Function
                    | SymbolKind::Method
                    | SymbolKind::Struct
                    | SymbolKind::Enum
                    | SymbolKind::Trait
                    | SymbolKind::Interface
                    | SymbolKind::Class
                    | SymbolKind::TypeAlias
            ),
        }
    }

    pub fn source_policy(self) -> &'static str {
        match self {
            Self::DocComment => "doc_comment_present_v1",
            Self::SymbolBodyV1 => SOURCE_POLICY_ID,
            Self::SymbolBodyV2 => SOURCE_POLICY_V2_ID,
        }
    }

    /// Check the source contract without a model or provider initialization.
    /// Other backend fields still require the existing full identity check.
    pub(crate) fn accepts_recorded_identity(self, identity: Option<&str>) -> bool {
        let recorded =
            identity.and_then(|identity| serde_json::from_str::<serde_json::Value>(identity).ok());
        let source = recorded
            .as_ref()
            .and_then(|value| value.get("source_input_policy"))
            .and_then(serde_json::Value::as_str);
        match self {
            Self::DocComment => source.is_none() || source == Some("doc_comment_present_v1"),
            Self::SymbolBodyV1 => source == Some(SOURCE_POLICY_ID),
            Self::SymbolBodyV2 => source == Some(SOURCE_POLICY_V2_ID),
        }
    }

    /// Legacy identities remain byte-for-byte unchanged. Body-policy vectors
    /// cannot validate against legacy identities, even with the same dimension.
    pub(crate) fn bind_identity(self, backend: String) -> String {
        match self {
            Self::DocComment => backend,
            Self::SymbolBodyV1 | Self::SymbolBodyV2 => {
                let mut identity: serde_json::Value = serde_json::from_str(&backend)
                    .unwrap_or_else(|_| serde_json::json!({"backend_identity": backend}));
                if !identity.is_object() {
                    identity = serde_json::json!({"backend_identity": backend});
                }
                identity["source_input_policy"] = self.source_policy().into();
                identity.to_string()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceFragment {
    pub range: ByteRange<usize>,
    pub text: String,
}

/// Captured while the parser still owns the exact bytes it indexed.
#[derive(Debug, Clone)]
pub struct SymbolSource {
    pub header: String,
    pub fragments: Vec<SourceFragment>,
    pub policy: CodeEmbeddingPolicy,
}

#[derive(Debug)]
pub(crate) struct SymbolInput {
    pub text: String,
    pub source_range: Option<ByteRange<usize>>,
}

impl SymbolSource {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.header.len() + self.fragments.iter().map(|f| f.text.len()).sum::<usize>()
    }

    pub(crate) fn inputs(&self, budget: &InputBudget) -> Result<Vec<SymbolInput>, String> {
        if self.fragments.is_empty() {
            budget.validate([self.header.as_str()])?;
            return Ok(vec![SymbolInput {
                text: self.header.clone(),
                source_range: None,
            }]);
        }
        let mut ranges = Vec::new();
        for (fragment_index, fragment) in self.fragments.iter().enumerate() {
            for range in budget.document_ranges(&self.header, &fragment.text)? {
                ranges.push((fragment_index, range));
            }
        }
        let selected = if self.policy == CodeEmbeddingPolicy::SymbolBodyV2 {
            let midpoint = (self.fragments.first().map_or(0, |f| f.range.start)
                + self.fragments.last().map_or(0, |f| f.range.end))
                / 2;
            let center = ranges.iter().position(|(fragment, range)| {
                let start = self.fragments[*fragment].range.start + range.start;
                let end = self.fragments[*fragment].range.start + range.end;
                start <= midpoint && midpoint < end
            });
            let mut selected = std::collections::BTreeSet::new();
            for index in center
                .into_iter()
                .chain((!ranges.is_empty()).then_some(0))
                .chain(ranges.len().checked_sub(1))
                .chain(distributed_indices(ranges.len(), MAX_SYMBOL_SEGMENTS))
            {
                selected.insert(index);
                if selected.len() == MAX_SYMBOL_SEGMENTS {
                    break;
                }
            }
            selected.into_iter().collect()
        } else {
            selected_indices(ranges.len(), MAX_SYMBOL_SEGMENTS)
        };
        let mut result = Vec::with_capacity(selected.len());
        for index in selected {
            let (fragment_index, range) = &ranges[index];
            let fragment = &self.fragments[*fragment_index];
            let text = format!("{}{}", self.header, &fragment.text[range.clone()]);
            budget.validate([text.as_str()])?;
            result.push(SymbolInput {
                text,
                source_range: Some(
                    fragment.range.start + range.start..fragment.range.start + range.end,
                ),
            });
        }
        Ok(result)
    }
}

fn selected_indices(len: usize, limit: usize) -> Vec<usize> {
    if len <= limit {
        return (0..len).collect();
    }
    let head = limit.div_ceil(2);
    (0..head).chain(len - (limit - head)..len).collect()
}

fn distributed_indices(len: usize, limit: usize) -> Vec<usize> {
    if len <= limit {
        return (0..len).collect();
    }
    if limit == 0 {
        return vec![];
    }
    let mut chosen = std::collections::BTreeSet::new();
    for index in [0, len - 1, len / 2]
        .into_iter()
        .chain((0..limit).map(|i| i * (len - 1) / limit.saturating_sub(1).max(1)))
        .chain(0..len)
    {
        chosen.insert(index);
        if chosen.len() == limit {
            break;
        }
    }
    chosen.into_iter().collect()
}

fn middle_fragments(
    source: &str,
    range: ByteRange<usize>,
    allowance: usize,
) -> Vec<SourceFragment> {
    let limit = allowance.min(MAX_SYMBOL_SOURCE_BYTES);
    if range.len() <= limit || limit < 3 {
        return fragments(source, range, allowance);
    }
    let width = limit / 3;
    let starts = [
        range.start,
        range.start + (range.len() - width) / 2,
        range.end - width,
    ];
    starts
        .into_iter()
        .filter_map(|mut start| {
            let mut end = (start + width).min(range.end);
            while start < end && !source.is_char_boundary(start) {
                start += 1;
            }
            while end > start && !source.is_char_boundary(end) {
                end -= 1;
            }
            (start < end).then(|| SourceFragment {
                range: start..end,
                text: source[start..end].to_owned(),
            })
        })
        .collect()
}

fn prefix(value: &str, max: usize) -> &str {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn field(header: &mut String, name: &str, value: Option<&str>, max: usize) {
    if let Some(value) = value {
        header.push_str(name);
        header.push_str(": ");
        header.push_str(prefix(value, max));
        if value.len() > max {
            header.push_str(" [bounded]");
        }
        header.push('\n');
    }
}

fn source_range(source: &str, lines: &[usize], range: Range) -> Option<ByteRange<usize>> {
    let offset = |line: u32, column: u32| {
        let line = line as usize;
        let start = *lines.get(line)?;
        let end = lines.get(line + 1).copied().unwrap_or(source.len());
        let content = source.get(start..end)?.trim_end_matches(['\n', '\r']);
        let column = column as usize;
        (column <= content.len() && content.is_char_boundary(column)).then_some(start + column)
    };
    let start = offset(range.start_line, range.start_column)?;
    let end = offset(range.end_line, range.end_column)?;
    (start < end).then_some(start..end)
}

fn fragments(source: &str, range: ByteRange<usize>, allowance: usize) -> Vec<SourceFragment> {
    let body = &source[range.clone()];
    let allowance = allowance.min(MAX_SYMBOL_SOURCE_BYTES);
    if allowance == 0 {
        return Vec::new();
    }
    if body.len() <= allowance {
        return vec![SourceFragment {
            range,
            text: body.to_owned(),
        }];
    }
    let head = prefix(body, allowance / 2).len();
    let mut tail = body.len().saturating_sub(allowance - head);
    while !body.is_char_boundary(tail) {
        tail += 1;
    }
    let mut result = Vec::new();
    if head > 0 {
        result.push(SourceFragment {
            range: range.start..range.start + head,
            text: body[..head].to_owned(),
        });
    }
    if tail < body.len() {
        result.push(SourceFragment {
            range: range.start + tail..range.end,
            text: body[tail..].to_owned(),
        });
    }
    result
}

#[cfg(test)]
pub(crate) fn capture(parsed: &mut ParsedFile, source: &str, root: Option<&Path>) {
    capture_with_policy(parsed, source, root, CodeEmbeddingPolicy::SymbolBodyV1);
}

pub(crate) fn capture_with_policy(
    parsed: &mut ParsedFile,
    source: &str,
    root: Option<&Path>,
    policy: CodeEmbeddingPolicy,
) {
    let mut lines = vec![0];
    lines.extend(source.match_indices('\n').map(|(offset, _)| offset + 1));
    let path = root.and_then(|root| parsed.path.strip_prefix(root).ok());
    // Never expose machine-specific absolute roots in embedding inputs.
    let path = path.unwrap_or_else(|| {
        if parsed.path.is_absolute() {
            Path::new("<outside-workspace>")
        } else {
            &parsed.path
        }
    });
    let path = path.to_string_lossy().replace('\\', "/");
    let mut remaining = MAX_FILE_SOURCE_BYTES;
    for symbol in &mut parsed.raw_symbols {
        symbol.code_embedding_policy = policy;
        if !policy.eligible(symbol.kind, symbol.doc_comment.is_some()) {
            continue;
        }
        let mut header =
            String::from("Code symbol representation v1\nRepository: current workspace\n");
        field(&mut header, "Path", Some(&path), 192);
        field(&mut header, "Module", parsed.module_path.as_deref(), 128);
        let owner = match symbol.scope_context.as_ref() {
            Some(ScopeContext::ClassMember { class_name }) => class_name.as_deref(),
            Some(ScopeContext::Local { parent_name, .. }) => parent_name.as_deref(),
            _ => None,
        };
        field(&mut header, "Owner", owner, 128);
        field(&mut header, "Name", Some(&symbol.name), 128);
        field(&mut header, "Kind", Some(&format!("{:?}", symbol.kind)), 32);
        field(&mut header, "Signature", symbol.signature.as_deref(), 256);
        field(
            &mut header,
            "Documentation",
            symbol.doc_comment.as_deref(),
            256,
        );
        if policy == CodeEmbeddingPolicy::SymbolBodyV2 {
            header = header.replacen("representation v1", "representation v2", 1);
            field(
                &mut header,
                "Context basis",
                Some("observed parser identity; no inferred behavior"),
                128,
            );
            field(
                &mut header,
                "Language",
                Some(parsed.language_id.as_str()),
                64,
            );
            field(
                &mut header,
                "Declaration role",
                Some(&format!("{:?}", symbol.kind)),
                32,
            );
        }
        // Reserve the fixed coverage line too. Exhaustion means a missing eligible
        // representation, never fallback to a mislabeled comment vector.
        let overhead = header.len() + 128;
        if overhead > remaining {
            continue;
        }
        remaining -= overhead;
        let range = source_range(source, &lines, symbol.range);
        let fragments = range.map_or_else(Vec::new, |range| {
            if policy == CodeEmbeddingPolicy::SymbolBodyV2 {
                middle_fragments(source, range, remaining)
            } else {
                fragments(source, range, remaining)
            }
        });
        let bytes = fragments.iter().map(|f| f.text.len()).sum::<usize>();
        remaining = remaining.saturating_sub(bytes);
        header.push_str(if fragments.is_empty() {
            "Source coverage: unavailable or file source budget exhausted\n"
        } else {
            "Source coverage: bounded parser-range excerpts; extraction completeness unverified\nImplementation excerpt:\n"
        });
        symbol.embedding_source = Some(SymbolSource {
            header,
            fragments,
            policy,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexing::pipeline::types::RawSymbol;
    use crate::parsing::LanguageId;

    fn parsed(source: &str) -> ParsedFile {
        let mut parsed = ParsedFile::new(
            "src/api.rs".into(),
            "fixture".into(),
            LanguageId::new("rust"),
        );
        parsed.raw_symbols.push(RawSymbol::new(
            "serve",
            SymbolKind::Function,
            Range::new(0, 0, 0, source.len() as u32),
        ));
        parsed
    }

    #[test]
    fn evidence_v1_body_v2_keeps_middle_in_final_embedding_inputs() {
        let source = format!(
            "fn serve() {{ head_marker(); {} middle_marker(); {} tail_marker(); }}",
            "a();".repeat(6000),
            "z();".repeat(6000)
        );
        let mut parsed = parsed(&source);
        capture_with_policy(
            &mut parsed,
            &source,
            None,
            CodeEmbeddingPolicy::SymbolBodyV2,
        );
        let representation = parsed.raw_symbols[0].embedding_source.as_ref().unwrap();
        let inputs = representation
            .inputs(&InputBudget::remote(Some(1024), None).unwrap())
            .unwrap();
        assert!(
            inputs
                .iter()
                .any(|input| input.text.contains("middle_marker"))
        );
        assert!(inputs.len() <= MAX_SYMBOL_SEGMENTS);
        assert!(inputs.iter().all(|input| {
            input
                .text
                .contains("Context basis: observed parser identity")
        }));
    }

    #[test]
    fn symbol_representation_includes_undocumented_body_literals_and_identity() {
        let source = "fn serve() { emit(\"calendar.changed\"); route(\"/api/calendar\"); }";
        let mut parsed = parsed(source);
        capture(&mut parsed, source, None);
        let representation = parsed.raw_symbols[0].embedding_source.as_ref().unwrap();
        let inputs = representation
            .inputs(&InputBudget::remote(Some(2048), None).unwrap())
            .unwrap();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].text.contains(source));
        assert!(inputs[0].text.contains("Path: src/api.rs"));
        assert!(inputs[0].text.contains("Name: serve"));
        assert_eq!(inputs[0].source_range, Some(0..source.len()));
        assert!(parsed.raw_symbols[0].doc_comment.is_none());
    }

    #[test]
    fn symbol_representation_policy_does_not_select_every_local() {
        for kind in [
            SymbolKind::Variable,
            SymbolKind::Constant,
            SymbolKind::Parameter,
            SymbolKind::Field,
            SymbolKind::Module,
        ] {
            assert!(!CodeEmbeddingPolicy::SymbolBodyV1.eligible(kind, true));
            assert!(CodeEmbeddingPolicy::DocComment.eligible(kind, true));
        }
        assert!(CodeEmbeddingPolicy::SymbolBodyV1.eligible(SymbolKind::Method, false));
        assert!(!CodeEmbeddingPolicy::DocComment.eligible(SymbolKind::Method, false));
        assert_eq!(
            CodeEmbeddingPolicy::DocComment.bind_identity("legacy".into()),
            "legacy"
        );
        assert_ne!(
            CodeEmbeddingPolicy::SymbolBodyV1.bind_identity("legacy".into()),
            "legacy"
        );
    }

    #[test]
    fn symbol_representation_segments_are_utf8_safe_bounded_and_keep_tail() {
        let source = format!(
            "fn serve() {{ {} route(\"/tail-marker\"); }}",
            "árvíz界 ".repeat(5000)
        );
        let mut parsed = parsed(&source);
        capture(&mut parsed, &source, None);
        let representation = parsed.raw_symbols[0].embedding_source.as_ref().unwrap();
        assert!(
            representation
                .fragments
                .iter()
                .map(|f| f.text.len())
                .sum::<usize>()
                <= MAX_SYMBOL_SOURCE_BYTES
        );
        let budget = InputBudget::remote(Some(1024), None).unwrap();
        let first = representation.inputs(&budget).unwrap();
        let second = representation.inputs(&budget).unwrap();
        assert_eq!(first.len(), MAX_SYMBOL_SEGMENTS);
        assert!(first.last().unwrap().text.contains("/tail-marker"));
        assert_eq!(
            first.iter().map(|i| &i.text).collect::<Vec<_>>(),
            second.iter().map(|i| &i.text).collect::<Vec<_>>()
        );
        for input in first {
            budget.validate([input.text.as_str()]).unwrap();
            let range = input.source_range.unwrap();
            assert!(input.text.ends_with(&source[range]));
        }
    }

    #[test]
    fn symbol_representation_invalid_range_never_borrows_neighbor_source() {
        let source = "fn serve() {}";
        let mut parsed = parsed(source);
        parsed.raw_symbols[0].range = Range::new(9, 0, 10, 0);
        capture(&mut parsed, source, None);
        let representation = parsed.raw_symbols[0].embedding_source.as_ref().unwrap();
        assert!(representation.fragments.is_empty());
        assert!(
            representation
                .header
                .contains("Source coverage: unavailable")
        );
    }

    #[test]
    fn symbol_representation_body_path_and_signature_edits_change_inputs() {
        let render = |source: &str, path: &str, signature: &str| {
            let mut parsed = parsed(source);
            parsed.path = path.into();
            parsed.raw_symbols[0].signature = Some(signature.into());
            capture(&mut parsed, source, None);
            parsed.raw_symbols[0]
                .embedding_source
                .as_ref()
                .unwrap()
                .inputs(&InputBudget::remote(Some(4096), None).unwrap())
                .unwrap()
                .remove(0)
                .text
        };
        let base = render("fn serve() { old(); }", "src/a.rs", "fn serve()");
        assert_ne!(
            base,
            render("fn serve() { new(); }", "src/a.rs", "fn serve()")
        );
        assert_ne!(
            base,
            render("fn serve() { old(); }", "src/b.rs", "fn serve()")
        );
        assert_ne!(
            base,
            render("fn serve() { old(); }", "src/a.rs", "fn serve(x: bool)")
        );
    }

    #[test]
    fn symbol_representation_source_columns_are_utf8_bytes() {
        let source = "界; fn serve() {}\nother();";
        let lines = vec![0, source.find('\n').unwrap() + 1];
        assert_eq!(
            &source[source_range(source, &lines, Range::new(0, 5, 0, 18)).unwrap()],
            "fn serve() {}"
        );
        assert!(source_range(source, &lines, Range::new(0, 1, 0, 18)).is_none());
        assert!(source_range(source, &lines, Range::new(0, 0, 0, 80)).is_none());
    }
}

#[cfg(test)]
mod pipeline_contracts {
    use super::*;
    use crate::indexing::pipeline::stages::{CollectStage, init_parser_cache, parse_file};
    use crate::indexing::pipeline::types::FileContent;
    use std::sync::Arc;

    #[test]
    fn symbol_representation_parse_collect_preserves_snapshot_and_legacy_contract() {
        for policy in [
            CodeEmbeddingPolicy::DocComment,
            CodeEmbeddingPolicy::SymbolBodyV1,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("source.rs");
            let source = "/// API documentation\nfn documented() { emit(\"old.route\"); }\nfn undocumented() { emit(\"calendar.changed\"); }\n";
            let mut settings = crate::Settings {
                workspace_root: Some(temp.path().to_path_buf()),
                index_path: temp.path().join("index"),
                ..Default::default()
            };
            settings.semantic_search.code_representation = policy;
            settings.semantic_search.enabled = true;
            let settings = Arc::new(settings);
            init_parser_cache(Arc::clone(&settings));
            let parsed = parse_file(
                FileContent::new(path.clone(), source.into(), "fixture".into()),
                &settings,
            )
            .unwrap();
            // A later filesystem version must not enter representations of this parse.
            std::fs::write(&path, "fn unrelated_neighbor() { changed(); }").unwrap();
            let index = Arc::new(
                crate::storage::DocumentIndex::new(&settings.index_path, &settings).unwrap(),
            );
            let (_, _, batch) = CollectStage::new(1).process_single(parsed, index).unwrap();
            match policy {
                CodeEmbeddingPolicy::DocComment => {
                    assert_eq!(batch.candidates.len(), 1);
                    assert!(batch.candidates[0].1.contains("API documentation"));
                    assert!(!batch.candidates[0].1.contains("old.route"));
                    assert!(batch.body_candidates.is_empty());
                }
                CodeEmbeddingPolicy::SymbolBodyV1 | CodeEmbeddingPolicy::SymbolBodyV2 => {
                    assert!(batch.candidates.is_empty());
                    assert_eq!(batch.body_candidates.len(), 2);
                    let inputs: Vec<_> = batch
                        .body_candidates
                        .iter()
                        .flat_map(|(_, source, _)| {
                            source
                                .inputs(&InputBudget::remote(Some(2048), None).unwrap())
                                .unwrap()
                        })
                        .collect();
                    assert!(
                        inputs
                            .iter()
                            .any(|input| input.text.contains("calendar.changed"))
                    );
                    assert!(
                        inputs
                            .iter()
                            .all(|input| !input.text.contains("unrelated_neighbor"))
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod evidence_v1_tests {
    use super::*;
    #[test]
    fn evidence_v1_middle_source_is_retained_without_growing_budget() {
        let source = format!(
            "{}timezone_sensitive_middle{}",
            "a".repeat(20_000),
            "z".repeat(20_000)
        );
        let old = fragments(&source, 0..source.len(), MAX_SYMBOL_SOURCE_BYTES);
        let new = middle_fragments(&source, 0..source.len(), MAX_SYMBOL_SOURCE_BYTES);
        assert!(
            !old.iter()
                .any(|f| f.text.contains("timezone_sensitive_middle"))
        );
        assert!(
            new.iter()
                .any(|f| f.text.contains("timezone_sensitive_middle"))
        );
        assert!(new.iter().map(|f| f.text.len()).sum::<usize>() <= MAX_SYMBOL_SOURCE_BYTES);
        for fragment in new {
            assert_eq!(fragment.text, source[fragment.range]);
        }
        assert!(distributed_indices(100, 8).contains(&50));
        assert!(!selected_indices(100, 8).contains(&50));
        let identity = CodeEmbeddingPolicy::SymbolBodyV2.bind_identity("{}".into());
        assert!(!CodeEmbeddingPolicy::SymbolBodyV1.accepts_recorded_identity(Some(&identity)));
        assert!(CodeEmbeddingPolicy::SymbolBodyV2.accepts_recorded_identity(Some(&identity)));
    }
    #[test]
    fn evidence_v1_middle_source_preserves_unicode_ranges() {
        let source = "árvíztűrő 🦀".repeat(4000);
        for fragment in middle_fragments(&source, 0..source.len(), 16384) {
            assert_eq!(fragment.text, source[fragment.range]);
        }
    }
}
