//! Evidence-backed knowledge beside, not inside, the symbol/vector indexes.
//! No model calls, networking, or source execution. See docs/knowledge.md.
pub mod io;
pub mod links;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_GRAPH_BYTES: u64 = 256 * 1024 * 1024;

pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes.as_ref()))
}

/// Length-delimited identities cannot collide through separators in user input.
pub fn identity(repo: &str, kind: &str, path: &str, key: &str) -> String {
    let mut hash = Sha256::new();
    for part in [repo, kind, path, key] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{repo}:{kind}:{}", hex::encode(hash.finalize()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    File,
    Symbol,
    Document,
    Section,
    Rationale,
    Test,
    Operation,
    Schema,
    Field,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// The source explicitly states this reference; not proof of runtime behavior.
    Explicit,
    /// A uniquely resolved structural reference imported from Codanna.
    Resolved,
    /// A suggestion only. Never a dependency or a CI error by itself.
    Candidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub repo: String,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: Kind,
    pub label: String,
    pub source: Span,
    /// Bounded excerpt, never instructions to an agent.
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub basis: Basis,
    pub evidence: Span,
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unresolved {
    pub from: String,
    pub reference: String,
    pub reason: String,
    pub candidates: Vec<String>,
    pub evidence: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub revision: Option<String>,
    /// Repository-relative paths and source hashes. No absolute host paths.
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    pub schema_version: u32,
    pub repositories: BTreeMap<String, Repository>,
    pub nodes: BTreeMap<String, Node>,
    pub edges: Vec<Edge>,
    pub unresolved: Vec<Unresolved>,
    pub limitations: Vec<String>,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            repositories: BTreeMap::new(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            unresolved: Vec::new(),
            limitations: Vec::new(),
        }
    }
}

impl Graph {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err("unsupported knowledge schema; rebuild the snapshot".into());
        }
        for (repo, stamp) in &self.repositories {
            validate_repo(repo)?;
            for (path, hash) in &stamp.files {
                validate_path(path)?;
                if !valid_hash(hash) {
                    return Err("invalid source hash".into());
                }
            }
        }
        for (id, node) in &self.nodes {
            if id != &node.id || id.is_empty() || node.label.len() > MAX_FILE_BYTES {
                return Err("invalid knowledge identity or label".into());
            }
            self.validate_span(&node.source)?;
        }
        for edge in &self.edges {
            if !self.nodes.contains_key(&edge.from) || !self.nodes.contains_key(&edge.to) {
                return Err("dangling knowledge edge".into());
            }
            self.validate_span(&edge.evidence)?;
        }
        for unresolved in &self.unresolved {
            if !self.nodes.contains_key(&unresolved.from)
                || unresolved
                    .candidates
                    .iter()
                    .any(|id| !self.nodes.contains_key(id))
            {
                return Err("dangling unresolved reference".into());
            }
            self.validate_span(&unresolved.evidence)?;
        }
        Ok(())
    }

    fn validate_span(&self, span: &Span) -> Result<()> {
        let hash = self
            .repositories
            .get(&span.repo)
            .and_then(|r| r.files.get(&span.path));
        if hash != Some(&span.hash) || span.start_line == 0 || span.end_line < span.start_line {
            return Err("invalid evidence span or source hash".into());
        }
        Ok(())
    }

    pub fn normalize(&mut self) {
        self.edges.sort_by(|a, b| {
            (
                &a.from,
                &a.to,
                &a.relation,
                &a.basis,
                &a.method,
                a.evidence.start_line,
            )
                .cmp(&(
                    &b.from,
                    &b.to,
                    &b.relation,
                    &b.basis,
                    &b.method,
                    b.evidence.start_line,
                ))
        });
        self.edges.dedup();
        self.unresolved.sort_by(|a, b| {
            (&a.from, &a.reference, &a.reason).cmp(&(&b.from, &b.reference, &b.reason))
        });
        self.unresolved.dedup();
        self.limitations.sort();
        self.limitations.dedup();
    }

    pub fn resolve(&self, query: &str) -> Result<&Node> {
        if let Some(node) = self.nodes.get(query) {
            return Ok(node);
        }
        let candidates: Vec<_> = self.nodes.values().filter(|n| n.label == query).collect();
        match candidates.as_slice() {
            [node] => Ok(node),
            [] => Err(format!("no entity matches {query:?}").into()),
            _ => Err(format!(
                "ambiguous entity {query:?}; use an id: {}",
                candidates
                    .iter()
                    .map(|n| n.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into()),
        }
    }
}

pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn validate_repo(repo: &str) -> Result<()> {
    if repo.is_empty()
        || repo.len() > 128
        || !repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(
            "repository id must contain 1..128 ASCII letters, digits, dots, dashes or underscores"
                .into(),
        );
    }
    Ok(())
}

pub fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains(['\\', ':', '\0'])
        || path.starts_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe repository-relative path: {path:?}").into());
    }
    Ok(())
}

pub fn excerpt(text: &str) -> String {
    text.chars().take(2048).collect()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodeSymbol {
    pub key: u64,
    pub name: String,
    pub qualified_name: String,
    pub signature: String,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodeEdge {
    pub from: u64,
    pub to: u64,
    pub relation: String,
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub repo: String,
    pub revision: Option<String>,
    pub files: BTreeMap<String, String>,
    pub symbols: Vec<CodeSymbol>,
    pub edges: Vec<CodeEdge>,
}
