//! Explicit retrieval objectives and deterministic, evidence-labelled dimensions.
use super::CodeEvidence;
use crate::Symbol;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    #[default]
    Relevant,
    ImplementationOwner,
    Impact,
    Coverage,
    EvidenceStrength,
}

#[derive(Debug, Clone, Serialize)]
pub struct Facet {
    pub facet: &'static str,
    pub value: String,
    pub basis: &'static str,
    pub source_path: String,
    pub line: u32,
    pub extractor_version: &'static str,
}

pub(crate) fn facets(symbol: &Symbol) -> Vec<Facet> {
    let mut dimensions = vec![
        ("kind", format!("{:?}", symbol.kind)),
        ("visibility", format!("{:?}", symbol.visibility)),
    ];
    if let Some(language) = symbol.language_id {
        dimensions.push(("language", language.to_string()));
    }
    dimensions
        .into_iter()
        .map(|(facet, value)| Facet {
            facet,
            value,
            basis: "observed_index",
            source_path: super::bounded_text(&symbol.file_path, 2048),
            line: symbol.range.start_line.saturating_add(1),
            extractor_version: "indexed-facets-v1",
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct CoverageItem {
    pub symbol_id: u32,
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub responsibility: &'static str,
    pub ownership: &'static str,
    pub component_family: String,
    pub test_source: bool,
    pub test_basis: &'static str,
    pub documents: Vec<String>,
    pub evidence_basis: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Coverage {
    pub status: &'static str,
    pub snapshot: String,
    pub total_candidates: usize,
    pub next_offset: Option<usize>,
    pub repository_complete: bool,
    pub limitations: Vec<String>,
    pub items: Vec<CoverageItem>,
}

pub(crate) fn coverage(
    rows: &[CodeEvidence],
    request_identity: &str,
    generation: u64,
    offset: usize,
    limit: usize,
    expected: Option<&str>,
    limitations: Vec<String>,
) -> Coverage {
    // Include complete provenance and content in the cursor, not only mutable numeric IDs.
    let identity = format!(
        "{request_identity}\n{generation}\n{}",
        serde_json::to_string(rows).unwrap_or_default()
    );
    let snapshot = crate::indexing::calculate_hash(&identity);
    let stale = expected.is_some_and(|value| value != snapshot);
    let items = if stale {
        vec![]
    } else {
        rows.iter()
            .skip(offset)
            .take(limit)
            .map(|row| {
                let consumer = row
                    .relationships
                    .iter()
                    .any(|path| path.direction == "incoming");
                CoverageItem {
                    symbol_id: row.symbol_id,
                    name: row.name.clone(),
                    file_path: row.file_path.clone(),
                    line: row.line,
                    responsibility: if consumer {
                        "consumer"
                    } else {
                        "direct_candidate"
                    },
                    ownership: "unverified",
                    component_family: row
                        .file_path
                        .rsplit_once('/')
                        .map_or(".", |(parent, _)| parent)
                        .to_owned(),
                    test_source: row
                        .file_path
                        .split('/')
                        .any(|part| matches!(part, "tests" | "test" | "__tests__"))
                        || row.file_path.contains(".test.")
                        || row.file_path.contains(".spec."),
                    test_basis: "path_convention_not_executed_coverage",
                    documents: row
                        .knowledge_links
                        .iter()
                        .map(|l| l.document_id.clone())
                        .collect(),
                    evidence_basis: vec!["indexed_evidence"],
                }
            })
            .collect()
    };
    Coverage {
        status: if stale {
            "snapshot_changed_restart_required"
        } else {
            "bounded_candidate_coverage"
        },
        snapshot,
        total_candidates: rows.len(),
        next_offset: (!stale && offset + limit < rows.len()).then_some(offset + limit),
        repository_complete: false,
        limitations,
        items,
    }
}

pub(crate) fn order(rows: &mut [CodeEvidence], profile: Profile) {
    if profile == Profile::ImplementationOwner {
        // Public declarations reached through a reverse reference are owner candidates.
        // Private helper callers and fan-in counts alone do not establish ownership.
        rows.sort_by_key(|row| {
            let public = row
                .facets
                .iter()
                .any(|f| f.facet == "visibility" && f.value == "Public");
            let references = row
                .relationships
                .iter()
                .any(|p| p.direction == "incoming" && p.relation == "References");
            let calls = row
                .relationships
                .iter()
                .any(|p| p.direction == "incoming" && p.relation == "Calls");
            if public && references {
                0
            } else if public && calls {
                1
            } else {
                2
            }
        });
    } else if profile == Profile::Impact {
        rows.sort_by_key(|row| {
            std::cmp::Reverse(
                row.relationships
                    .iter()
                    .filter(|p| p.direction == "incoming")
                    .map(|p| p.seed_symbol_id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
            )
        });
    } else if profile == Profile::EvidenceStrength {
        rows.sort_by_key(|row| row.knowledge_links.is_empty() && row.relationships.is_empty());
    } else if profile == Profile::Coverage {
        rows.sort_by(|a, b| {
            (&a.file_path, a.line, a.symbol_id).cmp(&(&b.file_path, b.line, b.symbol_id))
        });
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FacetFilter {
    pub facet: String,
    pub value: String,
}

pub(crate) fn matches(facets: &[Facet], filters: &[FacetFilter]) -> bool {
    filters.iter().all(|filter| {
        facets
            .iter()
            .any(|f| f.facet == filter.facet && f.value.eq_ignore_ascii_case(&filter.value))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evidence_v1_coverage_pages_reach_every_bounded_candidate_and_reject_changes() {
        let rows: Vec<_> = (1..=19)
            .map(|id| CodeEvidence {
                symbol_id: id,
                name: format!("consumer{id}"),
                file_path: format!("src/{id}.rs"),
                ..Default::default()
            })
            .collect();
        let first = coverage(&rows, "avatar|src", 7, 0, 10, None, vec![]);
        assert_eq!(first.items.len(), 10);
        let next = coverage(
            &rows,
            "avatar|src",
            7,
            first.next_offset.unwrap(),
            10,
            Some(&first.snapshot),
            vec![],
        );
        assert_eq!(next.items.len(), 9);
        assert_eq!(next.next_offset, None);
        assert!(!next.repository_complete);
        assert!(
            coverage(
                &rows,
                "avatar|src",
                8,
                10,
                10,
                Some(&first.snapshot),
                vec![]
            )
            .items
            .is_empty()
        );
        assert!(
            coverage(
                &rows,
                "timezone|src",
                7,
                10,
                10,
                Some(&first.snapshot),
                vec![]
            )
            .items
            .is_empty()
        );
        assert_eq!(first.items.len() + next.items.len(), 19);
    }
    #[test]
    fn evidence_v1_missing_facet_is_unknown_and_strict_filters_are_explicit() {
        let filter = FacetFilter {
            facet: "language".into(),
            value: "rust".into(),
        };
        assert!(!matches(&[], &[filter]));
        assert!(matches(&[], &[]));
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;
    #[test]
    fn evidence_v1_private_callers_do_not_displace_direct_owner_candidate() {
        let mut rows = vec![
            CodeEvidence {
                symbol_id: 1,
                name: "public_entry".into(),
                ..Default::default()
            },
            CodeEvidence {
                symbol_id: 2,
                name: "private_helper".into(),
                relationships: vec![super::super::expansion::PathEvidence {
                    seed_symbol_id: 1,
                    relation: "Calls".into(),
                    direction: "incoming",
                    basis: "resolved_index",
                    line: None,
                }],
                ..Default::default()
            },
        ];
        order(&mut rows, Profile::ImplementationOwner);
        assert_eq!(rows[0].symbol_id, 1);
        order(&mut rows, Profile::Impact);
        assert_eq!(
            rows[0].symbol_id, 2,
            "impact considers an indexed dependent even when it is not an owner candidate"
        );
    }
}
