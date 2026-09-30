# September 29 RC4 remediation

The [original qualification failure](2026-09-29-qualification.md) remains a
historical no-go receipt. This repair addresses code-graph recovery and config-only
alias rebinding. It does not activate an index or publish a replacement release.
The installed binary and active Assign index remain unchanged.

## Behavior changes

- Publish each successfully indexed file's pending resolution obligation in the
  same Tantivy batch as its registration/hash.
- Publish unchanged incoming callers' obligations with replacement cleanup, then
  recover queued sources in one Phase 1 batch after restart. Live queued sources
  recover even if cleanup already removed their registration. Read/parse/write
  failures retain retry obligations. Authoritative deletion retires them.
- Clear obligations only after successful relationship and embedding publication.
  Preserve INDEX-stage failure counts when combining read/parse failures.
- Rebuild effective TypeScript/JavaScript project rules, including inherited config,
  and fingerprint those inputs independently of source hashes. Changed rules
  invalidate indexed TS/JS files conservatively. Resolution registers the stored
  source identity rather than a portable display path.

Emission semantics advances from **v9 to v10**. Existing RC4 graphs lack recovery
records and need a staged `codanna index --force` rebuild. The project resolution
cache format, default embedding model and MCP contracts are unchanged.

## Validation

Local evidence root:
`/Users/bpstr/Github/codanna-release-evidence/2026-09-29-rc4-repair`.
All runs use isolated indexes and deterministic local sources. English embedding
measurements use cached MiniLM weights with an offline endpoint. No paid inference
or active-index fault injection is authorized or used.

The frozen candidate SHA-256 is
`1774f72bfda3b28edab9531696e22396a481f31c495a5066e9c65035112928dc`;
`candidate-manifest.json` records its Git base and hashes of modified Rust sources.

| Check | Result | Evidence under the local root |
| --- | --- | --- |
| Lifecycle regressions | 10 passed; no failures or ignored tests | `codanna-rc4-lifecycle-final4.log` |
| Original process-crash witness | 205 files / 20,500 symbols / **20,295 calls** recovered; exact forced-rebuild count parity; all nine sampled outgoing edges present | `code-recovery-batched/report.json` and retained `resumed-index/` |
| Original self-contained alias witness | Baseline, web-only redirect and restore pass; mobile binding preserved in each phase | `alias-witness-2/report.json` |
| English frozen owner queries | **11/11 Hit@5**, **MRR@5 0.955**, default MiniLM and `symbol_body_v1`; unchanged from the original English result | `english-symbol_body_v1/report.json` |
| Quick gate | Formatting and strict all-feature Clippy passed | `quick-check-final2.log` |
| Full gate | **2,710 passed, 0 failed, 63 ignored** across 69 test targets; no-default-features, CLI, docs and scratch MCP checks passed | `full-test-final2.log`, `gate-summary.json` |

Crash restart took 14.99 seconds on this witness. The SIGKILL followed observed
Tantivy publication while the indexer remained alive. This proves recovery at
that observed point, plus the separate outgoing/incoming and failed-reparse
fixtures; it does not cover every partial-write point. The alias witness uses
self-contained config, not real Expo package-extends qualification.

Initial compile/Clippy failures, a temporary-path mismatch in the new fixture,
and the earlier per-file recovery timeout remain in evidence. The first alias
replay stopped before any CLI command because the retained fixture began with a
redirected config; the second explicitly seeded the original variant. The
successful crash replay uses batch recovery. None of these failed or aborted
attempts is represented as a pass.

The English replay uses the opt-in body representation. The unchanged
comment-only representation previously found 3/11 English implementation owners
(MRR@5 0.273); this replay does not qualify that default representation. For the
measured English profile, configure `semantic_search.code_representation =
"symbol_body_v1"` and rebuild. No embedding default was changed by this repair.

## English indexing performance

Three alternating paired trials compared the frozen original RC4 executable with
the frozen repair, using identical sources, isolated indexes, cached MiniLM and
pipe-driven completion. All 12 profile/phase checks passed the existing **15%
time / 20% RSS and index-size** tripwires. Force-rebuild totals matched in every
pair: graph-only **384 files / 11,556 symbols / 18,403 relationships**; local
semantic **8 files / 505 symbols / 607 relationships**.

| Median time change | Graph-only | Cached MiniLM, body-aware |
| --- | ---: | ---: |
| Cold | −1.3% | +5.8% |
| No-op | −64.7% | −38.7% |
| Create | +1.1% | −6.6% |
| Body edit | −0.8% | −9.2% |
| Delete | −2.6% | +3.2% |
| Force rebuild | −1.0% | +1.1% |

Maximum measured RSS increase was 9.1%; maximum median index-size increase was
0.7%. Subsecond no-op samples varied substantially, so their negative deltas are
not general speedup claims. Raw ranges, RSS, index sizes, commands, source/model
hashes and unchanged count checks are retained in `performance/results.json`
and `performance/tripwires.json`. These local corpora do not establish full
Assign-scale performance or document quality.

## Disk hygiene

After all build/test processes finished, `lsof` found no open files in the
selected output directories. Removed only session-created reproducible outputs:

- `/Users/bpstr/Github/codanna/target/debug`
- `/Users/bpstr/Github/codanna/target/doc`
- 1,022 `.rlib`/`.rmeta` files created by this release build under
  `/Users/bpstr/Github/codanna/target/release/deps`; exact removed paths are in
  `release-archive-cleanup.json` under the evidence root.

Observed available space increased by **28.882 GiB**, leaving **50.345 GiB**.
Byte counts and checks are retained in `disk-hygiene.json`. Frozen binaries,
source, Git history, indexes, model assets and qualification evidence were
preserved; older release outputs remain.

## Multilingual follow-up

The user deferred multilingual work and requested English performance first.
The original default-model bilingual qualification remains failed: English
body-aware retrieval found 11/11 owners with MRR@5 0.955; Hungarian found 4/11
with MRR@5 0.318. These are historical measurements, not new repair passes.

An isolated exploratory E5Small run before that scope change found English 11/11
(MRR@5 0.791) and Hungarian 7/11 (MRR@5 0.591); combined 18/22 (MRR@5 0.691)
failed the unchanged bilingual targets. Its model-specific input changes were
restored and are absent from this patch. Experimental evidence is retained under
`bilingual-e5-small-2/`; it does not qualify a shipped multilingual configuration.
No multilingual support or default-model change is claimed.

The full Assign document stage and remaining release activation gates still need
qualification on a rebuilt candidate. Passing this focused repair does not turn
RC4's original release receipt into a go decision.
