# Release testing manual

Use this manual for a local release candidate before replacing an installed
binary or rebuilding a large workspace such as Assign. Record each gate as
**pass**, **fail**, **not run**, or **blocked**, with the exact command, exit code,
source revision, binary hash and evidence path. A skipped test is not a pass.
Keep the first failed run when a repair requires a second run.

Latest qualification: [September 29, 2026 — no-go](release-results/2026-09-29-qualification.md).
Previous installation receipt: [September 29 RC4](release-results/2026-09-29-rc4.md).
Previous baseline: [September 29 RC3](release-results/2026-09-29.md).

This extends the [RC3 process](retrieval/rc3/README.md), its
[measured results](retrieval/rc3/RESULTS.md), the
[body/rebuild qualification](retrieval/retrieval-body-qualification.md), and the
[real repository task evaluation](retrieval/repository-task-evaluation.md).
Their results apply to their recorded revisions and corpora. They are not
qualification receipts for a newer binary.

## 1. Freeze the candidate and protect the installed release

1. Start from the requested fork's current main. Inspect every remaining PR,
   conflicts, reviews and CI failures. Resolve conflicts against current main;
   retain both branches' behavior and advance compatibility versions when the
   combined graph differs. Reconcile affected rows in `UPSTREAM.md` without
   pretending that a lifecycle update is a new upstream review.
2. Keep unrelated work intact. Repair a PR in an isolated managed worktree,
   leaving the primary checkout on main. Run the checks below before merging;
   build and identify the merged main artifact again before installation.
3. Save the installed executable, its `--version`, SHA-256, permissions and
   resolved installation path outside the checkout. Record running MCP servers,
   their executable paths and selected workspaces. Replacing a file does not
   upgrade an already running server.
4. Record OS/architecture, CPU, RAM, Rust/Cargo versions, feature flags, source
   SHA and dirty state, model revision/hash, input policy, emission version and
   resolver-cache version. Never qualify a dirty build as the clean commit.
5. Record free space on the data volume and sizes of build, cache and evidence
   directories. Estimate the additional space needed for the planned build and
   tests, and pause only if available space cannot accommodate that work. There
   is no fixed minimum free-space requirement. Preserve indexes and model
   caches; neither is disposable test output.

## 2. Isolate tests and prohibit paid inference

Read Assign's `architecture/operations/paid-inference-testing-policy.md` before
AI-related testing. The current user's prohibition controls if a stored policy
contains broader authorization: no paid API, subscription proxy, ambient provider
credentials, `.secrets`, automatic fixture recording or live model grading.

Use deterministic fixtures and prepared/mock transports for automated checks.
The existing retrieval qualification commands below use an explicitly selected,
already cached model for local CPU inference over fixed source fixtures. They
make no paid-provider calls. Record these as local-model development measurements,
separately from deterministic contract tests. A missing model must fail closed;
do not add a download or provider fallback to make a run pass.

Use a fresh output directory outside all indexed source roots. Qualification
harnesses create disposable HOME/config/cache/workspace directories and an
allowlisted environment, verify model assets and disable downloads. For Cargo
checks, use an environment containing only build necessities; omit credentials
and configuration overrides. Check ignored tests before opting into any of them.
Loopback-only mock services are allowed. Read-only planner runs need no model.

## 3. Qualify the combined implementation

Run the focused regressions for the change first, then the repository gates:

```sh
./contributing/scripts/quick-check.sh
./contributing/scripts/full-test.sh
cargo test --locked --all-features --no-fail-fast
```

The scripts include formatting, strict all-target/all-feature Clippy,
no-default-feature compilation, default-feature tests, CLI help, documentation
and a scratch-workspace MCP smoke. The explicit all-feature test command covers
the additional full-CI lane. Capture complete logs and test totals, including
ignored and filtered tests. The scripts currently attempt a stable-toolchain
update; record the actual toolchain used. Keep their expected `target/debug`
location available if using an alternate Cargo target directory.

Also inspect applicable GitHub workflows at the candidate head. Required CI must
finish successfully; a skipped release-build job caused by failed tests is not
release evidence. Native macOS watcher tests and Linux CI complement each other.
Do not describe untested Windows or other architectures as qualified.

At minimum, confirm the existing regression suites exercise:

| Area | Required behavior |
| --- | --- |
| Persistence | Restart/reload, missing and corrupt state, metadata/graph/vector generations agree; no partial publication presented as complete. |
| Compatibility | Old emission/input/cache identities fail with an actionable rebuild requirement; no silent stamping of old data. |
| Incremental indexing | Create, body-only edit, rename, delete, ignored/reincluded roots, config changes and no-op indexing preserve correct symbols and edges. |
| Watcher recovery | Overflow reconciles filesystem truth; deferred writer failure retries without replaying Phase 1; cancellation does not strand a committed wave. |
| Imports/relationships | Dotted paths, symbol-free modules, barrels, aliases and multiple TypeScript projects cannot create foreign-project edges. |
| Retrieval/MCP | CLI and actual stdio transport agree; HTTP contracts, routing, scopes, budgets, errors, shutdown and source provenance remain correct. |
| Documents | Chunk bounds, token limits, headings, vector counts, churn/deletion and generation replacement remain consistent. |

Useful test targets include `upstream_regressions`, `web_export_regressions`,
`semantic_reload_retention`, `workspace_live`, `workspace_mcp` and `cli`. Discover the current
target/filter names in `Cargo.toml` and CI before executing; names alone do not
prove the required assertions exist. Run ignored scale witnesses separately only
after inspecting their resource needs and mock/local-only boundaries.

## 4. Build and measure the actual release executables

```sh
cargo build --locked --release --bin codanna --bin codanna-knowledge \
  --bin codanna-index-plan
shasum -a 256 target/release/codanna target/release/codanna-knowledge \
  target/release/codanna-index-plan
target/release/codanna --version
```

Freeze the model and query manifest before inspecting results. Run the same
commands against the saved installed binary and the candidate, with separate
output directories and matching auxiliary binaries where available. Record a
mixed auxiliary-binary baseline as a limitation. Never silently change judgments
or thresholds to make a candidate green.

```sh
python3 contributing/retrieval/qualify-local.py \
  --codanna /absolute/path/to/codanna \
  --knowledge /absolute/path/to/codanna-knowledge \
  --cached-models /absolute/path/to/existing/models \
  --out /absolute/path/to/new-fixture-run --allow-local-model
python3 contributing/retrieval/qualify-repository.py \
  --codanna /absolute/path/to/codanna \
  --planner /absolute/path/to/codanna-index-plan \
  --cached-models /absolute/path/to/existing/models \
  --out /absolute/path/to/new-repository-run --allow-local-model
```

Keep lexical, default comment-semantic and body-semantic profiles separate. The
repository probe copies eight exact source files and judges six exact owners;
record their hashes because changing source changes the corpus. Preserve query
ranks, expected paths/names, forbidden evidence, raw responses and errors. Report
Hit@5, MRR@5, required-evidence recall@5, document nDCG@5, duplicate-source slots
and per-language results. All invariants must pass. The original fixture targets
remain Hit@5 >= 0.90, MRR@5 >= 0.75 and required-evidence recall@5 >= 0.90, plus
each case's requirements; the six-query repository probe requires six hits.
See [the acceptance contract](retrieval/README.md#minimums-and-known-red-cases).

Known failures are release findings, never expected passes. RC3's MiniLM
repository result was 5/6 hits and MRR@5 0.444; its graded document Hit@5 was
0.700. The real-source operational/paraphrase task set also missed its relevance
targets. A passing synthetic fixture or an all-vectors-present count cannot
replace those judgments. Keep the model fixed for regression comparison;
multilingual model experiments are a separate lane with separate identities.

### Indexing performance protocol

Run performance work after builds/tests finish, with no concurrent indexer for
the measured corpus. Compare baseline and candidate on identical frozen source,
settings, hardware and cached model. Separate graph-only throughput from local
embedding throughput, and source parsing from document indexing. Report cold
index creation, unchanged incremental indexing, one-file/body edit, deletion,
and force rebuild with retained compatible embedding cache.

Perform at least three alternating baseline/candidate trials on fresh isolated
indexes. Record each wall time, median/range, process peak RSS (macOS
`/usr/bin/time -l`), source files/bytes, symbols/relationships, eligible owners,
vectors, cache reuse, index/cache disk growth and diagnostics. Warm-up and model
load times must remain explicit. Measure repeated query latency separately from
CLI startup; do not call subprocess wall time warm MCP latency. Never compare
different file counts or graph versions as a pure throughput improvement.

For subsecond commands, use pipe-driven process completion or an equivalent
high-resolution child timer. Python's timeout-based wait with file outputs can
add polling granularity comparable to the operation itself. Preserve any failed
initial measurement; investigate with additional alternating trials rather than
lowering thresholds or dropping inconvenient samples.

An unexplained slowdown greater than 15% in median time or increase greater than
20% in peak RSS/index size against the same baseline blocks performance
acceptance pending investigation. These are regression tripwires, not proof of
large-workspace capacity. Set absolute time, disk and memory budgets for the
intended Assign corpus before its staged rebuild. No baseline or uncontrolled
concurrency means the performance gate is unmeasured, not passed.

## 5. Decide installation and Assign readiness separately

**Local binary installation** requires successful combined-code checks, no new
correctness or retrieval regressions, completed applicable CI, a clean identified
release build and a retained rollback executable. Record known quality failures
explicitly. Installing a developer candidate does not certify a stable general
release when the original relevance targets remain red.

**Activating a rebuilt Assign index** additionally requires all of the following.
An isolated staging rebuild may collect missing qualification evidence while the
active index remains intact. Readiness to start that experiment does not mark any
quality or scale gate passed, or authorize activation. Before starting, freeze its
configuration, check available disk against a stated stage budget, bound time and
memory, and retain a matching rollback binary/index pair.

1. A disposable representative Assign subset with independently reviewed queries
   and exact implementation-owner judgments. Include undocumented code, JSX,
   isolated project aliases, configuration, long/multi-section documents, actual
   English/Hungarian queries and distractors. Keep the source inventory and
   judgments outside the indexed roots; exclude secrets and generated artifacts.
2. The intended model/input policy passes the fixed relevance thresholds and
   source-grounding checks on that subset. Re-run actual transport, restart,
   body-only freshness, watcher mutation and generation checks on the selected
   profile. An eight-file Codanna probe cannot substitute for this gate.
3. Check that `.codannaignore` excludes `.fastembed_cache` without a trailing
   slash, including in existing workspaces. Preserve the cache and keep source
   symlink rejection intact. Run the [source-only planner](retrieval/rebuild-cost-preflight.md) with the
   intended settings and save its JSON outside Assign. Inspect partial/blocked
   status, input rejections, estimated body segments, cache pressure and unknown
   values. A partial local-tokenizer result is not a full capacity estimate;
   the planner excludes document collections and is not a price quote.
4. Qualify a corpus above 10,000 document chunks and representative code scale.
   Check long-document dominance, duplicate slots, bounded memory and disk,
   cancellation/restart/recovery, query latency under indexing load and cache
   pressure. Tiny fixtures cannot close this gate.
5. Preserve the old binary, index, settings and model/input identities. Build a
   separate staged index, never overwrite the only working index. Compare source
   inventory, graph counts, eligible owners, missing/orphan vectors and query
   evidence before changing the active index. Test rollback with the matching
   old binary/index pair; an older binary may reject a newer index.
6. Record explicit go/no-go, evidence, budgets, stop conditions and remaining
   failures. Stop on corruption, incomplete publication, rejected input,
   unexplained missing sources, resource-budget breach or quality regression.
   If any required gate is red, blocked or unmeasured, leave Assign unchanged.

RC4's September 29 qualification failed because Phase 1 hashes survived while
call relationships did not. The repair advances emission semantics to v10:
Phase 1 publishes durable source-path resolution obligations with registrations,
and replacement cleanup publishes obligations for unchanged incoming callers
before removing their edges. Incremental restart reparses queued sources in a
batch, including live sources whose registration was lost during cleanup.
Failed reparses retain obligations; successful resolution and embedding
publication retire them. Authoritative deletion retires obligations with its rows.
Old RC4 graphs lack these records and require an isolated forced rebuild.

The [remediation receipt](release-results/2026-09-29-rc4-remediation.md) records
the regression tests and observed process-crash witness; these do not establish
recovery at every partial-write point. Do not inject failures into the active
Assign index. Any indexing or resolution error in a staged full rebuild must
abort qualification: preserve failed evidence and restart from empty with no
concurrent writer. Activation still requires the remaining scale, document,
transport and quality gates. English retrieval is the current focus;
multilingual quality remains a separate, unqualified follow-up.

The September 29 qualification reproduced missing call edges after process
termination and a successful incremental restart. On macOS, run the bounded,
model-free witness against the candidate, with a fresh output path outside the
checkout:

```sh
python3.11 contributing/retrieval/qualify-code-recovery.py \
  --codanna /absolute/path/to/codanna \
  --out /absolute/path/outside-checkout/code-recovery
```

It preserves the resumed index before a forced rebuild and checks exact counts
and sampled call edges. Its nonzero result blocks promotion. This single crash
point does not replace nonretryable failure injection or captured-incoming-edge
coverage. Also test configuration-only path-alias changes under ordinary
incremental indexing; success after forcing a rebuild does not pass that gate.

Paid provider dogfood, if later requested, needs its own exact real-content scope,
hard request/monetary cap and stop condition before any call. This manual grants
no spending authorization.

## 6. Install, verify and retain the receipt

Install by writing an executable sibling temporary file at the resolved local
binary location and atomically renaming it over the old file. Keep the backup
outside the checkout. Compare the installed file's SHA-256 to the tested artifact,
run `--version`, CLI help and a scratch-workspace index/query/restart smoke using
that installed path. Upgrade auxiliary binaries only from the same tested build.

Identify running server processes before restarting them. Restart only the
intended consumers after their index compatibility and rollback plan are known;
do not kill unrelated clients. Report which servers still hold the old executable.

Save a compact receipt with revisions/hashes, environment, complete logs, test
totals, first failures and repairs, timing trials, corpus/model identities,
quality scores by profile, installation/rollback paths and the Assign gate
decision. Check free space again. Remove only inactive reproducible artifacts
created by this session, preserving source, indexes, prior caches and evidence.
Report exact removed paths, reclaimed bytes and remaining space. Ask before
expanding cleanup to unrelated artifacts.
