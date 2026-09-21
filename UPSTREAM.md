# Upstream relationship

This repository is a downstream fork of [bartolli/codanna](https://github.com/bartolli/codanna).
The fork intentionally carries product-oriented workspace, retrieval, hardening, and
code-intelligence work beyond upstream. It is not expected to remain a small patch
stack that can be merged wholesale.

Last reviewed: 2026-09-21

- Upstream repository: `bartolli/codanna`
- Upstream default branch: `main`
- Fork default branch: `main`
- Reviewed upstream head: `12e823c4d965dc8269322869df760dd12d19e415` (v0.16.0)
- Upstream regression gateway: `tests/upstream_regressions.rs`

## Policy

Classify overlapping work before changing the fork:

- **UPSTREAM-CANDIDATE** — generic Codanna correctness or broadly useful behavior
  that should be extracted as a focused contribution based on upstream `main`.
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

| Upstream | Fork status | Fork evidence | Action |
| --- | --- | --- | --- |
| [#127 — TypeScript named import aliases](https://github.com/bartolli/codanna/issues/127) | **UPSTREAM-CANDIDATE** | `tests/upstream/named_import_alias.rs`; alias-aware import identity in fork parsers/resolution | Keep fork behavior. Candidate for a future focused upstream contribution. |
| [#122 / PR #124 — unresolved relative imports crossing roots](https://github.com/bartolli/codanna/issues/122) | **ALREADY-COVERED / UPSTREAM-OPEN** | `tests/upstream/unresolved_import.rs`; identity-safe resolution and incremental cleanup | Compare upstream semantics if PR #124 lands; do not blindly cherry-pick. |
| [#123 / PR #125 — relationship cache truncation above one million symbols](https://github.com/bartolli/codanna/issues/123) | **ALREADY-COVERED / UPSTREAM-OPEN** | `tests/upstream/relationship_cache_scale.rs`; streamed full-symbol hydration | Preserve the streaming implementation and use upstream changes only as compatibility evidence. |
| [#126 / PR #128 — macOS watcher registration scaling](https://github.com/bartolli/codanna/issues/126) | **ALREADY-COVERED / UPSTREAM-OPEN** | batched/recursive watcher registration and `tests/watcher_startup.rs` | Re-test when upstream lands; retain fork root-policy behavior. |
| [#129 — newly indexed roots not watched after settings reload](https://github.com/bartolli/codanna/issues/129) | **UPSTREAM-CANDIDATE** | `tests/upstream/watcher_config_reload.rs`; live settings/root refresh | Keep fork fix. Candidate after smaller upstream contributions. |
| [#130 / PR #131 — vector persistence syscall amplification](https://github.com/bartolli/codanna/issues/130) | **ALREADY-COVERED / UPSTREAM-OPEN** | buffered vector persistence and `tests/vector_storage_batch.rs` | Keep fork implementation and crash-ordering safeguards. |
| [#133 / PR #134 — watcher bursts and stdio shutdown](https://github.com/bartolli/codanna/issues/133) | **ALREADY-COVERED / UPSTREAM-OPEN** | bounded nonblocking event queue, overflow reconciliation, owned watcher lifecycle | Preserve bounded recovery semantics; compare behavior after upstream merge. |
| [#136 / PR #137 — unchanged watcher events rewriting semantic state](https://github.com/bartolli/codanna/issues/136) | **ALREADY-COVERED** | cached code observations skip semantic persistence and notifications; regression in `src/watcher/unified.rs` | Keep the fork fix; watch upstream status only. |

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
