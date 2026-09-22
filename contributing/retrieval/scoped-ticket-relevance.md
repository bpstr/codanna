# Combined ticket relevance and workspace scope

This #62 integration retains the histories of #61 (`c22eb98`), #57 (`43c15e1`)
and #59 (`c6f0a6a`) on a separate branch. No main/upstream merge, provider
inference, production reindex or embedding-policy change is involved.

## Executed combined baseline

[Integration run 35765600618](https://github.com/bpstr/codanna/actions/runs/35765600618)
merged only those pinned heads locally and passed **59 selected tests**,
formatting and strict all-target/all-feature Clippy. The tested source blobs
were committed through the GitHub app in merge commit
`c30be444312a1a88bccede8d3bf8069d413f0295`, preserving all dependency histories.

Same five frozen Rust files, 152 symbols, 20 questions and ten path/name owner
judgments; oracle SHA-256 remains
`129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`.

| Evidence surface | Operational | Paraphrase | Combined |
| --- | ---: | ---: | ---: |
| Direct owner Hit@5 | 7/10 | 1/10 | **8/20** |
| Owner among five direct plus at most six related results | 9/10 | 1/10 | **10/20** |

These outcomes were measured together, not summed across branches. Direct
MRR@5 is 0.433 operational and 0.100 paraphrase (0.267 combined). All twenty
direct result arrays remain unchanged by related-code opt-in. One explicit
reader-generation invalidation was discarded and logged in the combined
baseline; the existing bounded measurement retry succeeded. No missing owner,
empty candidate pool or ordinary error is retried.

The expanded surface has a larger evidence budget and is not improved Hit@5.
Paraphrases remain weak and the original 90% target remains unmet. These
source-inspected questions have influenced selection, so they are not an
independent holdout. Input availability is not semantic model quality.

## Scoped related implementation support

`include_related_code: true` now works with `code_path_prefix`. The scope is
resolved using #57's registered-file identity and the exact GraphView Searcher
used for relationship reads and endpoint hydration. Both seeds and targets
must qualify. Neither display-path shortening nor global-name fallback is used.

```json
{
  "query": "conversation timeout",
  "code_limit": 5,
  "code_path_prefix": "src",
  "include_related_code": true
}
```

A prefix includes its slash-descendants, not similarly named neighbors. An
exact file excludes `.backup` neighbors. Explicit `.` includes the configured
workspace, not separately indexed external checkouts. Legacy absolute file
registrations inside the workspace remain valid. An endpoint without a matching
file registration is excluded even when its display path looks in scope.

The existing one-hop, three-seed, 32-edge-per-seed and six-result budgets are
unchanged. The edge budget is checked before filtering; a large out-of-scope
neighborhood does not justify extra traversal to refill results. Related rows
still have no invented relevance score and do not rerank direct matches.

Scoped reports include `path_prefix`, per-seed `excluded_by_scope` counts and
matching text output. Missing symbol documents remain `unhydrated_edges`, not
scope exclusions. An unregistered/out-of-scope seed reports `seed_outside_scope`
without enumerating its edges. A completed scoped traversal with no eligible
related rows reports `empty_scoped_neighborhoods`; no direct matches still
reports `not_run_no_direct_matches`. Generation invalidation discards evidence.
Scopes are retrieval constraints, not authorization boundaries.

## Before/after scope verification

The first fixture attempt at `0eab5bf` failed to compile because it called a
private storage method. `128db47` switched only the fixture to the existing
public WriteStage; production visibility and expected results did not change.
That compilation failure is not a behavioral baseline.

[Run 35767443455](https://github.com/bpstr/codanna/actions/runs/35767443455)
started at `128db47b5d750d64e06cd9549aa42e95e03b2794` and verified the exact
original blobs before applying reviewed wiring. The same five public-handler
fixtures produced **1 passed / 4 failed before -> 5 passed / 0 failed after**.
Their SHA-256 was checked unchanged across both executions. The before-state
correctly rejected invalid arguments but did not implement scoped expansion.

All **66 selected candidate tests passed**, with no failures or ignored tests:

| Test group | Passed |
| --- | ---: |
| New scoped ticket handler and negative controls | 5 |
| New pinned graph registration/hydration and limits | 2 |
| Existing related-code library/schema/handler | 7 |
| Existing ticket fusion | 14 |
| Frozen direct and ticket relevance/compatibility | 3 |
| External-root scope contracts | 3 |
| Facade/MCP scope contracts | 7 |
| Scope and related-code real CLI contracts | 6 |
| Symbol ranking and previous runtime corpus | 6 |
| Scope inventory/snapshot/cache controls | 3 |
| Lexical normalization and coverage efficiency | 7 |
| Workspace ticket client and catalog/schema parity | 3 |

Formatting and strict all-target/all-feature Clippy passed. The scoped
candidate also reproduced direct 8/20 and direct-or-related 10/20 on the
original questions, with **zero generation retries** in this run. These are
not production latency, memory or semantic-quality measurements.

## Source provenance and retained checks

Verified runtime/test blobs are committed through the GitHub app. Both
one-time integration/wiring scripts and their write-enabled workflows are
removed. Retained `scoped-ticket-relevance.yml` checks committed sources only,
with read-only permissions. Its log checker requires all twenty per-query
outcomes, the exact oracle and known owners, and explicitly retains
`original_quality_gate_passed: false`. Final-head execution is distinct from
the staged evidence above and is recorded on #62 when it completes.

| File | Verified SHA-256 |
| --- | --- |
| `src/storage/tantivy/graph.rs` | `9fc7f86706c43895c31c964e769b2c83e53bcaae1284d91968dae50f64ec7993` |
| `src/storage/tantivy/graph_scope.rs` | `22a163593ae36a8f6db578ee9c58d3212f9965718eb0979c90686e02b446d429` |
| `src/mcp/tools/ticket_related.rs` | `e06cc572fb89e4966148c1abae8dc8f71d332a5d9228cc33359fe246413c2975` |
| `src/mcp/tools/ticket_related_tests.rs` | `312d8ca2c2a3927171b0c4eebf9e640abf58f67c566e219fd16b6f0ff4859b05` |
| `tests/ticket_related_cli.rs` | `563c642f4f1e7a52550768261cd9b66846ff6f28a8af79993e67305dfa79b405` |
| `tests/scoped_ticket_related.rs` | `258731203c4714c5d67c1ac99e00d3d1e764735fe9853ea06f7758f5a904fdd1` |

## Remaining boundaries

Only the lexical/scope/related-code branches are combined here, not all #43
work. Body representations and cache-policy/preflight branches still need
separate integration. Scoped semantic retrieval remains explicitly unsupported;
requesting related code does not opt into an embedding provider. Existing
document retrieval retains its own configured backend policy.

Canonical/symlink alias handling, cold oversized inventory peak memory,
independent multi-language judgments, controlled median/p95/RSS measurements
and all-branch release gates remain open. There is no new source/vector schema
or automatic rebuild, and existing indexed Calls determine the available
related evidence. No production reindex is required by this query-time change.
