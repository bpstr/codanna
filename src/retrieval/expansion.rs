//! Bounded typed expansion over one pinned graph view.
use super::{CodeEvidence, Source, add_evidence};
use crate::storage::tantivy::GraphView;
use crate::{RelationKind, Symbol, SymbolId};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct PathEvidence {
    pub seed_symbol_id: u32,
    pub relation: String,
    pub direction: &'static str,
    pub basis: &'static str,
    pub line: Option<u32>,
}
#[derive(Debug, Default, Serialize)]
pub struct Expansion {
    pub seeds_examined: usize,
    pub seeds_omitted: usize,
    pub edges_omitted: usize,
    pub excluded_by_scope: usize,
    pub missing_endpoints: usize,
    pub warnings: Vec<String>,
}

pub(crate) fn row(symbol: &Symbol) -> CodeEvidence {
    CodeEvidence {
        symbol_id: symbol.id.value(),
        name: super::bounded_text(&symbol.name, 256),
        kind: format!("{:?}", symbol.kind),
        file_path: super::bounded_text(&symbol.file_path, 2048),
        line: symbol.range.start_line.saturating_add(1),
        signature: symbol
            .signature
            .as_deref()
            .map(|s| super::bounded_text(s, 2048)),
        facets: super::profile::facets(symbol),
        ..Default::default()
    }
}

pub(crate) fn expand(
    view: &GraphView<'_>,
    candidates: &mut BTreeMap<u32, CodeEvidence>,
    seeds: &[u32],
    prefix: Option<&str>,
    incoming: bool,
    both: bool,
) -> Expansion {
    let mut report = Expansion {
        seeds_omitted: seeds.len().saturating_sub(64),
        ..Default::default()
    };
    for (rank, &seed) in seeds.iter().take(64).enumerate() {
        let Some(id) = SymbolId::new(seed) else {
            continue;
        };
        report.seeds_examined += 1;
        for direction in [incoming, !incoming]
            .into_iter()
            .take(if both { 2 } else { 1 })
        {
            let page = match view.relationships_page(
                &[id],
                direction,
                &[RelationKind::Calls, RelationKind::References],
                0,
                128,
            ) {
                Ok(page) => page,
                Err(error) => {
                    report.warnings.push(error.to_string());
                    continue;
                }
            };
            report.edges_omitted += page.total.saturating_sub(page.edges.len());
            let ids: Vec<_> = page
                .edges
                .iter()
                .map(|(from, to, _)| if direction { *from } else { *to })
                .collect();
            let symbols = match prefix {
                Some(prefix) => view.symbols_scoped(&ids, prefix),
                None => view.symbols(&ids),
            };
            let symbols: std::collections::HashMap<_, _> = match symbols {
                Ok(symbols) => symbols.into_iter().map(|s| (s.id, s)).collect(),
                Err(error) => {
                    report.warnings.push(error.to_string());
                    continue;
                }
            };
            let missing: Vec<_> = ids
                .iter()
                .filter(|id| !symbols.contains_key(id))
                .copied()
                .collect();
            let present = match view.symbols(&missing) {
                Ok(symbols) => symbols.len(),
                Err(error) => {
                    report.warnings.push(error.to_string());
                    0
                }
            };
            report.excluded_by_scope += present;
            report.missing_endpoints += missing.len().saturating_sub(present);
            for (from, to, relationship) in page.edges {
                let target = if direction { from } else { to };
                let Some(symbol) = symbols.get(&target) else {
                    continue;
                };
                add_evidence(candidates, row(symbol), Source::Graph, rank + 1, None, None);
                let evidence = PathEvidence {
                    seed_symbol_id: seed,
                    relation: format!("{:?}", relationship.kind),
                    direction: if direction { "incoming" } else { "outgoing" },
                    basis: "resolved_index",
                    line: relationship
                        .metadata
                        .as_ref()
                        .and_then(|m| m.line)
                        .map(|l| l.saturating_add(1)),
                };
                let paths = &mut candidates
                    .get_mut(&target.value())
                    .expect("inserted candidate")
                    .relationships;
                if paths.len() < 128
                    && !paths.iter().any(|p| {
                        p.seed_symbol_id == seed
                            && p.relation == evidence.relation
                            && p.direction == evidence.direction
                            && p.line == evidence.line
                    })
                {
                    paths.push(evidence);
                }
            }
        }
    }
    report
}
