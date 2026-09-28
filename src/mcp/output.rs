//! Model-facing projections and whole-result byte budgets.
//!
//! A result is packed as complete semantic rows, never sliced JSON/text. The
//! measured ceiling includes BOTH MCP text and structuredContent, including
//! JSON escaping. It does not include the transport's JSON-RPC envelope or a
//! host's sibling tool responses. Bytes are not advertised as tokenizer tokens.
use rmcp::model::{CallToolResult, ContentBlock, ErrorData};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

pub const DEFAULT_OUTPUT_BYTES: usize = 8192;
pub const MIN_OUTPUT_BYTES: usize = 1024;
pub const MAX_OUTPUT_BYTES: usize = 65536;

#[derive(
    Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OutputView {
    #[default]
    Compact,
    Detail,
}

#[derive(Debug, Clone, Copy, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct OutputBudget(#[schemars(range(min = 1024, max = 65536))] usize);
impl Default for OutputBudget {
    fn default() -> Self {
        Self(DEFAULT_OUTPUT_BYTES)
    }
}
impl OutputBudget {
    pub fn new(bytes: usize) -> Result<Self, &'static str> {
        if (MIN_OUTPUT_BYTES..=MAX_OUTPUT_BYTES).contains(&bytes) {
            Ok(Self(bytes))
        } else {
            Err("max_output_bytes must be an integer in 1024..=65536")
        }
    }
    pub fn bytes(self) -> usize {
        self.0
    }
}
impl<'de> Deserialize<'de> for OutputBudget {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let bytes = usize::deserialize(d)?;
        Self::new(bytes).map_err(serde::de::Error::custom)
    }
}

/// An explicitly lossy preview, never an identifier or source-code body.
pub(crate) fn preview(text: &str, chars: usize) -> (String, bool) {
    let end = text
        .char_indices()
        .nth(chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    (text[..end].to_owned(), end < text.len())
}

pub(crate) fn display_path(path: &Path, workspace: Option<&Path>) -> String {
    workspace
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Normalize a caller's relative document selector against the selected workspace.
pub(crate) fn document_scope(
    value: Option<&str>,
    workspace: Option<&Path>,
) -> Result<Option<std::path::PathBuf>, ErrorData> {
    let Some(value) = value else {
        return Ok(None);
    };
    let path = value.replace('\\', "/");
    if path.is_empty()
        || path.len() > 4096
        || path.chars().any(char::is_control)
        || path.starts_with('/')
        || path.as_bytes().get(1) == Some(&b':')
        || path.split('/').any(|part| part == "..")
    {
        return Err(ErrorData::invalid_params(
            "document must be a nonescaping workspace-relative path",
            None,
        ));
    }
    let root = workspace.ok_or_else(|| {
        ErrorData::invalid_params(
            "An explicit workspace root is required for document filtering",
            None,
        )
    })?;
    let selected = root.join(path);
    crate::indexing::facade::IndexFacade::contained_source(root, &selected)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    Ok(Some(selected))
}

/// JSON-pointer path to one ordered list. Secondary lists are considered only
/// after every primary row fits. Within a priority, sources receive turns.
pub(crate) struct Section {
    path: &'static str,
    secondary: bool,
    page_offset: Option<usize>,
}
impl Section {
    pub(crate) fn primary(path: &'static str) -> Self {
        Self {
            path,
            secondary: false,
            page_offset: None,
        }
    }
    pub(crate) fn secondary(path: &'static str) -> Self {
        Self {
            path,
            secondary: true,
            page_offset: None,
        }
    }
    pub(crate) fn page(path: &'static str, offset: usize) -> Self {
        Self {
            path,
            secondary: false,
            page_offset: Some(offset),
        }
    }
}
struct Pending {
    section: Section,
    rows: Vec<Value>,
    shown: usize,
    blocked: bool,
    original_next: Option<Value>,
}

fn encode(data: &Value, pending: &[Pending]) -> Result<CallToolResult, ErrorData> {
    let mut text = String::new();
    // Essential status first, then every primary list, then supplemental lists.
    // Removing rendered fields from a clone prevents duplicate evidence in text.
    let mut remaining = data.clone();
    for key in [
        "output",
        "schema_version",
        "source_freshness",
        "cross_source_snapshot",
        "trust",
    ] {
        if let Some(value) = remaining.as_object_mut().and_then(|map| map.remove(key)) {
            render_value(&mut text, key, &value, 0);
        }
    }
    for secondary in [false, true] {
        for list in pending
            .iter()
            .filter(|list| list.section.secondary == secondary)
        {
            let path = list.section.path;
            if let Some((parent, key)) = path.rsplit_once('/') {
                if let Some(value) = remaining
                    .pointer_mut(parent)
                    .and_then(Value::as_object_mut)
                    .and_then(|map| map.remove(key))
                {
                    render_value(&mut text, &path[1..].replace('/', "."), &value, 0);
                }
            }
        }
    }
    if let Some(map) = remaining.as_object() {
        for (key, value) in map {
            render_value(&mut text, key, value, 0);
        }
    }
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(data.clone());
    Ok(result)
}

fn render_value(out: &mut String, key: &str, value: &Value, depth: usize) {
    let indent = "  ".repeat(depth);
    match value {
        Value::Object(map) => {
            out.push_str(&format!("{indent}{key}:\n"));
            for (key, value) in map {
                render_value(out, key, value, depth + 1);
            }
        }
        Value::Array(rows) if !rows.is_empty() && rows.iter().all(Value::is_object) => {
            let columns: BTreeSet<&str> = rows
                .iter()
                .filter_map(Value::as_object)
                .flat_map(|row| row.keys().map(String::as_str))
                .collect();
            out.push_str(&format!(
                "{indent}{key}[{}]{{{}}}:\n",
                rows.len(),
                columns.iter().copied().collect::<Vec<_>>().join(",")
            ));
            for row in rows {
                out.push_str(&indent);
                out.push_str("  ");
                for (i, column) in columns.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    out.push_str(&row.get(*column).unwrap_or(&Value::Null).to_string());
                }
                out.push('\n');
            }
        }
        Value::String(text) if text.contains('\n') => {
            // No reflow: source remains copyable; the structured field is exact.
            let marker = if text.ends_with("\n\n") {
                "|+"
            } else if text.ends_with('\n') {
                "|"
            } else {
                "|-"
            };
            out.push_str(&format!("{indent}{key}: {marker}\n"));
            for line in text.split_inclusive('\n') {
                out.push_str(&indent);
                out.push_str("  ");
                out.push_str(line);
                if !line.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        _ => out.push_str(&format!("{indent}{key}: {value}\n")),
    }
}

fn measured(result: &CallToolResult) -> Result<usize, ErrorData> {
    serde_json::to_vec(result)
        .map(|bytes| bytes.len())
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))
}

fn metadata(data: &mut Value, pending: &[Pending], budget: OutputBudget) {
    let mut counts = serde_json::Map::new();
    let mut partial = false;
    for list in pending {
        let omitted = list.rows.len() - list.shown;
        partial |= omitted > 0;
        counts.insert(
            list.section.path.to_owned(),
            json!({"returned": list.shown, "omitted": omitted}),
        );
        let retrieval_path = match list.section.path {
            "/results" => Some("/retrieval"),
            "/documents/items" => Some("/documents/retrieval"),
            _ => None,
        };
        if let Some(retrieval) = retrieval_path
            .and_then(|path| data.pointer_mut(path))
            .and_then(Value::as_object_mut)
        {
            for (returned_key, retrieved_key) in [
                ("returned_chunks", "retrieved_chunks"),
                ("returned_symbols", "retrieved_symbols"),
            ] {
                if let Some(original) = retrieval.get(returned_key).cloned() {
                    retrieval
                        .entry(retrieved_key.to_owned())
                        .or_insert(original);
                    retrieval.insert(returned_key.to_owned(), json!(list.shown));
                }
            }
        }

        if let Some(offset) = list.section.page_offset {
            let parent = list
                .section
                .path
                .rsplit_once('/')
                .expect("static pointer")
                .0;
            let next = if omitted > 0 {
                if list.shown > 0 {
                    json!(offset + list.shown)
                } else {
                    Value::Null
                }
            } else {
                list.original_next.clone().unwrap_or(Value::Null)
            };
            data.pointer_mut(parent).expect("static section parent")["next_offset"] = next;
        }
    }
    data["output"] = json!({
        "max_output_bytes": budget.bytes(),
        "partial": partial,
        "lists": counts,
        "scope": "retrieved_candidates_not_corpus_coverage",
        "recovery": if partial { "Narrow the query, select an ID, or increase max_output_bytes. Page only with an advancing next_offset." } else { "none" }
    });
}

/// Keep each source's ranked prefix; never skip an oversized first hit and
/// pretend a later one was the best match. No non-advancing cursors are emitted.
pub(crate) fn bounded(
    mut data: Value,
    sections: Vec<Section>,
    budget: OutputBudget,
) -> Result<CallToolResult, ErrorData> {
    let mut pending = Vec::new();
    for section in sections {
        let original_next = section
            .path
            .rsplit_once('/')
            .and_then(|(parent, _)| data.pointer(parent))
            .and_then(|parent| parent.get("next_offset"))
            .cloned();
        let Some(slot) = data.pointer_mut(section.path) else {
            continue;
        };
        let Some(rows) = slot.as_array_mut() else {
            continue;
        };
        pending.push(Pending {
            section,
            rows: std::mem::take(rows),
            shown: 0,
            blocked: false,
            original_next,
        });
    }
    metadata(&mut data, &pending, budget);
    let initial = encode(&data, &pending)?;
    if measured(&initial)? > budget.bytes() {
        return Ok(too_small(
            budget,
            "Response metadata does not fit; narrow the request or increase max_output_bytes.",
        ));
    }
    for secondary in [false, true] {
        if secondary
            && pending
                .iter()
                .any(|list| !list.section.secondary && list.shown < list.rows.len())
        {
            break;
        }
        loop {
            let mut advanced = false;
            for i in 0..pending.len() {
                let list = &pending[i];
                if list.section.secondary != secondary
                    || list.blocked
                    || list.shown == list.rows.len()
                {
                    continue;
                }
                let path = list.section.path;
                let candidate = list.rows[list.shown].clone();
                data.pointer_mut(path)
                    .and_then(Value::as_array_mut)
                    .expect("static list")
                    .push(candidate);
                pending[i].shown += 1;
                metadata(&mut data, &pending, budget);
                if measured(&encode(&data, &pending)?)? <= budget.bytes() {
                    advanced = true;
                } else {
                    data.pointer_mut(path)
                        .and_then(Value::as_array_mut)
                        .expect("static list")
                        .pop();
                    pending[i].shown -= 1;
                    pending[i].blocked = true;
                    metadata(&mut data, &pending, budget);
                }
            }
            if !advanced {
                break;
            }
        }
    }
    let result = encode(&data, &pending)?;
    // Re-measure the final omission/page metadata too, not just row payloads.
    if measured(&result)? > budget.bytes() {
        return Ok(too_small(
            budget,
            "Output metadata exceeds the budget; narrow the request.",
        ));
    }
    Ok(result)
}

/// Error diagnostics are previews; unlike source/identity rows they may shrink.
pub(crate) fn failure(message: &str, budget: OutputBudget) -> CallToolResult {
    let mut chars = message.chars().count().min(512);
    loop {
        let (message, truncated) = preview(message, chars);
        let data = json!({"status":"error", "message":message, "message_truncated":truncated});
        let mut result = CallToolResult::error(vec![ContentBlock::text(data.to_string())]);
        result.structured_content = Some(data);
        if measured(&result).is_ok_and(|bytes| bytes <= budget.bytes()) {
            return result;
        }
        chars /= 2;
    }
}

fn too_small(budget: OutputBudget, message: &str) -> CallToolResult {
    let data = json!({"status": "output_budget_too_small", "max_output_bytes": budget.bytes(), "message": message});
    let mut result = CallToolResult::error(vec![ContentBlock::text(data.to_string())]);
    result.structured_content = Some(data);
    result
}

pub(crate) fn lexical_row(
    result: &crate::storage::SearchResult,
    view: OutputView,
    query: &str,
) -> Value {
    let (doc, doc_cut) = preview(
        result.doc_comment.as_deref().unwrap_or(""),
        if view == OutputView::Compact {
            240
        } else {
            1200
        },
    );
    let (signature, sig_cut) = preview(
        result.signature.as_deref().unwrap_or(""),
        if view == OutputView::Compact {
            240
        } else {
            1200
        },
    );
    let mut row = json!({"symbol_id": result.symbol_id.value(), "name": result.name, "kind": result.kind,
        "file_path": result.file_path, "line": result.line, "language_id":result.language_id, "signature": signature,
        "preview": doc, "preview_truncated": doc_cut || sig_cut});
    for key in ["signature", "preview"] {
        if row[key] == "" {
            row.as_object_mut().expect("row object").remove(key);
        }
    }
    if let Some((matched, total)) = crate::storage::tantivy::discovery_term_coverage(query, result)
    {
        row["term_coverage"] = json!([matched, total]);
    }
    if view == OutputView::Detail {
        row["raw_lexical_score"] = json!(result.score);
        row["module_path"] = json!(result.module_path);
    }
    row
}

pub(crate) fn symbol_row(symbol: &crate::Symbol, view: OutputView) -> Value {
    let (doc, doc_cut) = preview(
        symbol.doc_comment.as_deref().unwrap_or(""),
        if view == OutputView::Compact {
            240
        } else {
            1200
        },
    );
    let (signature, sig_cut) = preview(
        symbol.signature.as_deref().unwrap_or(""),
        if view == OutputView::Compact {
            240
        } else {
            1200
        },
    );
    json!({"symbol_id": symbol.id.value(), "name": symbol.name, "kind": symbol.kind,
        "file_path": symbol.file_path, "line": symbol.range.start_line + 1, "language_id":symbol.language_id,
        "signature": signature, "preview": doc, "preview_truncated": doc_cut || sig_cut})
}

pub(crate) fn document_row(
    result: &crate::documents::SearchResult,
    view: OutputView,
    workspace: Option<&Path>,
) -> Value {
    let (content, cut) = preview(
        &result.content_preview,
        if view == OutputView::Compact {
            280
        } else {
            4096
        },
    );
    json!({"chunk_id": result.chunk_id.value(), "collection": result.collection,
        "source_path": display_path(&result.source_path, workspace), "heading_context": result.heading_context,
        "byte_range": result.byte_range, "content_preview": content, "preview_truncated": cut,
        "similarity": result.similarity})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_output_budget_validates_numeric_boundaries() {
        for value in ["0", "1023", "65537", "-1", "1.5", "\"8192\""] {
            assert!(serde_json::from_str::<OutputBudget>(value).is_err());
        }
        for value in ["1024", "8192", "65536"] {
            assert!(serde_json::from_str::<OutputBudget>(value).is_ok());
        }
    }
    #[test]
    fn compact_output_counts_both_representations_and_escaping() {
        let rows: Vec<_> = (0..100)
            .map(|id| json!({"id": id, "path": "x\\\"🦀".repeat(20)}))
            .collect();
        let result = bounded(
            json!({"results": rows}),
            vec![Section::primary("/results")],
            OutputBudget::new(2048).unwrap(),
        )
        .unwrap();
        assert!(measured(&result).unwrap() <= 2048);
        let data = result.structured_content.unwrap();
        assert!(data["results"].as_array().unwrap().len() < 100);
        assert_eq!(data["output"]["partial"], true);
    }
    #[test]
    fn compact_output_round_robins_primary_sources_before_details() {
        let result = bounded(json!({"code": [{"id":1},{"id":2}], "docs": [{"id":3}], "details": [{"text":"x".repeat(4000)}]}),
            vec![Section::primary("/code"), Section::primary("/docs"), Section::secondary("/details")], OutputBudget::new(2048).unwrap()).unwrap();
        let data = result.structured_content.unwrap();
        assert_eq!(data["code"].as_array().unwrap().len(), 2);
        assert_eq!(data["docs"].as_array().unwrap().len(), 1);
        assert_eq!(data["details"], json!([]));
    }
    #[test]
    fn compact_output_primary_text_precedes_alphabetically_earlier_details() {
        let result = bounded(
            json!({"results":[{"name":"primary_hit"}], "impact":[{"name":"secondary_hit"}]}),
            vec![Section::primary("/results"), Section::secondary("/impact")],
            OutputBudget::default(),
        )
        .unwrap();
        let ContentBlock::Text(text) = &result.content[0] else {
            panic!("text result");
        };
        assert!(text.text.find("primary_hit").unwrap() < text.text.find("secondary_hit").unwrap());
        assert_eq!(text.text.matches("primary_hit").count(), 1);
    }
    #[test]
    fn compact_output_never_shortens_identity_or_emits_looping_page() {
        let path = "long/".repeat(3000);
        let result = bounded(
            json!({"coverage":{"items":[{"path":path},{"path":"small"}], "next_offset":25}}),
            vec![Section::page("/coverage/items", 0)],
            OutputBudget::new(2048).unwrap(),
        )
        .unwrap();
        let data = result.structured_content.unwrap();
        assert_eq!(data["coverage"]["items"], json!([]));
        assert!(data["coverage"]["next_offset"].is_null());
        assert_eq!(data["output"]["lists"]["/coverage/items"]["omitted"], 2);
    }
    #[test]
    fn compact_output_page_continues_after_actual_returned_rows() {
        let rows: Vec<_> = (0..25)
            .map(|id| json!({"id":id, "name":"x".repeat(100)}))
            .collect();
        let result = bounded(
            json!({"coverage":{"items":rows, "next_offset":50}}),
            vec![Section::page("/coverage/items", 25)],
            OutputBudget::new(2048).unwrap(),
        )
        .unwrap();
        let data = result.structured_content.unwrap();
        let shown = data["coverage"]["items"].as_array().unwrap().len();
        assert!(shown > 0 && shown < 25);
        assert_eq!(data["coverage"]["next_offset"], json!(25 + shown));
    }
    #[test]
    fn compact_output_preview_is_unicode_safe_and_explicit() {
        assert_eq!(preview("🦀éalpha", 2), ("🦀é".into(), true));
        assert_eq!(preview("🦀é", 2), ("🦀é".into(), false));
    }
    #[test]
    fn compact_output_large_metadata_is_a_bounded_error_not_empty_success() {
        let result = bounded(
            json!({"warning":"x".repeat(10000)}),
            vec![],
            OutputBudget::new(1024).unwrap(),
        )
        .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(measured(&result).unwrap() <= 1024);
    }
}
