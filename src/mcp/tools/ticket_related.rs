//! Optional one-hop Calls evidence. It never changes direct retrieval rankings.
use crate::indexing::facade::IndexFacade;
use crate::{RelationKind, Symbol, SymbolId};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_SEEDS: usize = 3;
pub(super) const EDGES_PER_SEED: usize = 32;
pub(super) const MAX_RELATED: usize = 6;
const MAX_DIRECT: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Via {
    pub seed_symbol_id: u32,
    pub seed_rank: usize,
    pub seed_name: String,
    pub seed_file_path: String,
    pub relation: &'static str,
    pub call_line: Option<u32>,
    pub call_column: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Item {
    pub symbol_id: u32,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub identity_text_shortened: bool,
    pub via: Vec<Via>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Probe {
    pub seed_symbol_id: u32,
    pub seed_rank: usize,
    pub status: &'static str,
    pub indexed_edges: Option<usize>,
    pub unhydrated_edges: usize,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RelatedCode {
    pub status: &'static str,
    pub source_coverage: &'static str,
    pub freshness: &'static str,
    pub reader_generation_before: Option<u64>,
    pub reader_generation_after: Option<u64>,
    pub max_seeds: usize,
    pub edges_per_seed: usize,
    pub result_limit: usize,
    pub distinct_targets: usize,
    pub omitted_targets: usize,
    pub probes: Vec<Probe>,
    pub items: Vec<Item>,
}

// Plain-text evidence only. ID fields remain authoritative when unusually long
// or control-bearing stored names/paths must be shortened for the output budget.
fn text(value: &str, budget: usize) -> String {
    let mut result = String::new();
    for character in value.chars().filter(|character| !character.is_control()) {
        if result.len() + character.len_utf8() > budget {
            break;
        }
        result.push(character);
    }
    result
}

impl RelatedCode {
    pub(super) fn unavailable(status: &'static str) -> Self {
        Self {
            status,
            source_coverage: "unknown",
            freshness: "unknown",
            reader_generation_before: None,
            reader_generation_after: None,
            max_seeds: MAX_SEEDS,
            edges_per_seed: EDGES_PER_SEED,
            result_limit: MAX_RELATED,
            distinct_targets: 0,
            omitted_targets: 0,
            probes: Vec::new(),
            items: Vec::new(),
        }
    }

    pub(super) fn render(&self) -> String {
        let mut output = format!(
            "\n## Related implementations (indexed Calls, not relevance scores)\nStatus: {}\n",
            self.status
        );
        for (rank, item) in self.items.iter().enumerate() {
            output.push_str(&format!(
                "{}. {} ({}) at {}:{} [symbol_id:{}]\n",
                rank + 1,
                item.name,
                item.kind,
                item.file_path,
                item.line,
                item.symbol_id
            ));
            for via in &item.via {
                output.push_str(&format!(
                    "   Indexed Calls from direct rank {}: {} [symbol_id:{}] at {}",
                    via.seed_rank, via.seed_name, via.seed_symbol_id, via.seed_file_path
                ));
                if let Some(line) = via.call_line {
                    output.push_str(&format!(":{line}"));
                }
                output.push('\n');
            }
            if item.identity_text_shortened {
                output.push_str("   Display identity shortened; resolve the symbol ID for full identity.\n");
            }
        }
        for probe in &self.probes {
            if let Some(warning) = &probe.warning {
                output.push_str(&format!(
                    "Warning for seed {}: {warning}\n",
                    probe.seed_symbol_id
                ));
            }
            if probe.unhydrated_edges > 0 {
                output.push_str(&format!(
                    "Seed {}: {} indexed edge(s) have missing target documents.\n",
                    probe.seed_symbol_id, probe.unhydrated_edges
                ));
            }
        }
        if self.omitted_targets > 0 {
            output.push_str(&format!(
                "{} additional distinct indexed target(s) omitted by the result limit.\n",
                self.omitted_targets
            ));
        }
        output.push_str("One-hop indexed evidence only; source coverage and freshness are unknown.\n");
        output
    }
}

fn item(symbol: &Symbol) -> Item {
    let name = text(&symbol.name, 256);
    let file_path = text(&symbol.file_path, 2048);
    Item {
        symbol_id: symbol.id.value(),
        identity_text_shortened: name != symbol.name.as_ref()
            || file_path != symbol.file_path.as_ref(),
        name,
        kind: format!("{:?}", symbol.kind),
        file_path,
        line: symbol.range.start_line.saturating_add(1),
        column: symbol.range.start_column,
        via: Vec::new(),
    }
}

pub(super) fn collect(
    indexer: &IndexFacade,
    direct_ids: &[u32],
    scope: Option<&str>,
    expected_generation: Option<u64>,
) -> RelatedCode {
    if scope.is_some() {
        return RelatedCode::unavailable("not_run_scoped_graph_unsupported");
    }
    if direct_ids.len() > MAX_DIRECT {
        return RelatedCode::unavailable("not_run_seed_budget_exceeded");
    }
    if direct_ids.is_empty() {
        return RelatedCode::unavailable("not_run_no_direct_matches");
    }
    let storage = indexer.document_index();
    let before = storage.generation();
    let mut result = RelatedCode::unavailable("completed_bounded");
    result.reader_generation_before = Some(before);
    if expected_generation != Some(before) {
        result.status = "not_run_generation_mismatch";
        result.reader_generation_after = Some(storage.generation());
        return result;
    }
    // Both edge enumeration and all endpoint hydration use this pinned view.
    let view = storage.graph_view();
    let excluded: BTreeSet<_> = direct_ids.iter().copied().collect();
    let mut seen_seeds = BTreeSet::new();
    let mut found: BTreeMap<u32, Item> = BTreeMap::new();
    for (position, &seed_id) in direct_ids.iter().enumerate() {
        if !seen_seeds.insert(seed_id) {
            continue;
        }
        if result.probes.len() == MAX_SEEDS {
            break;
        }
        let mut probe = Probe {
            seed_symbol_id: seed_id,
            seed_rank: position + 1,
            status: "completed_indexed_neighborhood",
            indexed_edges: None,
            unhydrated_edges: 0,
            warning: None,
        };
        let Some(id) = SymbolId::new(seed_id) else {
            probe.status = "invalid_seed";
            result.probes.push(probe);
            continue;
        };
        let operation = (|| {
            let seeds = view.symbols(&[id])?;
            let Some(seed) = seeds.first() else {
                probe.status = "missing_seed";
                return Ok::<(), crate::storage::StorageError>(());
            };
            let mut edges = view.relationships(&[id], false, &[RelationKind::Calls], Some(EDGES_PER_SEED))?;
            probe.indexed_edges = Some(edges.len());
            edges.sort_by_key(|(_, target, relationship)| {
                (
                    target.value(),
                    relationship.metadata.as_ref().and_then(|metadata| metadata.line),
                    relationship.metadata.as_ref().and_then(|metadata| metadata.column),
                )
            });
            let target_ids: Vec<_> = edges.iter().map(|(_, target, _)| *target).collect::<BTreeSet<_>>().into_iter().collect();
            let targets: BTreeMap<_, _> = view.symbols(&target_ids)?.into_iter().map(|symbol| (symbol.id, symbol)).collect();
            for (_, target_id, relationship) in edges {
                let Some(target) = targets.get(&target_id) else {
                    probe.unhydrated_edges += 1;
                    continue;
                };
                if excluded.contains(&target_id.value()) {
                    continue;
                }
                let related = found.entry(target_id.value()).or_insert_with(|| item(target));
                // Multiple call sites do not get extra relevance weight. One
                // deterministic site is enough to explain each seed/target edge.
                if related.via.iter().any(|via| via.seed_symbol_id == seed_id) {
                    continue;
                }
                let seed_name = text(&seed.name, 256);
                let seed_path = text(&seed.file_path, 2048);
                related.identity_text_shortened |= seed_name != seed.name.as_ref() || seed_path != seed.file_path.as_ref();
                related.via.push(Via {
                    seed_symbol_id: seed_id,
                    seed_rank: position + 1,
                    seed_name,
                    seed_file_path: seed_path,
                    relation: "Calls",
                    call_line: relationship.metadata.as_ref().and_then(|metadata| metadata.line).map(|line| line.saturating_add(1)),
                    call_column: relationship.metadata.as_ref().and_then(|metadata| metadata.column),
                });
            }
            if probe.unhydrated_edges > 0 {
                probe.status = "partial_unhydrated";
            }
            Ok(())
        })();
        if let Err(error) = operation {
            probe.status = match error {
                crate::storage::StorageError::GraphBudgetExceeded { .. } => "edge_budget_exceeded",
                _ => "unavailable",
            };
            probe.warning = Some(text(&error.to_string(), 512));
        }
        result.probes.push(probe);
    }
    result.reader_generation_after = Some(storage.generation());
    if result.reader_generation_after != Some(before) {
        result.status = "discarded_generation_changed";
        return result;
    }
    result.items = found.into_values().collect();
    result.items.sort_by(|left, right| {
        left.via[0].seed_rank.cmp(&right.via[0].seed_rank)
            .then_with(|| left.file_path.cmp(&right.file_path))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.column.cmp(&right.column))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.symbol_id.cmp(&right.symbol_id))
    });
    result.distinct_targets = result.items.len();
    result.omitted_targets = result.items.len().saturating_sub(MAX_RELATED);
    result.items.truncate(MAX_RELATED);
    result.status = if result.probes.iter().any(|probe| probe.status != "completed_indexed_neighborhood") {
        "partial"
    } else if result.items.is_empty() {
        "empty_indexed_neighborhoods"
    } else {
        "completed_bounded"
    };
    result
}
