//! Query existing knowledge snapshots; never infer an asserted relationship from prose.
use super::{CodeEvidence, Source, add_evidence};
use crate::knowledge::{Basis, Kind, io};
use crate::storage::tantivy::GraphView;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct LinkEvidence {
    pub document_id: String,
    pub target_id: String,
    pub source_path: String,
    pub source_line: u32,
    pub relation: String,
    pub basis: String,
    pub source_hash: String,
    pub freshness: &'static str,
}
#[derive(Debug, Default, Serialize)]
pub struct LinkReport {
    pub status: &'static str,
    pub matched_sections: usize,
    pub omitted_sections: usize,
    pub omitted_edges: usize,
    pub stale_or_unverified: usize,
    pub ambiguous_or_missing: usize,
    pub warnings: Vec<String>,
}

pub(crate) fn collect(
    view: &GraphView<'_>,
    root: &Path,
    repo: &str,
    query: &str,
    prefix: Option<&str>,
    candidates: &mut BTreeMap<u32, CodeEvidence>,
) -> LinkReport {
    let mut report = LinkReport {
        status: "completed_bounded",
        ..Default::default()
    };
    let path = root.join(".codanna/knowledge.json");
    if !path.exists() {
        report.status = "not_configured";
        return report;
    }
    let graph = match io::load(&path) {
        Ok(graph) => graph,
        Err(error) => {
            report.status = "unavailable";
            report.warnings.push(error.to_string());
            return report;
        }
    };
    if !graph.repositories.contains_key(repo) {
        report.status = "repository_not_found";
        return report;
    }
    let symbols = match view.inventory(Some(prefix.unwrap_or(".")), 100_000) {
        Ok(symbols) => symbols,
        Err(error) => {
            report.status = "unavailable";
            report.warnings.push(error.to_string());
            return report;
        }
    };
    let tokens: Vec<_> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .map(str::to_lowercase)
        .collect();
    let mut docs: Vec<_> = graph
        .nodes
        .values()
        .filter(|node| {
            node.source.repo == repo && matches!(node.kind, Kind::Document | Kind::Section)
        })
        .filter_map(|node| {
            let text = format!("{} {}", node.label, node.excerpt).to_lowercase();
            let score = tokens
                .iter()
                .filter(|token| text.contains(token.as_str()))
                .count();
            (score > 0).then_some((score, node))
        })
        .collect();
    docs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    report.matched_sections = docs.len();
    report.omitted_sections = docs.len().saturating_sub(16);
    let documents: BTreeSet<_> = docs.iter().take(16).map(|(_, n)| n.id.as_str()).collect();
    let mut checked: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut edges = 0;
    for edge in &graph.edges {
        if edge.basis == Basis::Candidate
            || !documents.contains(edge.from.as_str())
            || !matches!(
                edge.relation.as_str(),
                "references" | "describes" | "implements"
            )
        {
            continue;
        }
        let target = &graph.nodes[&edge.to];
        if target.source.repo != repo || !matches!(target.kind, Kind::Symbol | Kind::Test) {
            continue;
        }
        edges += 1;
        if edges > 128 {
            report.omitted_edges += 1;
            continue;
        }
        let document = &graph.nodes[&edge.from];
        let current = [&document.source, &target.source, &edge.evidence]
            .iter()
            .all(|span| {
                if span.repo != repo {
                    return false;
                }
                let hash = checked.entry(span.path.clone()).or_insert_with(|| {
                    io::source_path(root, &span.path)
                        .ok()
                        .and_then(|path| {
                            io::read_bounded(&path, crate::knowledge::MAX_FILE_BYTES as u64).ok()
                        })
                        .map(crate::knowledge::digest)
                });
                hash.as_deref() == Some(span.hash.as_str())
            });
        if !current {
            report.stale_or_unverified += 1;
            continue;
        }
        let matched: Vec<_> = symbols
            .iter()
            .filter(|symbol| {
                symbol.file_path.as_ref() == target.source.path
                    && symbol.range.start_line.saturating_add(1) == target.source.start_line
                    && (target.label == symbol.name.as_ref()
                        || target.label.ends_with(&format!("::{}", symbol.name))
                        || target.label.ends_with(&format!(".{}", symbol.name)))
            })
            .collect();
        let [symbol] = matched.as_slice() else {
            report.ambiguous_or_missing += 1;
            continue;
        };
        // Bind the current source to the pinned code registration as well as the graph.
        if view.file_hash(symbol.file_id).ok().flatten().as_deref()
            != Some(target.source.hash.as_str())
        {
            report.stale_or_unverified += 1;
            continue;
        }
        add_evidence(
            candidates,
            super::expansion::row(symbol),
            Source::KnowledgeLink,
            1,
            None,
            None,
        );
        let row = candidates
            .get_mut(&symbol.id.value())
            .expect("inserted link target");
        row.knowledge_links.push(LinkEvidence {
            document_id: edge.from.clone(),
            target_id: edge.to.clone(),
            source_path: document.source.path.clone(),
            source_line: edge.evidence.start_line,
            relation: edge.relation.clone(),
            basis: format!("{:?}", edge.basis),
            source_hash: target.source.hash.clone(),
            freshness: "source_hash_verified",
        });
    }
    if report.stale_or_unverified
        + report.ambiguous_or_missing
        + report.omitted_sections
        + report.omitted_edges
        > 0
    {
        report.status = "partial";
    }
    report
}
