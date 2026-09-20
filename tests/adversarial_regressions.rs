//! Deterministic adversarial regressions across extraction, resolution, and indexing.
//! See contributing/retrieval/adversarial/README.md for scenarios and baseline results.
//! All data is synthetic; no embedding provider or model is used.

#[path = "adversarial/root/adversarial_root.rs"]
mod root_probes;

#[path = "adversarial/rust_go_python/parser_edge_cases.rs"]
mod parser_edge_cases;

#[path = "adversarial/index_lifecycle/review_index_lifecycle.rs"]
mod index_lifecycle;

#[path = "adversarial/search_graph/search_graph_review.rs"]
mod search_graph;

#[path = "adversarial/ts_php/regressions.rs"]
mod web_parser;

#[path = "adversarial/rust_go_python/pipeline_edge_cases.rs"]
mod resolver_stages;

#[path = "adversarial/rust_go_python/end_to_end.rs"]
mod language_end_to_end;
