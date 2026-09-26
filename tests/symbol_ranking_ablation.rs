//! Code-symbol ranking ablations for the calendar investigation.
//! Synthetic local sources only; no semantic model or production index.

use codanna::Settings;
use codanna::indexing::facade::IndexFacade;
use codanna::storage::SearchResult;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

#[path = "support/raw_symbol_ranking.rs"]
mod raw_symbol_ranking;

struct Fixture {
    _temp: tempfile::TempDir,
    index: IndexFacade,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("src");
        std::fs::create_dir_all(&root).unwrap();

        // High-frequency exact-name distractors model the real Assign symptom:
        // dozens of local "settings", "subscription", "development" symbols.
        for i in 0..48 {
            write_ts(
                &root,
                &format!("generic/settings_{i:02}.ts"),
                &format!(
                    "/** Generic application settings holder {i}. */\nexport function settings() {{ return {i}; }}\n"
                ),
            );
        }
        for i in 0..28 {
            write_ts(
                &root,
                &format!("generic/subscription_{i:02}.ts"),
                &format!(
                    "/** Generic subscription state {i}. */\nexport function subscription() {{ return {i}; }}\n"
                ),
            );
        }
        for i in 0..24 {
            write_ts(
                &root,
                &format!("generic/development_{i:02}.ts"),
                &format!(
                    "/** Generic development flag {i}. */\nexport function development() {{ return {i}; }}\n"
                ),
            );
        }
        for i in 0..16 {
            write_ts(
                &root,
                &format!("generic/feature_{i:02}.ts"),
                &format!(
                    "/** Generic feature content {i}. */\nexport function featureContent() {{ return {i}; }}\n"
                ),
            );
        }

        for (path, body) in [
            (
                "calendar/presentation.ts",
                "/** Calendar settings apply account first day of week preferences. */\nexport function useAccountPresentation() { return 1; }\n",
            ),
            (
                "calendar/module.ts",
                "/** Shared calendar feature module for workspace calendar views. */\nexport function calendarModule() { return 1; }\n",
            ),
            (
                "integration/binding.ts",
                "/** Create integration binding for GitHub subscription development activity and eligible webhook targets. */\nexport function createIntegrationBinding() { return 1; }\n",
            ),
            (
                "account/current.ts",
                "/** Backend resolves account first day of week inheritance from workspace default in GET me. */\nexport function getCurrentAccount() { return 1; }\n",
            ),
            (
                "workspace/preferences.ts",
                "/** Workspace default locale timezone and calendar settings inheritance. */\nexport function resolveWorkspacePreferences() { return 1; }\n",
            ),
            (
                "upload/policy.ts",
                "/** Attachment upload policy enforces the configured mebibyte limit. */\nexport function enforceUploadPolicy() { return 1; }\n",
            ),
            (
                "delivery/retry.ts",
                "/** Delivery retry backoff prevents duplicate processing after acknowledgement loss. */\nexport function retryDeliveryAfterAckLoss() { return 1; }\n",
            ),
        ] {
            write_ts(&root, path, body);
        }

        let mut settings = Settings {
            workspace_root: Some(temp.path().to_path_buf()),
            index_path: temp.path().join("index"),
            ..Default::default()
        };
        settings.semantic_search.enabled = false;
        settings.add_indexed_path(root.clone()).unwrap();

        let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
        index.index_directory(&root, true).unwrap();
        Self { _temp: temp, index }
    }

    fn raw(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        raw_symbol_ranking::raw_search(&self.index, query, limit, Some("typescript"))
    }
}

fn write_ts(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn rank(results: &[SearchResult], expected: &str) -> Option<usize> {
    results
        .iter()
        .position(|result| result.name == expected)
        .map(|index| index + 1)
}

fn simple_discovery_terms(query: &str) -> Option<Vec<String>> {
    if query.chars().any(|c| {
        matches!(
            c,
            ':' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | '~' | '*' | '?' | '\\' | '/'
        )
    }) || query
        .split_whitespace()
        .any(|word| matches!(word, "AND" | "OR" | "NOT"))
    {
        return None;
    }
    let terms: BTreeSet<_> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 3)
        .collect();
    (terms.len() >= 2).then(|| terms.into_iter().collect())
}

fn coverage(result: &SearchResult, terms: &[String]) -> usize {
    let mut searchable = result.name.to_lowercase();
    for value in [
        result.doc_comment.as_deref(),
        result.signature.as_deref(),
        result.context.as_deref(),
        Some(result.module_path.as_str()),
        Some(result.file_path.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        searchable.push(' ');
        searchable.push_str(&value.to_lowercase());
    }
    terms
        .iter()
        .filter(|term| searchable.contains(term.as_str()))
        .count()
}

fn rerank_r1(query: &str, mut candidates: Vec<SearchResult>, limit: usize) -> Vec<SearchResult> {
    let Some(terms) = simple_discovery_terms(query) else {
        candidates.truncate(limit);
        return candidates;
    };
    candidates.sort_by(|a, b| {
        coverage(b, &terms)
            .cmp(&coverage(a, &terms))
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
    });
    candidates.truncate(limit);
    candidates
}

#[derive(Clone, Copy)]
struct Case {
    id: &'static str,
    query: &'static str,
    expected: &'static str,
}

const TOPIC_CASES: &[Case] = &[
    Case {
        id: "calendar-settings",
        query: "calendar settings",
        expected: "useAccountPresentation",
    },
    Case {
        id: "calendar-feature",
        query: "calendar feature",
        expected: "calendarModule",
    },
    Case {
        id: "integration-binding",
        query: "integration binding creation subscription github development activity eligible webhook targets",
        expected: "createIntegrationBinding",
    },
    Case {
        id: "account-inheritance",
        query: "backend resolves account first day of week inheritance from workspace default GET me",
        expected: "getCurrentAccount",
    },
    Case {
        id: "workspace-preferences",
        query: "workspace default locale timezone calendar settings inheritance",
        expected: "resolveWorkspacePreferences",
    },
    Case {
        id: "upload-policy",
        query: "attachment upload policy configured mebibyte limit",
        expected: "enforceUploadPolicy",
    },
    Case {
        id: "retry-policy",
        query: "delivery retry acknowledgement loss duplicate processing backoff",
        expected: "retryDeliveryAfterAckLoss",
    },
];

#[test]
fn current_top_k_loses_multi_concept_owners_that_exist_in_the_bounded_candidate_pool() {
    let fixture = Fixture::new();
    let mut top5_misses = 0;
    let mut recoverable = 0;
    let mut rows = Vec::new();

    for case in TOPIC_CASES {
        let top5 = fixture.raw(case.query, 5);
        let top200 = fixture.raw(case.query, 200);
        let r0 = rank(&top5, case.expected);
        let pool_rank = rank(&top200, case.expected);
        if r0.is_none() {
            top5_misses += 1;
        }
        if r0.is_none() && pool_rank.is_some() {
            recoverable += 1;
        }
        rows.push(json!({
            "id": case.id,
            "r0_rank_at_5": r0,
            "candidate_rank_at_200": pool_rank,
            "top5": top5.iter().map(|hit| hit.name.as_str()).collect::<Vec<_>>(),
        }));
    }

    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
    assert!(
        top5_misses >= 2,
        "fixture must reproduce noisy top-k discovery"
    );
    assert!(
        recoverable >= 2,
        "fixture must distinguish rank loss from absent candidates"
    );
}

#[test]
fn bounded_distinct_term_coverage_ablation_measures_candidate_pool_size_before_selection() {
    let fixture = Fixture::new();
    let mut r0_hits = 0;
    let mut r1x4_hits = 0;
    let mut r1x8_hits = 0;
    let mut r1x16_hits = 0;
    let mut r1cap_hits = 0;

    for case in TOPIC_CASES {
        let r0 = fixture.raw(case.query, 5);
        r0_hits += usize::from(rank(&r0, case.expected).is_some());

        let four = rerank_r1(case.query, fixture.raw(case.query, 20), 5);
        r1x4_hits += usize::from(rank(&four, case.expected).is_some());

        let eight = rerank_r1(case.query, fixture.raw(case.query, 40), 5);
        r1x8_hits += usize::from(rank(&eight, case.expected).is_some());

        let sixteen = rerank_r1(case.query, fixture.raw(case.query, 80), 5);
        r1x16_hits += usize::from(rank(&sixteen, case.expected).is_some());

        let capped = rerank_r1(case.query, fixture.raw(case.query, 200), 5);
        r1cap_hits += usize::from(rank(&capped, case.expected).is_some());
    }

    println!(
        "topic_hit_at_5: r0={r0_hits}/{} r1x4={r1x4_hits}/{} r1x8={r1x8_hits}/{} r1x16={r1x16_hits}/{} r1cap200={r1cap_hits}/{}",
        TOPIC_CASES.len(),
        TOPIC_CASES.len(),
        TOPIC_CASES.len(),
        TOPIC_CASES.len(),
        TOPIC_CASES.len()
    );
    assert!(r1x4_hits >= r0_hits);
    assert!(r1x8_hits >= r1x4_hits);
    assert!(
        r1x8_hits < TOPIC_CASES.len() - 1,
        "fixture must retain evidence that 8x overfetch is insufficient"
    );
    assert!(r1x16_hits >= r1x8_hits);
    assert!(r1cap_hits >= r1x16_hits);
    assert!(
        r1cap_hits >= TOPIC_CASES.len() - 1,
        "a bounded 200-candidate pool should show whether ranking rather than recall is the blocker"
    );
}

#[test]
fn identifier_single_term_absent_and_explicit_query_controls_are_preserved_by_the_ablation() {
    let fixture = Fixture::new();

    for query in [
        "useAccountPresentation",
        "calendarModule",
        "createIntegrationBinding",
        "getCurrentAccount",
    ] {
        let raw = fixture.raw(query, 5);
        assert_eq!(raw.first().map(|hit| hit.name.as_str()), Some(query));
        let reranked = rerank_r1(query, raw.clone(), 5);
        assert_eq!(
            reranked
                .iter()
                .map(|hit| (&hit.name, hit.score))
                .collect::<Vec<_>>(),
            raw.iter()
                .map(|hit| (&hit.name, hit.score))
                .collect::<Vec<_>>()
        );
    }

    assert!(fixture.raw("qzvnoexist75391", 5).is_empty());

    // Explicit Tantivy syntax is intentionally outside R1. Its ordering is
    // inherited unchanged so the discovery experiment cannot rewrite semantics.
    for query in ["calendar AND settings", "\"calendar settings\""] {
        let raw = fixture.raw(query, 20);
        let reranked = rerank_r1(query, raw.clone(), 20);
        assert_eq!(
            reranked
                .iter()
                .map(|hit| (&hit.name, hit.score))
                .collect::<Vec<_>>(),
            raw.iter()
                .map(|hit| (&hit.name, hit.score))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn filters_apply_before_any_test_local_reranking() {
    let fixture = Fixture::new();
    let hits = fixture
        .index
        .search("calendar settings", 40, None, None, Some("rust"))
        .unwrap();
    assert!(
        hits.is_empty(),
        "language filter must not be filled after ranking"
    );
}

#[test]
fn candidate_pool_cost_is_measured_without_a_latency_gate() {
    use std::time::Instant;

    let fixture = Fixture::new();
    let mut small_elapsed = std::time::Duration::ZERO;
    let mut expanded_elapsed = std::time::Duration::ZERO;
    let mut small_results = 0usize;
    let mut expanded_results = 0usize;
    let mut small_json_bytes = 0usize;
    let mut expanded_json_bytes = 0usize;

    // Measure the production candidate path, not the complete raw-score drain
    // used by the historical ablations. Timing is fixture evidence, not a gate.
    for _ in 0..10 {
        for case in TOPIC_CASES {
            let start = Instant::now();
            let small = fixture
                .index
                .search(case.query, 5, None, None, Some("typescript"))
                .unwrap();
            small_elapsed += start.elapsed();
            small_results += small.len();
            small_json_bytes += serde_json::to_vec(&small).unwrap().len();

            let start = Instant::now();
            let expanded = fixture
                .index
                .search(case.query, 128, None, None, Some("typescript"))
                .unwrap();
            expanded_elapsed += start.elapsed();
            expanded_results += expanded.len();
            expanded_json_bytes += serde_json::to_vec(&expanded).unwrap().len();
        }
    }

    println!(
        "production_candidate_cost_fixture: queries={} limit5_results={} limit128_results={} limit5_json_bytes={} limit128_json_bytes={} limit5_elapsed_us={} limit128_elapsed_us={}",
        TOPIC_CASES.len() * 10,
        small_results,
        expanded_results,
        small_json_bytes,
        expanded_json_bytes,
        small_elapsed.as_micros(),
        expanded_elapsed.as_micros(),
    );
    assert!(expanded_results >= small_results);
    assert!(expanded_json_bytes >= small_json_bytes);
}
