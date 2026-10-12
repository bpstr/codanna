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
- Publication-time fork main comparison: `3a61d25bdf63ba9a8de0504a33b75e26da0f52f3`
- Inspected upstream main: `58295d290f70c2ca46a5ace48fe2aab8687d7d6e`
- Previous upstream baseline: `12e823c4d965dc8269322869df760dd12d19e415`
- Latest inspected release: v0.16.0; the 19 later upstream commits are unreleased.
- Regression gateway: `tests/upstream_regressions.rs`

These SHAs describe the upstream-review inputs. **THIS-PR** marks repairs delivered
by [fork PR #84](https://github.com/bpstr/codanna/pull/84), merged on 2026-09-29 as
`dacade8142cc009671ae21887818dc37a6080904`. Review/CI evidence remains scoped to
its recorded head; the September 28 upstream review date has not been advanced.

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
| [Config sync](https://github.com/bartolli/codanna/commit/f4ac575619f3323e3c9c687f85c602ba59407b4d) / [reload-root persistence](https://github.com/bartolli/codanna/commit/2857d7fd56653734c129320e00b46452f3e02830) | On main; unreleased | **THIS-PR** diffs an unrecorded root instead of forcing duplicate indexing and persists accepted roots without rewriting vectors. Live watcher catch-up already used incremental indexing. Evidence: `tests/upstream/index_state.rs`, `tests/upstream/watcher_config_reload.rs`. September 29 fork repair also fingerprints effective TS/JS rules, reloads inherited config and rebinds config-only alias changes; `tests/index_lifecycle_regressions.rs` and the remediation receipt record local evidence. Upstream status was not rechecked for this repair. |
| [Deferred writer failure](https://github.com/bartolli/codanna/commit/46f4bbfc16e413077cc61fe7b72e142d7fa77c0d) / [watcher propagation](https://github.com/bartolli/codanna/commit/c965e49c75cb7cedbe56fd1aecf487bc0a84d968) | On main; unreleased | **THIS-PR** retains one prepared wave and retries only before the first writer acquisition. Fork store/commit errors are not blindly replayable. Evidence: `src/watcher/deferred_code.rs`, `src/indexing/pipeline/stages/write.rs`, `tests/upstream/index_state.rs`. September 29 fork repair adds durable source-path obligations for crash recovery (emission v10); local lifecycle/crash evidence is in `contributing/release-results/2026-09-29-rc4-remediation.md`. Upstream status was not rechecked for this repair. |
| [Reload serialization](https://github.com/bartolli/codanna/commit/3039c966e3d2f8bca6dbfa31485abb28a7c875c4) | On main; unreleased | **ALREADY-COVERED**: `src/watcher/hot_reload.rs` loads/publishes inside `runtime::mutate`. No mechanical port. |
| [PR #131: buffered vector writes](https://github.com/bartolli/codanna/pull/131) | Merged 2026-09-23; unreleased | **ALREADY-COVERED** with stronger crash-ordering safeguards; retain `tests/vector_storage_batch.rs`. |
| [PR #134: bursts and stdio shutdown](https://github.com/bartolli/codanna/pull/134) | Closed unmerged 2026-09-24; only nonblocking delivery adopted upstream | Preserve fork bounded queue and filesystem-truth reconciliation, not upstream's unbounded channel. **THIS-PR** adds a Phase-1/cancel/Phase-2 worker-ownership regression in `src/runtime.rs`. The 32-path notification threshold is unchanged; native transport/burst behavior still requires its own witness. |
| [PR #137: unchanged watcher events](https://github.com/bartolli/codanna/pull/137) | Merged 2026-09-23; unreleased | **ALREADY-COVERED**: cached code observations skip vector persistence and notifications. Keep the private publication regression in `src/watcher/unified.rs`. |

### Open fork work checked before adaptation

September 30 scoped performance qualification: fork main `c5f0a93a` and all
open fork performance diffs #87 (`690fed37`), #88 (`1745582c`), #89 (`39f9905a`)
were inspected; GitHub parent/source remain `bartolli/codanna`, both defaults
remain `main`, and upstream main resolved to `58295d290`. The full upstream
review date above is unchanged. PR #88 is **DOWNSTREAM, merged** (verified September 30), with
the same v2 policy, exact inputs, schema and rebuild compatibility. Its prepared
CLI witness `body_v2_cross_window_rebuild_reuses_inputs_and_invalidates_one_edit`
in `tests/support/body_rebuild_cache_cases.rs` covers 80 commentless parents,
cross-window cold inference, warm rebuild with remapped IDs, and one edited body.
That CLI witness and 129 pipeline-stage tests passed on macOS with zero failures
or ignored tests; further execution results are recorded in the PR. Prepared vectors are not model-quality
or Assign throughput evidence. The merged document-cache snapshot change
touches document indexing and does not duplicate #88.

PRs #71 (Markdown headings) and #72 (TypeScript object methods) were already merged
on the initial base. The complete diffs of the then-open PRs #70 and #73-#83 were
checked. None implements the above config-sync, retry, or file-presence fixes.
In particular, #83 prepares isolated-project fixtures, not the missing import
classifier; this PR does not copy or edit those fixtures.

During implementation, #70 and #73 merged on `f8018e7`; those changes are integrated
unchanged. The updated #80 head `42e8ee4a973e06499dd3b86b8778272a7a31a0b0`
was inspected and subsequently verified merged as `3a61d25`. The complete main
comparison from `f8018e7` to `3a61d25` adds only its segment-measurement fixture and
receipt; neither overlaps this adaptation. Its test results are scoped to that PR,
not borrowed as qualification for this one.

The other open work at publication is #74-#79 and #81-#83, separate from this PR.
Retrieval, representation, document support, corpus drift, and isolated-project
fixtures remain independent; fixture presence alone does not establish a passing
qualification or shipped behavior.

### September 29 fork integration

Fork main `b60032fb` includes the previously pending PRs #74-#79 and #81-#83,
and compact output PR #85. PR #84 was the sole remaining open fork PR at that
integration checkpoint and is now merged; no open fork PRs remained on verification. Its TypeScript import-presence classification is combined
with main's exact project/export binding; the emission stamp advances to v9.
This is a fork lifecycle update, not a new upstream review.

The first full-test and reliability runs on `0bf5e932` failed two watcher fixtures:
the facade and watcher used different index paths, so metadata publication failed.
Both fixtures now use their facade's explicit index path; local watcher validation
passes 49 tests. The final native macOS gate passed 2,705 tests with 63
explicitly ignored; all 21 applicable final-head GitHub checks passed, including
default/all-feature Linux tests and native macOS watcher checks. Native macOS qualification additionally exposed two upstream
fixtures using `/tmp` aliases instead of canonical source identities; these now
match CLI/workspace startup. Original failed runs remain part of the evidence.
See the [release testing manual](contributing/release-testing.md) for the retained
nonretryable-wave recovery limitation and the separate Assign readiness gate.

### Compatibility and validation boundaries

After integration with fork main `b60032fb`, this adaptation advances fork emission semantics from v8 to **v9** and the
project-resolution cache format from 1.0 to **1.1**. Existing indexes require
`codanna index --force`; do not stamp old graphs as rebuilt without rebuilding.
`resolve_deferred` / `resolve_pending` now take `&mut PendingResolution`: only
`is_writer_unavailable()` permits retry, successful work is drained, and adding
another root to a prepared wave is rejected. There is no new MCP schema or dependency.

The original v9 watcher wave is bounded to one in-memory value. RC4 qualification
confirmed that discarding it after Phase 1 could lose calls. The September 29 fork
repair advances emission semantics to **v10** and publishes durable source-path
obligations with registrations and incoming-edge cleanup. Restart reparses these
sources and hydrates resolution from the persisted graph. Failed recovery retains
obligations; it does not blindly replay a partially written wave. RC4 indexes need
`codanna index --force` because they lack those records. The resolution cache
format remains 1.1; no MCP schema or dependency changes were added. This scoped
fork repair does not advance the upstream review date or inspected SHAs.

See the [remediation receipt](contributing/release-results/2026-09-29-rc4-remediation.md)
for current local validation and the deferred multilingual qualification gap.

The new fixtures are deterministic, local, and disable semantic providers. Initial
implementation commit `4f626009a54219f3c87589d31b318803fde02da9` passed formatting
and 35 focused tests in [run 36413674627](https://github.com/bpstr/codanna/actions/runs/36413674627).

After integrating `f8018e7`, code commit `f63b6fc2e0ed7d9b7a8ea6afb063a6e1a921e098`
passed [qualification run 36414379842](https://github.com/bpstr/codanna/actions/runs/36414379842):

- Strict Clippy across all targets and all features, with warnings denied.
- Compilation with no default features.
- 5 focused unit regressions.
- 7 upstream integration regressions; 2 scale witnesses intentionally ignored.
- All 23 export-barrel regressions and 4 typed-symbol-ID contract tests.

Those are 39 executed passing tests, not a claim that the full repository or native
platform matrix passed. Subsequent edits only update agent/ledger documentation and
remove the temporary runner; normal PR checks qualify the final head. Native
macOS/Windows, million-symbol scaling, and full transport shutdown remain separate
qualification scopes, not implied by Linux helper tests.

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

### September 30 document cache follow-up

September 30 scoped fork performance follow-up (the upstream review date above
is unchanged): fork main `c5f0a93ad50babab5cae5788370baee56ec8a21d` and all open
performance PR diffs #87 (`690fed37`), #88 (`1745582c`, then the reviewed/tested
`cc958518` follow-up), #89 (`39f9905a`)
were inspected, along with recent merges #79–#86. GitHub parent/source and both
`main` defaults were rechecked; upstream main still resolved to `58295d290`.
No full upstream issue/PR status refresh is claimed.

| Fork work | State and compatibility | Evidence |
| --- | --- | --- |
| Preserve document starting-cache hits across embedding batches | **DOWNSTREAM, merged as #90** (verified September 30); combined with the merged code/runtime performance PRs #87–#89. Same body-v2/source policy, model identity, cache format and index schema; no rebuild required. Retains one bounded cache snapshot per embedding run. | `src/documents/store.rs`: `document_runs_preserve_starting_cache_hits_across_batches`; `contributing/performance/assign-indexing-2026-09-30.md`. The regression failed before the fix (65 inferred inputs instead of 64); after the fix, `cargo test --lib documents::` passed 69 tests, zero failed/ignored. Strict quick checks and the serial full gate passed (2,707 passed, 63 ignored, zero failed; CLI/docs/MCP passed). No Assign speedup or native model qualification is claimed. |

### September 30 branch consolidation

The current fork integration combines PRs #87–#90 and the preserved RC4 repair.
The original review date/SHAs above remain unchanged. All current PR heads were
rechecked before integration and had not moved. Historical branches were reviewed
for equivalent implementations; their commits are retained as ancestry while
superseded source/workflow variants remain excluded. The affected buffered-vector
write behavior now preflights the u32 count limit before writes and publishes the
in-memory count only after a successful header write. The optional non-strict
`auto` provider selection uses CPU below 4 GiB of memory headroom, retaining strict
selection and explicit CPU controls. Neither change alters body-v2, embedding
identity or the storage format, and neither requires an index rebuild.

Evidence: `contributing/release-results/2026-09-30-branch-consolidation.md` records
branch dispositions, reproduced vector defects, 19 passing vector tests, 11 passing
runtime tests, strict quick checks and the full serial gate (2,727 passed, zero
failed, 63 ignored; CLI/docs/MCP passed). This scoped fork work does not refresh
upstream issue statuses or claim measured Assign throughput improvements.

Publication verified September 30: PRs #87–#90 are merged, the fork has zero open
PRs, and its only remote branch is `main`. Superseded heads remain reachable in
main history. The separate Codecase remote/branch was excluded.

RC5 executable follow-up: `contributing/release-results/2026-09-30-rc5.md` records
the clean `be166174` macOS candidate, model-free smoke/structural/recovery checks,
graph-only baseline comparisons and passing default/all-feature Linux CI. The
original resource-exhausted CI run is preserved as failed evidence. This adds
qualification evidence only; the upstream review date and compatibility remain
unchanged. No tag, installed binary or active Assign index changed in this step.

After user approval, tag `v1.0.0-rc5` was pushed at qualified commit `be166174`
and the tested macOS executables were installed with hash and scratch-MCP
verification. `contributing/release-results/2026-09-30-rc5-installation.md` records
rollback and existing-server limits. No index/schema or upstream status changed.

### Fork binary installer (October 8)

| Behavior | Upstream status | Downstream state and evidence |
| --- | --- | --- |
| One-command prebuilt installation | Not rechecked; the upstream review date remains September 28. | **DOWNSTREAM**: `README.md` points to `scripts/install.sh` in `bpstr/codanna`. The installer uses unauthenticated `curl` against GitHub's `/releases/latest` endpoint to select the latest stable release, excluding drafts/prereleases by default, then downloads the platform asset and verifies SHA-256 before replacing the executable. GitHub CLI is not required. `CODANNA_VERSION` pins a public tag; `CODANNA_INSTALL_DIR` overrides the destination. No index rebuild is required. |

GitHub release metadata was checked October 8: RC5 and RC1 remain drafts; RC4
is a published prerelease with failed-qualification notes. The unauthenticated
stable-release endpoint returned HTTP 404: no stable release is currently
published. The installer reports that condition without changing an existing
installation. Publishing the fork installer does not publish or qualify these
binaries. Offline evidence:
`python3 contributing/scripts/test-install.py` passed 22 tests with zero failures
or skips; `sh -n scripts/install.sh`, ShellCheck and `git diff --check` passed.
Rust build/test gates and live binary/model probes were not run for this shell
installer change. Existing RC qualification receipts above remain scoped to
their original runs.
