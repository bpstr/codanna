//! Broader T06 runtime acceptance: 21 labeled topic queries across domains and languages.
//! This exercises the selected bounded discovery policy directly.

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
        self.index.search(query, limit, None, None, None).unwrap()
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

fn reciprocal_rank(results: &[SearchResult], expected: &str) -> f64 {
    results
        .iter()
        .position(|result| result.name == expected)
        .map(|rank| 1.0 / (rank as f64 + 1.0))
        .unwrap_or(0.0)
}

fn evaluate_runtime(fixture: &Fixture, cases: &[Case]) -> (usize, f64) {
    let mut hits = 0;
    let mut reciprocal_rank_sum = 0.0;
    for case in cases {
        let results = fixture.raw(case.query, 5);
        let rr = reciprocal_rank(&results, case.name);
        let rank = results
            .iter()
            .position(|result| result.name == case.name)
            .map(|rank| rank + 1);
        println!("case={} expected={} runtime_rank={rank:?}", case.id, case.name);
        hits += usize::from(rank.is_some());
        reciprocal_rank_sum += rr;
    }
    (hits, reciprocal_rank_sum / cases.len() as f64)
}

#[test]
fn runtime_tuning_and_holdout_metrics_meet_the_t06_floor() {
    let fixture = Fixture::new();
    let all = evaluate_runtime(&fixture, CASES);
    let holdout = evaluate_runtime(&fixture, &CASES[HOLDOUT_START..]);

    println!(
        "runtime: hit5={}/{} mrr5={:.3}; holdout: hit5={}/{} mrr5={:.3}",
        all.0,
        CASES.len(),
        all.1,
        holdout.0,
        CASES.len() - HOLDOUT_START,
        holdout.1,
    );

    assert!(
        all.0 as f64 / CASES.len() as f64 >= 0.90,
        "runtime Hit@5 must meet the proposed 0.90 floor"
    );
    assert!(all.1 >= 0.75, "runtime MRR@5 must meet the proposed 0.75 floor");
    assert!(
        holdout.0 as f64 / (CASES.len() - HOLDOUT_START) as f64 >= 0.85,
        "holdout Hit@5 must not collapse"
    );
    assert!(holdout.1 >= 0.70, "holdout MRR@5 must remain useful");
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
