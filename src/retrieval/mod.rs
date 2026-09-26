//! Shared candidate/evidence contracts. Scores express rank contributions, not confidence.
use crate::storage::SearchResult;
use serde::Serialize;
use std::collections::BTreeMap;
const MAX_ANCHORS: usize = 8;
const RRF_K: f64 = 60.0;
pub(crate) mod expansion;
pub(crate) mod links;
pub mod profile;
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Anchor {
    pub(crate) identifier: String,
    pub(crate) document_rank: usize,
    pub(crate) source_path: String,
    /// Byte offsets within the returned preview, not the original document.
    pub(crate) preview_start_byte: usize,
    pub(crate) preview_end_byte: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    Lexical,
    Semantic,
    DocumentAnchor,
    KnowledgeLink,
    Graph,
    Facet,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Contribution {
    pub(crate) source: Source,
    pub(crate) rank: usize,
    pub(crate) raw_score: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct CodeEvidence {
    pub(crate) symbol_id: u32,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) file_path: String,
    pub(crate) line: u32,
    pub(crate) signature: Option<String>,
    pub(crate) fusion_score: f64,
    pub(crate) contributions: Vec<Contribution>,
    pub(crate) document_anchors: Vec<Anchor>,
    pub(crate) facets: Vec<profile::Facet>,
    pub(crate) relationships: Vec<expansion::PathEvidence>,
    pub(crate) knowledge_links: Vec<links::LinkEvidence>,
}

impl CodeEvidence {
    pub(crate) fn from_lexical(result: &SearchResult) -> Self {
        Self {
            symbol_id: result.symbol_id.value(),
            name: bounded_text(&result.name, 256),
            kind: format!("{:?}", result.kind),
            file_path: bounded_text(&result.file_path, 2048),
            line: result.line,
            signature: result
                .signature
                .as_deref()
                .map(|text| bounded_text(text, 2048)),
            fusion_score: 0.0,
            contributions: Vec::new(),
            document_anchors: Vec::new(),
            ..Self::default()
        }
    }
}

pub(crate) fn add_evidence(
    candidates: &mut BTreeMap<u32, CodeEvidence>,
    row: CodeEvidence,
    source: Source,
    rank: usize,
    raw_score: Option<f32>,
    anchor: Option<&Anchor>,
) {
    let current = candidates.entry(row.symbol_id).or_insert(row);
    let rank = rank.max(1);
    if let Some(previous) = current
        .contributions
        .iter_mut()
        .find(|item| item.source == source)
    {
        if rank < previous.rank {
            previous.rank = rank;
            previous.raw_score = raw_score.filter(|score| score.is_finite());
        }
    } else {
        current.contributions.push(Contribution {
            source,
            rank,
            raw_score: raw_score.filter(|score| score.is_finite()),
        });
    }
    if let Some(anchor) = anchor {
        if current.document_anchors.len() < MAX_ANCHORS
            && !current.document_anchors.iter().any(|old| {
                old.identifier == anchor.identifier && old.source_path == anchor.source_path
            })
        {
            current.document_anchors.push(anchor.clone());
        }
    }
}

pub(crate) fn rank_candidates(
    candidates: BTreeMap<u32, CodeEvidence>,
    query: &str,
    limit: usize,
) -> Vec<CodeEvidence> {
    let mut rows: Vec<_> = candidates.into_values().collect();
    for row in &mut rows {
        row.contributions.sort_by_key(|item| item.source);
        row.fusion_score = row
            .contributions
            .iter()
            .map(|item| 1.0 / (RRF_K + item.rank as f64))
            .sum();
    }
    let exact = |row: &CodeEvidence| {
        identifier(query)
            && row.name == query
            && row
                .contributions
                .iter()
                .any(|item| item.source == Source::Lexical)
    };
    rows.sort_by(|left, right| {
        exact(right)
            .cmp(&exact(left))
            .then_with(|| right.fusion_score.total_cmp(&left.fusion_score))
            .then_with(|| left.file_path.cmp(&right.file_path))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.symbol_id.cmp(&right.symbol_id))
    });
    rows.truncate(limit);
    rows
}

pub(crate) fn bounded_text(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

pub(crate) fn identifier(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 96
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
