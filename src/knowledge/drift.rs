//! Freshness and drift checks distinguish definite broken references from review signals.
use crate::knowledge::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Review,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub repo: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub errors: usize,
    pub reviews: usize,
    pub infos: usize,
    pub clean: bool,
}

pub fn check(graph: &Graph, roots: &BTreeMap<String, String>) -> Result<Report> {
    graph.validate()?;
    let mut findings = Vec::new();
    for (repo, stamp) in &graph.repositories {
        let Some(root) = roots.get(repo) else {
            findings.push(Finding {
                severity: Severity::Info,
                code: "ROOT_UNAVAILABLE".into(),
                message: "repository root was not supplied; live freshness was not checked".into(),
                repo: repo.clone(),
                path: String::new(),
            });
            continue;
        };
        let root = Path::new(root).canonicalize()?;
        let live_revision = io::revision(&root);
        if stamp.revision.is_some() && live_revision != stamp.revision {
            findings.push(Finding {
                severity: Severity::Review,
                code: "REVISION_CHANGED".into(),
                message: format!(
                    "indexed revision {:?}, live revision {:?}",
                    stamp.revision, live_revision
                ),
                repo: repo.clone(),
                path: String::new(),
            });
        }
        for (path, indexed_hash) in &stamp.files {
            let live = match io::source_path(&root, path) {
                Ok(path) => path,
                Err(_) => {
                    findings.push(Finding {
                        severity: Severity::Error,
                        code: "SOURCE_MISSING_OR_ESCAPED".into(),
                        message:
                            "indexed source is missing or no longer resolves inside repository root"
                                .into(),
                        repo: repo.clone(),
                        path: path.clone(),
                    });
                    continue;
                }
            };
            let bytes = io::read_bounded(&live, MAX_FILE_BYTES as u64)?;
            let live_hash = digest(&bytes);
            if &live_hash != indexed_hash {
                findings.push(Finding { severity: Severity::Review, code: "SOURCE_CHANGED".into(), message: "source content differs from indexed evidence; rebuild knowledge before relying on links".into(), repo: repo.clone(), path: path.clone() });
            }
        }
    }
    // A local link that was explicitly emitted as unresolved is a definite broken reference only for that extractor method.
    for unresolved in &graph.unresolved {
        if unresolved.reason == "unresolved_local_link" {
            findings.push(Finding {
                severity: Severity::Error,
                code: "BROKEN_LOCAL_LINK".into(),
                message: format!("unresolved local link: {}", unresolved.reference),
                repo: unresolved.evidence.repo.clone(),
                path: unresolved.evidence.path.clone(),
            });
        } else if unresolved.reason == "unresolved_decision" {
            findings.push(Finding {
                severity: Severity::Review,
                code: "UNRESOLVED_DECISION_REFERENCE".into(),
                message: format!("decision reference needs review: {}", unresolved.reference),
                repo: unresolved.evidence.repo.clone(),
                path: unresolved.evidence.path.clone(),
            });
        } else if unresolved.reason.contains("ambiguous") {
            findings.push(Finding {
                severity: Severity::Review,
                code: "AMBIGUOUS_REFERENCE".into(),
                message: format!("ambiguous reference: {}", unresolved.reference),
                repo: unresolved.evidence.repo.clone(),
                path: unresolved.evidence.path.clone(),
            });
        }
    }
    findings.sort_by(|a, b| {
        (a.severity as u8, &a.repo, &a.path, &a.code, &a.message).cmp(&(
            b.severity as u8,
            &b.repo,
            &b.path,
            &b.code,
            &b.message,
        ))
    });
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let reviews = findings
        .iter()
        .filter(|f| f.severity == Severity::Review)
        .count();
    let infos = findings
        .iter()
        .filter(|f| f.severity == Severity::Info)
        .count();
    Ok(Report {
        clean: errors == 0 && reviews == 0,
        findings,
        errors,
        reviews,
        infos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_content_is_review_not_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "new").unwrap();
        let graph = Graph {
            repositories: BTreeMap::from([(
                "r".into(),
                Repository {
                    revision: None,
                    files: BTreeMap::from([("a.md".into(), digest(b"old"))]),
                },
            )]),
            nodes: BTreeMap::from([(
                identity("r", "Document", "a.md", ""),
                Node {
                    id: identity("r", "Document", "a.md", ""),
                    kind: Kind::Document,
                    label: "a.md".into(),
                    source: Span {
                        repo: "r".into(),
                        path: "a.md".into(),
                        start_line: 1,
                        end_line: 1,
                        hash: digest(b"old"),
                    },
                    excerpt: String::new(),
                },
            )]),
            ..Graph::default()
        };
        let report = check(
            &graph,
            &BTreeMap::from([("r".into(), dir.path().to_string_lossy().into_owned())]),
        )
        .unwrap();
        assert_eq!(report.errors, 0);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == "SOURCE_CHANGED" && f.severity == Severity::Review)
        );
    }
    #[test]
    fn missing_source_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let graph = Graph {
            repositories: BTreeMap::from([(
                "r".into(),
                Repository {
                    revision: None,
                    files: BTreeMap::from([("a.md".into(), digest(b"old"))]),
                },
            )]),
            nodes: BTreeMap::from([(
                identity("r", "Document", "a.md", ""),
                Node {
                    id: identity("r", "Document", "a.md", ""),
                    kind: Kind::Document,
                    label: "a.md".into(),
                    source: Span {
                        repo: "r".into(),
                        path: "a.md".into(),
                        start_line: 1,
                        end_line: 1,
                        hash: digest(b"old"),
                    },
                    excerpt: String::new(),
                },
            )]),
            ..Graph::default()
        };
        assert!(
            check(
                &graph,
                &BTreeMap::from([("r".into(), dir.path().to_string_lossy().into_owned())])
            )
            .unwrap()
            .errors
                > 0
        );
    }
}
