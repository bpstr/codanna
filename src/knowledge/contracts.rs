//! Cross-repository contract linking without executing generators or trusting names alone.
use crate::knowledge::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Workspace {
    pub repositories: BTreeMap<String, String>,
    pub graph_paths: BTreeMap<String, String>,
    pub contracts: Vec<ContractSource>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractSource {
    pub repo: String,
    pub path: String,
}

pub fn merge(graphs: Vec<Graph>) -> Result<Graph> {
    let mut out = Graph::default();
    for graph in graphs {
        graph.validate()?;
        for (repo, stamp) in graph.repositories {
            if out.repositories.insert(repo.clone(), stamp).is_some() {
                return Err(format!("duplicate repository id in workspace: {repo}").into());
            }
        }
        for (id, node) in graph.nodes {
            if out.nodes.insert(id.clone(), node).is_some() {
                return Err("knowledge identity collision".into());
            }
        }
        out.edges.extend(graph.edges);
        out.unresolved.extend(graph.unresolved);
        out.limitations.extend(graph.limitations);
    }
    out.normalize();
    out.validate()?;
    Ok(out)
}

pub fn add_openapi(graph: &mut Graph, repo: &str, path: &str, text: &str) -> Result<()> {
    validate_repo(repo)?;
    validate_path(path)?;
    if text.len() > MAX_FILE_BYTES {
        return Err("OpenAPI document exceeds source limit".into());
    }
    let repo_stamp = graph
        .repositories
        .get_mut(repo)
        .ok_or("OpenAPI repository not present in workspace")?;
    let hash = digest(text.as_bytes());
    repo_stamp.files.insert(path.into(), hash.clone());
    let value: Value =
        serde_json::from_str(text).map_err(|_| "OpenAPI ingestion currently requires JSON")?;
    let object = value.as_object().ok_or("OpenAPI root must be an object")?;
    let version = object
        .get("openapi")
        .and_then(Value::as_str)
        .ok_or("missing openapi version")?;
    if !version.starts_with("3.") {
        return Err("only OpenAPI 3.x is supported".into());
    }
    let span = Span {
        repo: repo.into(),
        path: path.into(),
        start_line: 1,
        end_line: text.lines().count().max(1) as u32,
        hash,
    };
    let contract_id = identity(repo, "OpenAPI", path, "root");
    graph.nodes.insert(
        contract_id.clone(),
        Node {
            id: contract_id.clone(),
            kind: Kind::Document,
            label: format!("OpenAPI {path}"),
            source: span.clone(),
            excerpt: excerpt(text),
        },
    );
    if let Some(paths) = object.get("paths").and_then(Value::as_object) {
        for (route, methods) in paths {
            let Some(methods) = methods.as_object() else {
                continue;
            };
            for (method, operation) in methods {
                if !matches!(
                    method.as_str(),
                    "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace"
                ) {
                    continue;
                }
                let Some(operation) = operation.as_object() else {
                    continue;
                };
                let key = operation
                    .get("operationId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if key.is_empty() {
                    continue;
                }
                let id = identity(repo, "Operation", path, key);
                graph.nodes.insert(
                    id.clone(),
                    Node {
                        id: id.clone(),
                        kind: Kind::Operation,
                        label: key.into(),
                        source: span.clone(),
                        excerpt: format!("{} {}", method.to_uppercase(), route),
                    },
                );
                graph.edges.push(Edge {
                    from: contract_id.clone(),
                    to: id,
                    relation: "defines".into(),
                    basis: Basis::Explicit,
                    evidence: span.clone(),
                    method: "openapi_operation_id".into(),
                });
            }
        }
    }
    if let Some(schemas) = value
        .pointer("/components/schemas")
        .and_then(Value::as_object)
    {
        for (name, schema) in schemas {
            let schema_id = identity(repo, "Schema", path, name);
            graph.nodes.insert(
                schema_id.clone(),
                Node {
                    id: schema_id.clone(),
                    kind: Kind::Schema,
                    label: name.to_string(),
                    source: span.clone(),
                    excerpt: excerpt(&schema.to_string()),
                },
            );
            graph.edges.push(Edge {
                from: contract_id.clone(),
                to: schema_id.clone(),
                relation: "defines".into(),
                basis: Basis::Explicit,
                evidence: span.clone(),
                method: "openapi_schema".into(),
            });
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for name in properties.keys() {
                    let id = identity(repo, "Field", path, &format!("{schema_id}:{name}"));
                    graph.nodes.insert(
                        id.clone(),
                        Node {
                            id: id.clone(),
                            kind: Kind::Field,
                            label: name.to_string(),
                            source: span.clone(),
                            excerpt: format!("{name} field of {}", graph.nodes[&schema_id].label),
                        },
                    );
                    graph.edges.push(Edge {
                        from: schema_id.clone(),
                        to: id,
                        relation: "defines".into(),
                        basis: Basis::Explicit,
                        evidence: span.clone(),
                        method: "openapi_property".into(),
                    });
                }
            }
        }
    }
    link_contract_consumers(graph, repo, &span)?;
    graph.limitations.push("Cross-repository contract links require exact operation/schema names in indexed symbols or explicit generated-client metadata; unresolved ambiguity is retained.".into());
    graph.normalize();
    graph.validate()
}

fn link_contract_consumers(graph: &mut Graph, contract_repo: &str, evidence: &Span) -> Result<()> {
    let contract_nodes: Vec<_> = graph
        .nodes
        .values()
        .filter(|n| {
            n.source.repo == contract_repo
                && matches!(n.kind, Kind::Operation | Kind::Schema | Kind::Field)
        })
        .cloned()
        .collect();
    let symbols: Vec<_> = graph
        .nodes
        .values()
        .filter(|n| matches!(n.kind, Kind::Symbol | Kind::Test) && n.source.repo != contract_repo)
        .cloned()
        .collect();
    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for symbol in symbols {
        names
            .entry(symbol.label.clone())
            .or_default()
            .push(symbol.id);
    }
    let mut seen = BTreeSet::new();
    for contract in contract_nodes {
        if let Some(candidates) = names.get(&contract.label) {
            if candidates.len() == 1 {
                let key = (candidates[0].clone(), contract.id.clone());
                if seen.insert(key.clone()) {
                    graph.edges.push(Edge {
                        from: key.0,
                        to: key.1,
                        relation: "generated_from".into(),
                        basis: Basis::Resolved,
                        evidence: evidence.clone(),
                        method: "exact_contract_symbol_name".into(),
                    });
                }
            } else if candidates.len() > 1 {
                graph.unresolved.push(Unresolved {
                    from: contract.id.clone(),
                    reference: contract.label.clone(),
                    reason: "ambiguous_cross_repo_contract_consumer".into(),
                    candidates: candidates.clone(),
                    evidence: evidence.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_operation_links_across_repos() {
        let openapi =
            r#"{"openapi":"3.1.0","paths":{"/tasks/{id}":{"get":{"operationId":"getTask"}}}}"#;
        let core = links::build(&Input {
            repo: "core".into(),
            files: BTreeMap::from([("openapi.json".into(), openapi.into())]),
            ..Input::default()
        })
        .unwrap();
        let client = links::build(&Input {
            repo: "web".into(),
            files: BTreeMap::from([("src/api.ts".into(), "function getTask() {}".into())]),
            symbols: vec![CodeSymbol {
                key: 1,
                name: "getTask".into(),
                qualified_name: "getTask".into(),
                signature: "function getTask()".into(),
                path: "src/api.ts".into(),
                start_line: 1,
                end_line: 1,
            }],
            ..Input::default()
        })
        .unwrap();
        let mut graph = merge(vec![core, client]).unwrap();
        add_openapi(&mut graph, "core", "openapi.json", openapi).unwrap();
        assert!(graph.edges.iter().any(|e| e.relation == "generated_from" && e.method == "exact_contract_symbol_name"));
    }
    #[test]
    fn ambiguity_is_retained_not_guessed() {
        let openapi = r#"{"openapi":"3.1.0","paths":{"/tasks":{"get":{"operationId":"getTask"}}}}"#;
        let core = links::build(&Input {
            repo: "core".into(),
            files: BTreeMap::from([("openapi.json".into(), openapi.into())]),
            ..Input::default()
        })
        .unwrap();
        let client_a = links::build(&Input {
            repo: "web-a".into(),
            files: BTreeMap::from([("src/api.ts".into(), "a".into())]),
            symbols: vec![CodeSymbol {
                key: 1,
                name: "getTask".into(),
                qualified_name: "a::getTask".into(),
                signature: "a".into(),
                path: "src/api.ts".into(),
                start_line: 1,
                end_line: 1,
            }],
            ..Input::default()
        })
        .unwrap();
        let client_b = links::build(&Input {
            repo: "web-b".into(),
            files: BTreeMap::from([("src/api.ts".into(), "b".into())]),
            symbols: vec![CodeSymbol {
                key: 1,
                name: "getTask".into(),
                qualified_name: "b::getTask".into(),
                signature: "b".into(),
                path: "src/api.ts".into(),
                start_line: 1,
                end_line: 1,
            }],
            ..Input::default()
        })
        .unwrap();
        let mut graph = merge(vec![core, client_a, client_b]).unwrap();
        add_openapi(&mut graph, "core", "openapi.json", openapi).unwrap();
        assert!(graph.unresolved.iter().any(|u| u.reason
            == "ambiguous_cross_repo_contract_consumer"
            && u.candidates.len() == 2));
        assert!(!graph.edges.iter().any(|e| e.relation == "generated_from"));
    }
    #[test]
    fn duplicate_repo_merge_is_rejected() {
        let a = Graph {
            repositories: BTreeMap::from([("x".into(), Repository::default())]),
            ..Graph::default()
        };
        assert!(merge(vec![a.clone(), a]).is_err());
    }
}
