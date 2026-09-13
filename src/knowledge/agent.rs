//! Fast agent-oriented analysis over an already-loaded knowledge graph.
//! The index is deterministic, read-only and intentionally excludes candidate edges.
use crate::knowledge::*;
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone)]
pub struct AnalysisIndex {
    incoming: BTreeMap<String, Vec<usize>>,
    outgoing: BTreeMap<String, Vec<usize>>,
    by_file: BTreeMap<(String, String), Vec<String>>,
}

impl AnalysisIndex {
    pub fn new(graph: &Graph) -> Self {
        let mut incoming: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut outgoing: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, edge) in graph.edges.iter().enumerate() {
            if edge.basis == Basis::Candidate {
                continue;
            }
            outgoing.entry(edge.from.clone()).or_default().push(index);
            incoming.entry(edge.to.clone()).or_default().push(index);
        }
        let mut by_file: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for node in graph.nodes.values() {
            by_file
                .entry((node.source.repo.clone(), node.source.path.clone()))
                .or_default()
                .push(node.id.clone());
        }
        for ids in by_file.values_mut() {
            ids.sort();
        }
        Self {
            incoming,
            outgoing,
            by_file,
        }
    }

    pub fn impact(&self, graph: &Graph, request: &ImpactRequest) -> Result<ImpactReport> {
        if request.files.is_empty()
            || request.files.len() > 128
            || request.max_depth > 8
            || !(1..=512).contains(&request.max_nodes)
        {
            return Err("impact limits: files 1..128, depth 0..8, nodes 1..512".into());
        }
        for file in &request.files {
            validate_path(file)?;
        }
        if !graph.repositories.contains_key(&request.repo) {
            return Err("unknown repository".into());
        }

        let mut seed_ids = BTreeSet::new();
        let mut missing_files = Vec::new();
        for file in &request.files {
            match self.by_file.get(&(request.repo.clone(), file.clone())) {
                Some(ids) => seed_ids.extend(ids.iter().cloned()),
                None => missing_files.push(file.clone()),
            }
        }

        let mut queue: VecDeque<(String, usize)> =
            seed_ids.iter().cloned().map(|id| (id, 0)).collect();
        let mut visited: BTreeMap<String, usize> =
            seed_ids.iter().cloned().map(|id| (id, 0)).collect();
        let mut reasons: BTreeMap<String, String> = BTreeMap::new();
        let mut truncated = false;
        let mut examined = 0usize;
        while let Some((id, depth)) = queue.pop_front() {
            if depth >= request.max_depth {
                continue;
            }
            for edge_index in self.incoming.get(&id).into_iter().flatten() {
                examined += 1;
                if examined > 100_000 {
                    truncated = true;
                    break;
                }
                let edge = &graph.edges[*edge_index];
                if matches!(edge.relation.as_str(), "defines" | "contains") {
                    continue;
                }
                let next = edge.from.clone();
                if visited.contains_key(&next) {
                    continue;
                }
                if visited.len() >= request.max_nodes {
                    truncated = true;
                    continue;
                }
                let next_depth = depth + 1;
                visited.insert(next.clone(), next_depth);
                reasons.insert(
                    next.clone(),
                    format!("depends on changed evidence via {}", edge.relation),
                );
                queue.push_back((next, next_depth));
            }
            if examined > 100_000 {
                break;
            }
        }

        let mut impacted: Vec<_> = visited
            .iter()
            .filter(|(id, _)| !seed_ids.contains(*id))
            .map(|(id, depth)| ImpactItem {
                node: graph.nodes[id].clone(),
                depth: *depth,
                reason: reasons
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "connected impact".into()),
            })
            .collect();
        impacted.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.node.id.cmp(&b.node.id))
        });

        let cross_repo = impacted
            .iter()
            .filter(|item| item.node.source.repo != request.repo)
            .count();
        let direct = impacted.iter().filter(|item| item.depth == 1).count();
        let score = direct.saturating_mul(4) + cross_repo.saturating_mul(8) + impacted.len();
        let risk = if score >= 40 || cross_repo >= 3 {
            "high"
        } else if score >= 12 || cross_repo > 0 {
            "medium"
        } else {
            "low"
        };

        Ok(ImpactReport {
            repo: request.repo.clone(),
            files: request.files.clone(),
            changed_nodes: seed_ids.into_iter().filter_map(|id| graph.nodes.get(&id).cloned()).collect(),
            impacted,
            missing_files,
            direct_impacts: direct,
            cross_repository_impacts: cross_repo,
            risk: risk.into(),
            risk_score: score,
            truncated,
            semantics: "Reverse dependency traversal over resolved/explicit edges; candidate edges and defines/contains ownership edges are excluded. Risk is structural, not runtime severity.".into(),
        })
    }

    pub fn dead_code(&self, graph: &Graph, request: &DeadCodeRequest) -> Result<DeadCodeReport> {
        if request.limit == 0 || request.limit > 1000 {
            return Err("dead-code limit must be 1..1000".into());
        }
        if request
            .repo
            .as_ref()
            .is_some_and(|repo| !graph.repositories.contains_key(repo))
        {
            return Err("unknown repository".into());
        }
        let mut candidates = Vec::new();
        for node in graph.nodes.values() {
            if node.kind != Kind::Symbol {
                continue;
            }
            if request
                .repo
                .as_ref()
                .is_some_and(|repo| &node.source.repo != repo)
            {
                continue;
            }
            let label = node.label.rsplit("::").next().unwrap_or(&node.label);
            if is_common_entrypoint(label, &node.source.path) {
                continue;
            }
            let incoming = self
                .incoming
                .get(&node.id)
                .into_iter()
                .flatten()
                .filter(|edge_index| {
                    let edge = &graph.edges[**edge_index];
                    !matches!(edge.relation.as_str(), "defines" | "contains")
                })
                .count();
            if incoming != 0 {
                continue;
            }
            let outgoing = self
                .outgoing
                .get(&node.id)
                .into_iter()
                .flatten()
                .filter(|edge_index| {
                    !matches!(
                        graph.edges[**edge_index].relation.as_str(),
                        "defines" | "contains"
                    )
                })
                .count();
            candidates.push(DeadCodeCandidate {
                node: node.clone(),
                incoming_non_ownership_edges: 0,
                outgoing_edges: outgoing,
                confidence: if outgoing == 0 { "medium".into() } else { "low".into() },
                reason: "No resolved/explicit non-ownership relationship points to this symbol in the indexed graph.".into(),
            });
        }
        candidates.sort_by(|a, b| {
            b.outgoing_edges
                .cmp(&a.outgoing_edges)
                .then_with(|| a.node.id.cmp(&b.node.id))
        });
        let total = candidates.len();
        candidates.truncate(request.limit);
        Ok(DeadCodeReport {
            candidates,
            total_candidates: total,
            returned: total.min(request.limit),
            semantics: "Candidates only. Dynamic dispatch, reflection, framework entry points, generated code and incomplete language resolution can produce false positives; never delete automatically.".into(),
        })
    }
}

fn is_common_entrypoint(label: &str, path: &str) -> bool {
    matches!(
        label,
        "main" | "init" | "run" | "start" | "register" | "bootstrap"
    ) || path.ends_with("/main.rs")
        || path.ends_with("/main.go")
        || path.ends_with("/__init__.py")
        || path.ends_with("/index.ts")
        || path.ends_with("/index.js")
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactRequest {
    pub repo: String,
    pub files: Vec<String>,
    #[serde(default = "impact_depth")]
    pub max_depth: usize,
    #[serde(default = "impact_nodes")]
    pub max_nodes: usize,
}
fn impact_depth() -> usize {
    3
}
fn impact_nodes() -> usize {
    200
}

#[derive(Debug, Clone, Serialize)]
pub struct ImpactItem {
    pub node: Node,
    pub depth: usize,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ImpactReport {
    pub repo: String,
    pub files: Vec<String>,
    pub changed_nodes: Vec<Node>,
    pub impacted: Vec<ImpactItem>,
    pub missing_files: Vec<String>,
    pub direct_impacts: usize,
    pub cross_repository_impacts: usize,
    pub risk: String,
    pub risk_score: usize,
    pub truncated: bool,
    pub semantics: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeadCodeRequest {
    pub repo: Option<String>,
    #[serde(default = "dead_limit")]
    pub limit: usize,
}
fn dead_limit() -> usize {
    100
}
#[derive(Debug, Clone, Serialize)]
pub struct DeadCodeCandidate {
    pub node: Node,
    pub incoming_non_ownership_edges: usize,
    pub outgoing_edges: usize,
    pub confidence: String,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct DeadCodeReport {
    pub candidates: Vec<DeadCodeCandidate>,
    pub total_candidates: usize,
    pub returned: usize,
    pub semantics: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Graph {
        let input = Input {
            repo: "core".into(),
            files: BTreeMap::from([("src/a.rs".into(), "fn a() {}\nfn b() {}\n".into())]),
            symbols: vec![
                CodeSymbol {
                    key: 1,
                    name: "a".into(),
                    qualified_name: "a".into(),
                    signature: "fn a()".into(),
                    path: "src/a.rs".into(),
                    start_line: 1,
                    end_line: 1,
                },
                CodeSymbol {
                    key: 2,
                    name: "b".into(),
                    qualified_name: "b".into(),
                    signature: "fn b()".into(),
                    path: "src/a.rs".into(),
                    start_line: 2,
                    end_line: 2,
                },
            ],
            edges: vec![CodeEdge {
                from: 1,
                to: 2,
                relation: "Calls".into(),
            }],
            ..Input::default()
        };
        links::build(&input).unwrap()
    }
    #[test]
    fn impact_walks_reverse_dependencies() {
        let graph = sample();
        let index = AnalysisIndex::new(&graph);
        let report = index
            .impact(
                &graph,
                &ImpactRequest {
                    repo: "core".into(),
                    files: vec!["src/a.rs".into()],
                    max_depth: 2,
                    max_nodes: 20,
                },
            )
            .unwrap();
        assert_eq!(report.missing_files.len(), 0);
        assert_eq!(report.changed_nodes.len(), 3); // file + two symbols
    }
    #[test]
    fn dead_code_is_conservative_candidate_only() {
        let graph = sample();
        let index = AnalysisIndex::new(&graph);
        let report = index
            .dead_code(
                &graph,
                &DeadCodeRequest {
                    repo: Some("core".into()),
                    limit: 10,
                },
            )
            .unwrap();
        assert!(report.candidates.iter().any(|c| c.node.label == "a"));
        assert!(!report.candidates.iter().any(|c| c.node.label == "b"));
    }
}
