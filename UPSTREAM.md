# Upstream relationship

This repository is a downstream fork of [bartolli/codanna](https://github.com/bartolli/codanna).
The fork intentionally carries product-oriented workspace, retrieval, hardening, and
code-intelligence work beyond upstream. It is not expected to remain a small patch
stack that can be merged wholesale.

Last reviewed: 2026-09-28

- Verified GitHub parent and source: `bartolli/codanna`
- Verified fork/upstream default branches: `main` / `main`
- Coverage window: 2026-09-21 through 2026-09-28 (UTC)
- Initial fork base: `705f946982437941eb845d76c524129c868c151f`
- Rechecked and integrated fork main: `f8018e78db3eb7058141e435a7f0c4f11bbb182e`
- Inspected upstream main: `58295d290f70c2ca46a5ace48fe2aab8687d7d6e`
- Previous upstream baseline: `12e823c4d965dc8269322869df760dd12d19e415`
- Latest inspected release: v0.16.0; the 19 later upstream commits are unreleased.
- Regression gateway: `tests/upstream_regressions.rs`

These SHAs describe the inspected base, not the future merge commit. **THIS-PR**
means implemented on `fix/upstream-regressions-20260928`, not shipped on the
inspected fork main. Review/CI evidence belongs to that exact PR head.

## Policy

Classify overlapping work before changing the fork:

- **UPSTREAM-CANDIDATE** — generic Codanna correctness or broadly useful behavior
  that may be extracted only with explicit authorization, based on upstream `main`.
- **ALREADY-COVERED** — the fork contains equivalent or stronger behavior; do not
  mechanically cherry-pick the upstream implementation.
- **UPSTREAM-OPEN** — upstream has an issue or PR in progress; compare semantics and
  regression coverage before adopting anything.
- **DOWNSTREAM** — intentionally fork-specific product or experimental architecture.
- **UPSTREAMED** — accepted upstream; evaluate whether the downstream patch can be
  removed after the fork's stronger contracts still pass.

Do not open an upstream PR from the fork's full commit history. Generic contributions
should be reconstructed as small branches from current upstream `main`, with the
minimal regression and implementation required for that issue.

## Current overlap map

| Upstream | Actual upstream status at review | Fork coverage and action |
| --- | --- | --- |
| [Named import identity](https://github.com/bartolli/codanna/commit/71b6cdb3100223108f19253fe4f6bc4fe32e5563) | On main; unreleased | **ALREADY-COVERED** by explicit imported-member identity and `tests/upstream/named_import_alias.rs`. Retain fork parser/resolver behavior. |
| [Dangling relative import evidence](https://github.com/bartolli/codanna/commit/58295d290f70c2ca46a5ace48fe2aab8687d7d6e) | On main; unreleased; broader than proposal [PR #124](https://github.com/bartolli/codanna/pull/124) | **THIS-PR** adapts complete/partial file evidence, symbol-free files, same-directory stems, directory/index targets, and TS/JS redirection guards. Preserve explicit export slots/barrels. Evidence: `tests/upstream/index_state.rs`, `tests/upstream/unresolved_import.rs`, `tests/web_export_regressions.rs`. |
| [PR #125: full resolution cache](https://github.com/bartolli/codanna/pull/125) | Merged 2026-09-23; unreleased | **ALREADY-COVERED** by streamed symbol hydration. Keep `tests/upstream/relationship_cache_scale.rs`; its million-symbol witness is opt-in. |
| [PR #128: macOS watcher scaling](https://github.com/bartolli/codanna/pull/128) | Closed unmerged 2026-09-22; rewritten on main in `58dd51f` and `77222c6` | Recursive/batched registration already covered. **THIS-PR** selectively adds code-event admission without gating document/config handlers; helper regression in `src/watcher/unified.rs`. Native FSEvents validation remains distinct from Linux helper tests. |
| [Config sync](https://github.com/bartolli/codanna/commit/f4ac575619f3323e3c9c687f85c602ba59407b4d) / [reload-root persistence](https://github.com/bartolli/codanna/commit/2857d7fd56653734c129320e00b46452f3e02830) | On main; unreleased | **THIS-PR** diffs an unrecorded root instead of forcing duplicate indexing and persists accepted roots without rewriting vectors. Live watcher catch-up already used incremental indexing. Evidence: `tests/upstream/index_state.rs`, `tests/upstream/watcher_config_reload.rs`. |
| [Deferred writer failure](https://github.com/bartolli/codanna/commit/46f4bbfc16e413077cc61fe7b72e142d7fa77c0d) / [watcher propagation](https://github.com/bartolli/codanna/commit/c965e49c75cb7cedbe56fd1aecf487bc0a84d968) | On main; unreleased | **THIS-PR** retains one prepared wave and retries only before the first writer acquisition. Fork store/commit errors are not blindly replayable. Evidence: `src/watcher/deferred_code.rs`, `src/indexing/pipeline/stages/write.rs`, `tests/upstream/index_state.rs`. |
| [Reload serialization](https://github.com/bartolli/codanna/commit/3039c966e3d2f8bca6dbfa31485abb28a7c875c4) | On main; unreleased | **ALREADY-COVERED**: `src/watcher/hot_reload.rs` loads/publishes inside `runtime::mutate`. No mechanical port. |
| [PR #131: buffered vector writes](https://github.com/bartolli/codanna/pull/131) | Merged 2026-09-23; unreleased | **ALREADY-COVERED** with stronger crash-ordering safeguards; retain `tests/vector_storage_batch.rs`. |
| [PR #134: bursts and stdio shutdown](https://github.com/bartolli/codanna/pull/134) | Closed unmerged 2026-09-24; only nonblocking delivery adopted upstream | Preserve fork bounded queue and filesystem-truth reconciliation, not upstream's unbounded channel. **THIS-PR** adds a Phase-1/cancel/Phase-2 worker-ownership regression in `src/runtime.rs`. The 32-path notification threshold is unchanged; native transport/burst behavior still requires its own witness. |
| [PR #137: unchanged watcher events](https://github.com/bartolli/codanna/pull/137) | Merged 2026-09-23; unreleased | **ALREADY-COVERED**: cached code observations skip vector persistence and notifications. Keep the private publication regression in `src/watcher/unified.rs`. |

### Open fork work checked before adaptation

PRs #71 (Markdown headings) and #72 (TypeScript object methods) are merged on the
inspected main and are preserved. The complete diffs of the 12 remaining open PRs
were checked: #70, #73-#83. None implements the above config-sync, retry, or
file-presence fixes. In particular, #83 prepares isolated-project fixtures, not
the missing import classifier; this change does not copy or edit those fixtures.
The retrieval, representation, document support, and corpus-drift PRs remain
independent. A final recheck found #70 and #73 merged on `f8018e7`; those changes
are integrated unchanged. The remaining 10 drafts are #74-#83. The updated #80
head `42e8ee4a973e06499dd3b86b8778272a7a31a0b0` adds segment-measurement
fixtures and a measured receipt, not any of these fixes. Its prepared-data test
result is scoped to that PR and is not qualification of this adaptation.

### Compatibility and validation boundaries

This adaptation advances fork emission semantics from v5 to **v6** and the
project-resolution cache format from 1.0 to **1.1**. Existing indexes require
`codanna index --force`; do not stamp old graphs as rebuilt without rebuilding.
`resolve_deferred` / `resolve_pending` now take `&mut PendingResolution`: only
`is_writer_unavailable()` permits retry, successful work is drained, and adding
another root to a prepared wave is rejected. There is no new MCP schema or dependency.

The retained watcher wave is bounded to one in-memory value. This is not a durable
pending-work journal: a process crash or shutdown while a writer remains unavailable
can still require graph recovery/rebuild. Partial-write failures are surfaced rather
than represented as safe retries.

The new fixtures are deterministic, local, and disable semantic providers. On
implementation commit `4f626009a54219f3c87589d31b318803fde02da9`,
[run 36413674627](https://github.com/bpstr/codanna/actions/runs/36413674627)
passed formatting, 5 focused unit regressions, 7 upstream integration regressions
(2 scale witnesses intentionally ignored), and all 23 export-barrel regressions.
That receipt predates integration of `f8018e7`; combined-head checks must be
verified separately. Native macOS/Windows, million-symbol scaling, and full
transport shutdown are separate qualification scopes, not implied by these passes.

## Upstream regression suite

`cargo test --test upstream_regressions` runs the focused integration fixtures that
represent upstream overlap. Some scale witnesses are intentionally ignored by default
and should be run explicitly when validating the corresponding upstream change.

The cached-watcher semantic persistence regression is a unit-level watcher test because
it verifies the private publication boundary directly rather than requiring a real
embedding provider.

Before removing a downstream implementation in favor of upstream code:

1. Run the focused upstream regression.
2. Run the closest adversarial/retrieval/workspace fixture set.
3. Run `./contributing/scripts/quick-check.sh`.
4. Run `./contributing/scripts/full-test.sh` for consequential changes.
5. Compare performance and persistence behavior where the fork deliberately has
   stronger contracts than upstream.

## Downstream architecture

The following areas should be assumed downstream unless deliberately proposed upstream:

- automatic workspace/CWD isolation and routing;
- workspace-scoped recall and knowledge behavior;
- bounded watcher overflow recovery and filesystem-truth reconciliation;
- generation-based document/vector publication and crash recovery;
- embedding backend/model/dimension/input-policy identity enforcement;
- lexical/BM25 and hybrid retrieval acceptance infrastructure;
- fork-specific review, stress, and product integration tooling.

Upstream changes in these areas are inputs for comparison, not automatic replacements.
