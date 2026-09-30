# September 30 branch consolidation

The user requested one Codanna branch, no open Codanna PRs, and all changes pushed.
The starting fork main was `c5f0a93ad50babab5cae5788370baee56ec8a21d`.
GitHub parent/source remained `bartolli/codanna`; fork and upstream default branches
were `main`, with upstream at `58295d290f70c2ca46a5ace48fe2aab8687d7d6e`.
This is a scoped fork integration, not a new upstream issue/PR audit.

## Integrated work

- PRs #87–#90: shared CPU budgeting, bounded body batches, pool overhead reductions,
  and document starting-cache preservation. Their reviewed heads are `690fed37`,
  `cc958518`, `39f9905a`, and `625b568c`.
- `2daedf9d` preserves and integrates the primary checkout's RC4 repairs.
  Its 13 Rust source/test hashes matched the retained RC4 candidate manifest.
  Pending-resolution recovery and project binding invalidation retain emission
  semantics v10, already installed in the recorded Assign activation. Body-v2,
  embedding identity and vector storage formats remain unchanged. Older emission
  v9 graphs still require the previously documented rebuild; none was run here.
- `ae0bada` preserves the older dirty release checkout as historical evidence.
  Selective fork help/repository links and conservative low-memory optional `auto`
  provider selection are carried forward. Strict/explicit provider selection and
  configured CPU controls remain authoritative. The 4 GiB threshold is a heuristic,
  not an accelerator performance measurement.
- Vector appends now reject count overflow before touching storage and advance
  the in-memory count only after the header write succeeds. Both new regressions
  failed before the fix; all 19 vector-storage tests passed afterward.

The older release snapshot's stable version/changelog, obsolete batching, swallowed
context errors, and alternate persistence paths are superseded. Installer changes
are deferred: the fork has an RC4 prerelease, not a stable latest release, so simply
changing the installer repository would break its default resolution. External
Context7 registration/ownership was not verified and is unchanged.

## Historical branch disposition

Reviewed historical heads are reconciled as ancestry while retaining the current
source tree. This preserves their commits without restoring obsolete workflows,
encoded patch payloads, weaker watcher queues, or replaced storage recovery.
The two review matrices below describe source equivalence and the one narrow
vector guard carried forward. They are review evidence, not test-pass claims.


# Historical upstream/watcher branch review

Read-only review against primary `c5f0a93a`, with current dirty RC4 files inspected for overlap. No tests/builds or provider calls executed. The examined implementations below are unchanged by the dirty RC4 patch. This is branch-content equivalence review, not a fresh upstream GitHub status audit.

| Historical ref | Disposition and current evidence |
| --- | --- |
| `fix/macos-watcher-shutdown` | Superseded. `src/watcher/unified.rs:33,735,879,893,1652` retains large-burst batching and uses bounded nonblocking enqueue plus filesystem reconciliation, stronger than this branch's unbounded channel. `src/cli/commands/serve.rs:179,305` owns, aborts and awaits stdio background tasks. `tests/cli/test_serve_stdio_dual_generation.rs:383` retains the burst/EOF fixture. Cancellation-token grace helper is an alternative lifecycle implementation, not a missing fix. |
| `origin/codex/adapt-upstream-correctness-2026-09-28` | Superseded instructions. Only remaining diff is AGENTS.md ledger policy; current `AGENTS.md:40-73` supplies stricter freshness, overlap, evidence and upstream read-only rules. Do not restore the older instruction block. |
| `origin/fix/upstream-regression-maintenance` | Integrated equivalently. `src/watcher/unified.rs:1144,1201,1720,1765` skips cached semantic saves/notifications and retains both regressions. `tests/upstream_regressions.rs` imports all four grouped modules; `contributing/scripts/review-regressions.sh:27` uses the grouped cache fixture. Current UPSTREAM.md replaces the historical September 21 receipt. All 12 unique commits examined; most perform test renames. |
| `origin/hardening/typescript-import-resolution` | No remaining merge-base diff in inventory. No missing behavior indicated. |
| `origin/hardening/upstream-122-relative-import` | Superseded by stronger origin-aware resolution. `src/indexing/pipeline/stages/resolve.rs:500-524` blocks known internal/dangling unresolved imports while keeping unknown TS/JS inventory cases distinct; `tests/upstream/unresolved_import.rs` retained. Do not reinstate unconditional relative-path rejection. |
| `origin/hardening/upstream-123-symbol-cache` | Integrated. `src/indexing/pipeline/types.rs:1080-1100` streams all symbols with `for_each_symbol`, now also hydrates file inventory. `tests/upstream/relationship_cache_scale.rs:35` retains regression. |
| `origin/hardening/upstream-126-macos-watcher` | Integrated and strengthened. `src/watcher/unified.rs:257,334-359,1453` registers recursive macOS roots; native preparation now precedes transport admission and propagates errors. `.github/workflows/hardening.yml:67-78` retains macOS compile/watcher witness. |
| `origin/hardening/upstream-127-ts-import-alias` | Obsolete patch-delivery scaffolding; all four unique commits only stage/edit one-shot workflow and Python patch script. Intended behavior is present with stronger explicit `Import.imported_name` identity (`src/parsing/typescript/parser.rs:1650-1661`, `behavior.rs:421`), plus `tests/upstream/named_import_alias.rs`. Do not import path-suffix encoding or one-shot workflows. |
| `origin/hardening/upstream-129-config-reload` | Superseded by transactional validated reload. `src/watcher/config_reload.rs:149-175` refuses workspace/storage/semantic identity changes; `src/indexing/facade.rs:1498-1525` refreshes accepted roots, document settings and pipeline. `tests/upstream/watcher_config_reload.rs` retained. |
| `origin/hardening/upstream-130-buffered-writes` | Obsolete one-shot patch workflow only; intended buffered append exists at `src/vector/storage.rs:279-286`. Do not resurrect workflow. |
| `origin/hardening/upstream-130-buffered-writes-v2` | Integrated. `src/vector/storage.rs:279-286` buffers append payload and flushes; full generation writes also buffer at lines 225-236. No v2 schema change needed. |
| `origin/hardening/upstream-133-stdio-shutdown` | Integrated. `src/cli/commands/serve.rs:179,194,261,305-310` owns watcher tasks and aborts/awaits them when stdio transport ends. Current transport also observes disconnect cancellation. |
| `origin/hardening/watcher-overflow-reconcile` | Integrated and strengthened. `src/watcher/unified.rs:735,879,1652` reconciles filesystem truth on bounded nonblocking queue overflow, with retry/config handling beyond original. Queue regression retained at line 1829. |
| `origin/work/review-20260912` | Obsolete delivery/workbench branch: remaining 37-file diff consists of temporary workflows and encoded patch payloads, not source changes. Inspected unique-commit inventory and publishing workflows: they export validated patches to `fix/review-20260912`, deliberately excluding the workbench files. Verified published source commits `c58acdf2` (review hardening), `e187a3ab` (watcher readiness), `193f3790` (server-observed SSE fixture), and evidence `65371c80` are ancestors of current main. Current `docs/security/review-startup-followup.md` preserves failures and corrected validation. Do not bring historical self-mutating workflows or base64 archives onto main. |

## Result

No concrete missing source fix found in this assigned subset. Historical refs can be retired after the root agent preserves their commit reachability as needed and completes the broader consolidation checks. Prefer the current main source tree when recording historical branch ancestry; ordinary merge conflict resolution must not overwrite the stronger current watcher/config/import implementations with these snapshots. This review does not claim that every embedded workbench byte has been revalidated: final published implementation commits and their branch ancestry were checked, and temporary delivery artifacts are intentionally excluded.


# Historical vector, semantic, ingestion and discovery review

Read-only review against c5f0a93ad50babab5cae5788370baee56ec8a21d plus the shared working tree on September 30. No builds, inference, source edits, merges or pushes. Source fallback was used because the parent established the Codanna reader was unavailable. Diff receipts are the corresponding origin__hardening__*.diff files beside this report. Existing tests cited below were inspected, not executed by this reviewer.

| Historical ref (origin/hardening/) | Decision and current evidence |
|---|---|
| authoritative-discovery | Obsolete one-shot patch workflow. Its intended fail-closed walk is present in src/indexing/pipeline/stages/discover.rs:87,169,324; do not restore automation. |
| authoritative-discovery-v2 | Equivalent/superseded. Both parallel and incremental traversal errors fail closed; missing-root regression at discover.rs:535. Current code additionally rejects per-entry ignore errors. |
| borrowed-semantic-save | Superseded by Arc-backed save snapshots (src/semantic/simple.rs:985), borrowed persistence (src/semantic/storage.rs:229), and immutable journal checkpoint saving (src/semantic/journal.rs:353). Regression remains simple.rs:1786. |
| bounded-ingestion | Reject obsolete broken prototype. Adds a max_file_size_bytes setting without wiring it to reads, changes unrelated defaults and removes most Settings implementation/tests. The effective 32 MiB protection is implemented in read.rs:19,185,204, including a bounded reader that prevents growth races. Configurability in the prototype is not functional behavior to preserve. |
| bounded-ingestion-v2 | Superseded. src/indexing/pipeline/stages/read.rs uses opened-handle metadata, rejects nonregular/oversize files and limits read allocation with take(MAX_SOURCE_FILE_BYTES + 1):204; stronger than prototype pre/post fs::read_to_string checks. |
| ci-foundation | Equivalent/superseded by .github/workflows/hardening.yml: reliability, watcher and stress jobs retained, with additional macOS/workspace checks and corrected isolated stress working directory/CLI command. No need to reintroduce historical workflow. |
| non-finite-semantic-vectors | Obsolete one-shot workflow; implemented protections and deterministic regressions are present in current source. |
| non-finite-semantic-vectors-v2 | Equivalent/superseded by finite vector ingestion checks at simple.rs:332,379,412, total_cmp at :1302,1306 and overflow-safe cosine at :1344. Current regressions at :1752,1766. |
| nonfinite-vectors | Equivalent meaningful changes. Remote finite validation at src/semantic/remote.rs:287 with regression :301; dimension/finite validation at src/vector/types.rs:274 with regression :423. Other historical changes are comment removal. |
| parse-worker-panic | Equivalent and extended to READ workers. src/indexing/pipeline/workers.rs:122-159 propagates parse panic as fatal; regression :187. |
| semantic-storage-validation | Equivalent and strengthened. src/semantic/storage.rs:72,88 validates physical layout; helper :286 and corruption regressions :477,:501 retained. Nonfinite legacy-vector regression :522 also present. |
| vector-batch-memory | Equivalent. src/vector/storage.rs:178 writes borrowed slices directly; no owned full-batch clone. |
| vector-physical-layout | Equivalent. src/vector/storage.rs:136,:762,:769 validates open/remap physical length, version and dimensions; corruption/dimension/remap regressions :936,:987,:1014,:1034 retained. |
| vector-write-path | Equivalent. Borrowed write_batch at storage.rs:178 and BufWriter append path :261-287 preserve payload buffering. |
| vector-append-recovery | Partly superseded, partly missing: see below. Do not reapply wholesale. |

## Specific carry-forward finding

The historical vector-append-recovery implementation checked new_count with checked_add and rejected counts beyond u32::MAX before writing, then updated the in-memory count only after header publication. Current src/vector/storage.rs:290-295 increments self.vector_count unchecked before update_header_count; :816 casts it to u32. Carry forward that narrow guard and publication ordering, with deterministic tests for overflow and failed header writes if implementing the latter.

Historical automatic truncation inside open is deliberately not recommended: it mutates a file from a read path and can conflict with other mapped readers. Current live semantic/document publication supersedes that recovery model: src/semantic/journal.rs:348-359 builds and syncs a private checkpoint before atomic manifest publication; src/documents/store.rs:2235 and :2357 write private vector staging, and src/documents/generation.rs:329-350 syncs/opens the complete segment before publication. Preserve strict read-only physical validation and the current v2 contracts. Historical one-shot workflows are obsolete and must not return.

This review establishes implementation equivalence or explicit rejection reasons for history-only reconciliation, except the narrow count guard finding above. It does not claim runtime performance or fresh test passes.


## Additional ancestry-only heads

`codex/integrate-pr38` has no unmatched patches against the starting main. The
merge commits in `codex/integrate-pr71`, `origin/feature/knowledge-agent-boost`,
`origin/feature/knowledge-architecture`, `origin/feature/knowledge-links`, and
`origin/feature/languages-ruby-bash` have no extra remerge conflict resolutions.
Their content is already represented in main.

## Boundaries and recovery

The separate `codecase` remote, its `codex/codecase-1.0.0` branch, and dirty Codecase
checkout are outside this repository consolidation and remain untouched. Original
refs and dirty-tree inventories, plus a complete pre-consolidation Git bundle,
are retained locally under `.git/consolidation-backups/2026-09-30/`. The old release
checkout's changes are additionally preserved by `ae0bada` in main's ancestry.
No live Assign index, failed document store, credentials or model cache was removed.
No inference provider was called for these checks.

## Combined validation

Combined source at `57c9334c` (unchanged through ancestry reconciliation
`51176ed4`) passed both `./contributing/scripts/quick-check.sh` and the complete
serial `./contributing/scripts/full-test.sh`: **2,727 passed, zero failed, 63 ignored
across 69 test targets**, with CLI, documentation and scratch-workspace MCP checks
also passing. Focused runtime tests passed 11/11; vector storage passed 19/19.
Tests used a clean environment with no provider credentials, deterministic local
fixtures and mocked transports. Ignored tests remain unqualified.

Logs are retained at `/tmp/codanna-consolidation-20260930/`, including the original
failing vector regression log. GitHub publication and branch retirement follow
this validated local integration; their verified result is recorded below.

## Publication and branch retirement

GitHub confirmed PRs #87, #88, #89 and #90 merged after the normal fast-forward
push of `main` through `3dee387a`. Zero open PRs remain. All 101 non-main remote
Codanna branches were deleted atomically with exact-head leases after confirming
every head was an ancestor of main. All 40 redundant local Codanna branches were
also removed after their clean worktrees were detached. The original worktree
directories remain; the primary checkout returns to `main`. The fork's sole remote
branch is `main`. The separate Codecase branch remains outside this scope.

## Session disk cleanup

After the test process exited and `lsof` found no open files, removed only
`/Users/bpstr/.codex/worktrees/assign-index-performance/codanna/target`
(11.09 GiB allocated). Free space increased by 10.40 GiB
to 31.20 GiB. The pre-existing primary `target` (3.0 GiB), Rust toolchains,
indexes, model caches, logs and recovery bundle were preserved.
