//! Broader T06 lexical evaluation: 21 labeled topic queries across domains and languages.
//! The selected reranker remains test-local on this investigation branch.

use codanna::Settings;
use codanna::indexing::facade::IndexFacade;
use codanna::storage::SearchResult;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy)]
struct Case {
    id: &'static str,
    path: &'static str,
    name: &'static str,
    doc: &'static str,
    query: &'static str,
}

const CASES: &[Case] = &[
    Case {
        id: "calendar-settings",
        path: "calendar/presentation.ts",
        name: "useAccountPresentation",
        doc: "Calendar settings account first day week preferences.",
        query: "calendar settings account first day week preferences",
    },
    Case {
        id: "integration-binding",
        path: "integration/binding.ts",
        name: "createIntegrationBinding",
        doc: "Integration binding creation GitHub subscription development activity eligible webhook targets.",
        query: "integration binding creation github subscription development activity eligible webhook targets",
    },
    Case {
        id: "workspace-preferences",
        path: "workspace/preferences.ts",
        name: "resolveWorkspacePreferences",
        doc: "Workspace default locale timezone calendar settings inheritance.",
        query: "workspace default locale timezone calendar settings inheritance",
    },
    Case {
        id: "billing-proration",
        path: "billing/proration.ts",
        name: "previewPlanChange",
        doc: "Subscription billing proration invoice preview plan change.",
        query: "subscription billing proration invoice preview plan change",
    },
    Case {
        id: "notification-digest",
        path: "notifications/digest.ts",
        name: "buildNotificationDigest",
        doc: "Notification digest unread mention email preference delivery.",
        query: "notification digest unread mention email preference delivery",
    },
    Case {
        id: "search-generation",
        path: "search/rebuild.ts",
        name: "rebuildSearchGeneration",
        doc: "Rebuild search index stale document generation replacement.",
        query: "rebuild search index stale document generation replacement",
    },
    Case {
        id: "project-access",
        path: "permissions/access.ts",
        name: "resolveProjectAccess",
        doc: "Workspace role permission inherited project access policy.",
        query: "workspace role permission inherited project access policy",
    },
    Case {
        id: "multipart-upload",
        path: "uploads/multipart.ts",
        name: "uploadMultipartAttachment",
        doc: "Multipart attachment upload checksum retry storage completion.",
        query: "multipart attachment upload checksum retry storage completion",
    },
    Case {
        id: "dependency-cycle",
        path: "tasks/dependencies.ts",
        name: "validateTaskDependencies",
        doc: "Task dependency cycle blocker scheduling validation.",
        query: "task dependency cycle blocker scheduling validation",
    },
    Case {
        id: "rate-limit",
        path: "api/rate_limit.ts",
        name: "enforceRequestQuota",
        doc: "API rate limit request quota retry after response.",
        query: "api rate limit request quota retry after response",
    },
    Case {
        id: "tool-latency",
        path: "observability/tool_latency.ts",
        name: "recordToolLatency",
        doc: "Trace latency span tool call first token timing.",
        query: "trace latency span tool call first token timing",
    },
    Case {
        id: "cache-invalidation",
        path: "cache/workspace.ts",
        name: "invalidateWorkspaceSettingsCache",
        doc: "Redis cache invalidation workspace settings version refresh.",
        query: "redis cache invalidation workspace settings version refresh",
    },
    Case {
        id: "offline-mutations",
        path: "mobile/offline.ts",
        name: "reconcileOfflineMutations",
        doc: "Offline sync optimistic mutation queue conflict reconciliation.",
        query: "offline sync optimistic mutation queue conflict reconciliation",
    },
    Case {
        id: "agent-approval",
        path: "agents/approval.ts",
        name: "requireToolApproval",
        doc: "Tool approval human loop mutation confirmation policy.",
        query: "tool approval human loop mutation confirmation policy",
    },
    // Holdout starts here (one third of the corpus).
    Case {
        id: "session-rotation",
        path: "auth/session.rs",
        name: "refresh_session_token",
        doc: "Refresh session token rotation expiry authentication security.",
        query: "refresh session token rotation expiry authentication security",
    },
    Case {
        id: "request-origin",
        path: "security/origin.rs",
        name: "validate_request_origin",
        doc: "CSRF origin cookie same site request validation security.",
        query: "csrf origin cookie same site request validation security",
    },
    Case {
        id: "realtime-replay",
        path: "realtime/replay.go",
        name: "ReplayRealtimeEvents",
        doc: "Websocket reconnect missed event cursor replay realtime delivery.",
        query: "websocket reconnect missed event cursor replay realtime delivery",
    },
    Case {
        id: "transaction-retry",
        path: "database/retry.go",
        name: "RetrySerializableTransaction",
        doc: "Transaction retry serialization deadlock database backoff.",
        query: "transaction retry serialization deadlock database backoff",
    },
    Case {
        id: "deadline-timezone",
        path: "calendar/deadline.rs",
        name: "localize_calendar_deadline",
        doc: "Timezone daylight saving calendar deadline localization.",
        query: "timezone daylight saving calendar deadline localization",
    },
    Case {
        id: "project-route",
        path: "routes/project.go",
        name: "LoadProjectRoute",
        doc: "Route loader workspace project redirect permission navigation.",
        query: "route loader workspace project redirect permission navigation",
    },
    Case {
        id: "task-export",
        path: "exports/tasks.ts",
        name: "exportTasksCsv",
        doc: "CSV export task columns assignee status filter.",
        query: "csv export task columns assignee status filter",
    },
];

const HOLDOUT_START: usize = 14;

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

        // Repeated one-concept names mimic the production crowding symptom.
        for i in 0..32 {
            for (family, name) in [
                ("settings", "settings"),
                ("subscription", "subscription"),
                ("development", "development"),
                ("workspace", "workspace"),
                ("retry", "retry"),
                ("cache", "cache"),
                ("route", "route"),
            ] {
                let path = root.join(format!("generic/{family}_{i:02}.ts"));
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(
                    path,
                    format!(
                        "/** Generic {family} helper {i}. */\nexport function {name}() {{ return {i}; }}\n"
                    ),
                )
                .unwrap();
            }
        }

        for case in CASES {
            write_case(&root, *case);
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
        raw_symbol_ranking::raw_search(&self.index, query, limit, None)
    }
}

fn write_case(root: &Path, case: Case) {
    let path = root.join(case.path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let source = if case.path.ends_with(".rs") {
        format!("/// {}\npub fn {}() {{}}\n", case.doc, case.name)
    } else if case.path.ends_with(".go") {
        format!("// {}\nfunc {}() {{}}\n", case.doc, case.name)
    } else {
        format!(
            "/** {} */\nexport function {}() {{ return 1; }}\n",
            case.doc, case.name
        )
    };
    std::fs::write(path, source).unwrap();
}

fn terms(query: &str) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for term in query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 3)
    {
        terms.insert(term);
    }
    terms.into_iter().collect()
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

fn r1(mut candidates: Vec<SearchResult>, query: &str, limit: usize) -> Vec<SearchResult> {
    let terms = terms(query);
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

fn reciprocal_rank(results: &[SearchResult], expected: &str) -> f64 {
    results
        .iter()
        .position(|result| result.name == expected)
        .map(|rank| 1.0 / (rank as f64 + 1.0))
        .unwrap_or(0.0)
}

fn evaluate(fixture: &Fixture, cases: &[Case]) -> ((usize, f64), (usize, f64)) {
    let mut r0_hits = 0;
    let mut r0_rr = 0.0;
    let mut r1_hits = 0;
    let mut r1_rr = 0.0;

    for case in cases {
        let baseline = fixture.raw(case.query, 5);
        let reranked = r1(fixture.raw(case.query, 80), case.query, 5);
        let r0_rank = baseline
            .iter()
            .position(|result| result.name == case.name)
            .map(|rank| rank + 1);
        let r1_rank = reranked
            .iter()
            .position(|result| result.name == case.name)
            .map(|rank| rank + 1);
        let candidate_rank_200 = fixture
            .raw(case.query, 200)
            .iter()
            .position(|result| result.name == case.name)
            .map(|rank| rank + 1);
        println!(
            "case={} expected={} r0_rank={r0_rank:?} r1_rank={r1_rank:?} candidate_rank_200={candidate_rank_200:?}",
            case.id, case.name
        );
        assert!(
            candidate_rank_200.is_some(),
            "labeled owner {} is absent even from the bounded 200-candidate pool",
            case.name
        );
        r0_hits += usize::from(r0_rank.is_some());
        r0_rr += reciprocal_rank(&baseline, case.name);
        r1_hits += usize::from(r1_rank.is_some());
        r1_rr += reciprocal_rank(&reranked, case.name);
    }
    (
        (r0_hits, r0_rr / cases.len() as f64),
        (r1_hits, r1_rr / cases.len() as f64),
    )
}

#[test]
fn broader_tuning_and_holdout_metrics_meet_the_proposed_t06_floor() {
    let fixture = Fixture::new();
    let (r0_all, r1_all) = evaluate(&fixture, CASES);
    let (r0_holdout, r1_holdout) = evaluate(&fixture, &CASES[HOLDOUT_START..]);

    println!(
        "all: r0_hit5={}/{} r0_mrr5={:.3} r1_hit5={}/{} r1_mrr5={:.3}; holdout: r0_hit5={}/{} r0_mrr5={:.3} r1_hit5={}/{} r1_mrr5={:.3}",
        r0_all.0,
        CASES.len(),
        r0_all.1,
        r1_all.0,
        CASES.len(),
        r1_all.1,
        r0_holdout.0,
        CASES.len() - HOLDOUT_START,
        r0_holdout.1,
        r1_holdout.0,
        CASES.len() - HOLDOUT_START,
        r1_holdout.1,
    );

    assert!(
        r1_all.0 as f64 / CASES.len() as f64 >= 0.90,
        "R1 Hit@5 must meet the proposed 0.90 floor"
    );
    assert!(
        r1_all.1 >= 0.75,
        "R1 MRR@5 must meet the proposed 0.75 floor"
    );
    assert!(
        r1_holdout.0 as f64 / (CASES.len() - HOLDOUT_START) as f64 >= 0.85,
        "holdout Hit@5 must not collapse"
    );
    assert!(r1_holdout.1 >= 0.70, "holdout MRR@5 must remain useful");
    assert!(r1_all.0 >= r0_all.0);
    assert!(r1_holdout.0 >= r0_holdout.0);
}

#[test]
fn cross_language_identifier_controls_remain_exact() {
    let fixture = Fixture::new();
    for name in [
        "refresh_session_token",
        "ReplayRealtimeEvents",
        "RetrySerializableTransaction",
        "exportTasksCsv",
    ] {
        let results = fixture.raw(name, 5);
        assert_eq!(
            results.first().map(|result| result.name.as_str()),
            Some(name)
        );
    }
}
