# Adversarial code-intelligence regressions

The [implementation report](IMPLEMENTATION.md) records the final validation,
compatibility notes and remaining limits. The separate
[lexical acceptance run](LEXICAL-RESULTS.md) retains all measured failures as well
as passes.

This suite makes the edge-case investigation reproducible inside the repository.
It contains **62 executable tests in seven modules**, **27 native source files**,
and a separate checklist of **21 prepared or proposed follow-ups**. The cases
cover source extraction, declaration identity, production resolution, incremental
indexing, search, and graph boundaries. All content is synthetic.

[CHECKLIST.md](CHECKLIST.md) describes every executed assertion, links its test
source, and shows baseline and implementation results separately.
[cases.json](cases.json) is the machine-readable inventory.
[baseline.json](baseline.json) preserves the investigation measurements.
[results.json](results.json) records implementation verification; `pending`
means that no later passing result has been recorded.

[ADDITIONAL-CHECKLIST.md](ADDITIONAL-CHECKLIST.md) and
[additional-fixtures.json](additional-fixtures.json) cover **99 new tests and two
revised existing tests** from implementing fixes and investigating document
embedding. They keep the original 62-case baseline stable, identify the separately
measured 16-case document baseline, and distinguish fixtures without a historical
baseline for their current assertions. The two revised Python resolver units
retain their former names and explain the assertion or fixture-evidence change;
they are not counted as newly added tests.
[FOLLOW-UP-COVERAGE.md](FOLLOW-UP-COVERAGE.md) maps all 21 original follow-ups to
their implemented portions, named regression cases, and remaining qualifications.

The implementation notes explain the evidence and remaining boundaries for
[language semantics](language-semantics.md),
[web export and property resolution](web-resolution.md),
[incremental indexing](index-lifecycle.md), and
[document embedding](document-embedding-review.md).

## Run and record results

From the repository root, use a test environment without provider credentials or
embedding overrides. The original 62 fixtures explicitly disable semantic search
when they create an index. They do not load models, call providers, run the
miniature source applications, or use a production index. Additional document
and semantic-context fixtures use deterministic mock vectors or a loopback HTTP
server to exercise configured embedding behavior without downloading models or
calling external providers.

```bash
python3 contributing/retrieval/adversarial/check.py --check
python3 contributing/retrieval/adversarial/check_additional.py --check
cargo test --locked --no-default-features --test adversarial_regressions -- --test-threads=1
```

The dedicated native Rust, Go, Python, TypeScript, JavaScript, and PHP parsers do
not need the generic language-pack parser archive. A build with cached native
dependencies can use `TSLP_OFFLINE=1` to suppress that archive download and add
Cargo's `--offline` flag. `--no-default-features` does not currently remove the
unconditional fastembed/ONNX build dependencies, even though these tests never
perform inference. These options do not establish generic-language coverage.

To retain a reviewable result, capture exactly this integration target with no
test filter. Keep the process log outside the checkout; only the measured result
summary and its hashes are committed. For example, in Bash:

```bash
set -o pipefail
adversarial_log="$(mktemp)"
cargo test --locked --no-default-features --test adversarial_regressions \
  -- --test-threads=1 2>&1 | tee "$adversarial_log"

python3 contributing/retrieval/adversarial/check.py \
  --record-log "$adversarial_log" --revision "$(git rev-parse HEAD)"
python3 contributing/retrieval/adversarial/check.py --check
```

Record immediately after running the tests, before modifying tested source. The
recorder stores the tested base revision, whether the worktree is dirty, hashes
of test sources, and a hash of the current production/test source tree. A dirty
worktree result is therefore distinguishable from a result for the base commit.
An incomplete log, ignored case, filtered case, repeated test, or mismatched
summary is rejected. A completed failing run remains a failing run; the recorder
does not convert failures to skips. `--record-log` also regenerates the checklist.

The repository's broader gates remain:

```bash
./contributing/scripts/quick-check.sh
./contributing/scripts/full-test.sh
```

The fixture checker verifies inventory consistency only. Running it does not
execute Rust tests or establish retrieval quality. The separate
[retrieval acceptance workspace](../README.md) retains its own 32 executable
case definitions and 10 manual scenarios.

## Baseline and interpretation

The investigation executed all 62 cases against
[`1968a6fc14c4d5c080ac91b5e74799a4b0aa0cc2`](https://github.com/bpstr/codanna/commit/1968a6fc14c4d5c080ac91b5e74799a4b0aa0cc2):
**10 passed, 52 failed desired-behavior assertions, and none were ignored.**
Every target compiled and completed. Several tests expose the same problem at
different layers; others specify a richer capability or an API consistency
contract. The failures are not 52 independent defects, and this deliberately
adversarial sample is not a measure of ordinary repository accuracy.

| Module | Measured layer | Tests | Baseline pass | Baseline fail |
| --- | --- | ---: | ---: | ---: |
| `index_lifecycle` | Real incremental discovery, indexing, and graph queries | 8 | 0 | 8 |
| `language_end_to_end` | Full Rust/Go/Python IndexFacade pipeline | 6 | 0 | 6 |
| `parser_edge_cases` | Native parsers, language behavior, inheritance helper | 16 | 1 | 15 |
| `resolver_stages` | Real parser output fed to production ResolveStage | 4 | 0 | 4 |
| `root_probes` | Call extraction, byte positions, formatting graph control | 7 | 4 | 3 |
| `search_graph` | Real storage/service contracts and graph controls | 6 | 2 | 4 |
| `web_parser` | TS/JS/PHP parsers, import binding, and three full graph tests | 15 | 3 | 12 |

The original positive controls protect useful behavior: complete dense graph
enumeration and paging, explicit graph budget errors, cycles and diamonds,
ordinary PHP receivers, parenthesized arrows, named imports, Rust call-site
positions, and the full TypeScript formatting graph.

The measurements have precise limits:

- The TypeScript same-line/split-line failures concern raw extraction records.
  Both layouts already passed the normalized complete-graph comparison.
- The original multi-root test failed in the callee-first iteration, before its
  reverse-order iteration. A later passing run must complete both orders.
- The original qualified PHP test failed its Extends assertion before reaching
  Implements. The latter was source-confirmed, not separately measured then.
- The malformed-ignore case proves that discovery accepted a partial rule error;
  it does not itself reproduce later symbol deletion.
- The Go implementation case identifies the Socket declaration and promoted
  Wrapper. Separate method-set tests are needed to establish pointer/value and
  cross-package semantics in detail.
- Semantic context overflow, live nested-ignore reconciliation, and barrel-export
  capabilities were initially source findings. Their follow-up checklist entries
  are not counted as executed baseline tests.

The integrated modules preserve the original assertions. Source-column helpers
now use the widened `u32` API, and the direct TypeScript binding fixture supplies
the explicit export facts now required by the production cache. The Go generic graph case also adds a negative
assertion rejecting `Number[int](1)` as a function call. Its earlier missing-call
failure happened before this additional check; no historical pass is claimed for
the new assertion. `baseline.json` retains hashes of the original detached source.

## Why these scenarios matter

The implementation sequence follows the consequence of a wrong answer.

1. **Preserve target identity.** Removing `Alpha.make` must not move its callers
   to `Beta.make`. A default import locally named `save` must target the default
   `persist` export. Explicit `Token` annotations must outrank the owner of a
   `Client.make_token` factory. An annotation on `self.worker` must not replace
   the type of bare local `worker`. Python normal and `super` dispatch must share
   C3 ordering. Same-name decoys make false concrete targets observable.
2. **Retain ordinary syntax and source evidence.** Bare-parameter arrows,
   nullsafe calls, PHP aliases/groups/promoted properties, generic callees, grouped
   Go parameters, and definition-time dependencies must produce the same useful
   facts as their ordinary equivalents. Distinct physical calls on one line must
   keep distinct spans, and extraction passes must agree on the same physical site.
3. **Make incremental state converge to fresh state.** Creating or restoring a
   missing module and changing only a re-export must repair unchanged consumers.
   Multi-root replay must not resurrect an old source ID. Subsecond file changes,
   ignore errors, and file limits must describe the actual admitted inventory.
4. **Make completeness visible.** Precise queries must remain reachable among
   more than 100 common names. Literal code fragments need matching analyzers.
   Language filters should agree for ID and name lookups. Graph bounds must be
   explicit, and optional context expansion must not discard valid search matches.
5. **Add richer capabilities with evidence.** Go method sets, export chains,
   property receivers, callback references, and framework registrations need
   explicit identities and provenance. A callback argument or registration is
   useful evidence without being a proven immediate call. Dynamic targets should
   retain an unresolved reason or candidate set when static evidence is insufficient.

For additional mutation fixtures, compare an incremental index of source state B
with a fresh index of B. Normalize graph IDs using file/module/owner/kind/signature
and source position appropriate to the case. Check required and forbidden edges,
both caller and callee directions, endpoint liveness, multiplicity, and evidence
spans. Replay selected deltas through one-file, directory, multi-root, watcher,
persisted/reopened, and warm-cache paths. The checklist explicitly separates
those proposed qualifications from already executed assertions.

## Source-coordinate compatibility

Columns are zero-based UTF-8 byte positions. `Range`, relationship metadata, and
search-result source columns use `u32`, allowing generated lines beyond the old
65,535-byte boundary. This is a Rust API type change. Existing Tantivy integer
fields and JSON numbers can represent the wider values; the compact symbol stays
32 bytes, and the storage schema does not need a version bump for this widening.

Emission semantics advance from v3 to v4 for the combined parser, export-slot,
reference, and coordinate changes. The existing CLI/MCP compatibility guard
rejects stale reads and requires a complete rebuild; an indexing command heals
the older generation instead of mixing old and corrected rows. The Tantivy
schema itself stays compatible. An old index may already contain wrapped
coordinates whose lost high bits cannot be recovered from storage. Rebuild the
configured source inventory to regenerate its evidence, for example:

```bash
codanna index src --force
```

The generated-line integration case checks extraction; the storage codec's
`long_source_columns_survive_storage_and_reload` unit test checks persistence.
