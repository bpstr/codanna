# Evidence retrieval verification — 2026-09-26

Baseline source: `4c70ac20aa788697206e5a04ed8938582cf4016c`.
Raw logs: `/Users/bpstr/codanna-evidence-v1-20260926`.

The eight accepted changes have implementations and deterministic regression
fixtures. No paid inference, model download, production index rebuild, or live
model grading was performed. Fixture results measure retrieval contracts and
source selection; they do not establish real-model quality, production recall,
latency, or exhaustive repository coverage.

## Before and after

The first two behavioral tests were executed against the baseline before implementation:
**0 passed / 2 failed**. The baseline rejects coverage request fields and reports
scoped semantic retrieval as unsupported. Original failures are retained in
`before.txt`. After implementation both tests pass; the checked-in `fixtures-before.txt` retains
the baseline failures.

Additional fixtures compare old and new behavior on identical local data.
`measurements.json` records these results with fixture names and limits:

| Fixture | Before | After | Meaning |
| --- | --- | --- | --- |
| Scoped target below 40 stronger out-of-scope parents | Target absent from global top 32; postfilter cannot recover it | Target first in scoped results | Fixed-vector candidate recall 0/1 → 1/1; not model relevance |
| Long-function middle marker | Head/tail retained text omits marker | V2 retains marker, including final embedding inputs | Source-evidence retention improves within the same byte/segment caps |
| Nineteen candidate coverage rows | One ten-result page exposes ten rows | Two pages expose all nineteen | Candidate-pool pagination, not repository completeness |
| Reverse-consumer fixture | Default one-result query returns the matched implementation | Coverage includes the implementation and 17 incoming consumers | Indexed one-hop consumer discovery |
| Documentation reference beyond displayed preview | Empty lexical/preview-anchor candidates for the task wording | Stored explicit link admits the implementation | Persistent identity survives presentation truncation |

An intermediate implementation passed source-fragment retention but failed the
final-segment marker test. That failed run is retained in `after-expanded-2.txt`.
The repair selects the segment containing the source byte midpoint, rather than
assuming the middle segment index represents the source midpoint.

The first full CI run exposed a nondeterministic owner fixture: it appended a
second symbol record with changed visibility rather than constructing one public
symbol. The correction creates that record once and asserts the exact expected
owner. The original failure is retained in `ci-reliability.txt`; this was a fixture
repair, not a weakened owner assertion.

The first full default-feature run also found an existing before/after measurement
helper that checked reader stability only for the second response. The first query
can omit facets when its reader changes, while the next stable query includes them.
The repair requires concrete, equal reader generations within and across the pair,
records explicit drift and warnings, preserves the three-attempt cap, and retains
full response-item equality. It never retries a result difference on its own.
The reviewer confirmed this as a measurement repair; `ci-full-first.txt` retains
the original failure. The same policy now covers scoped comparisons. A separate
cross-process comparison omits facets only for numeric CLI reader drift with all
facets empty and an explicit skipped-enrichment warning; it asserts the stable
response's expected facets and compares every other field. Provider-input counts
and index-immutability assertions remain intact. The corresponding failed runs
are retained in `ci-contract-failure.txt`, `ci-contract-failure-2.txt`, and
`ci-contract-failure-3.txt`. These checks now share the same guarded helper so
source-policy mismatch and corrupt-metadata paths retain identical comparison rules.

## Negative and compatibility contracts

- Scoped semantic snapshots retain language filtering and parent-segment aggregation;
  query filtering does not modify the live vector corpus.
- Persistent links reject stale filesystem content, fresh graph content with stale
  indexed file hashes, ambiguous targets and provisional candidate edges.
- Incoming References remain distinct from Calls, and out-of-scope endpoints are excluded.
- Private callers do not displace a direct owner candidate merely because they call it;
  impact can still return them as dependents.
- Coverage fingerprints reject changed queries/readers; missing facets remain unknown.
- Explicit facet filters admit observed matches without depending on prose similarity.
- V1 representation identity remains unchanged; V2 cannot reuse V1 vectors.
- Diagnostics distinguish absent policy from mismatch without initializing a provider.
- Existing prepared HTTP/CLI contracts now exercise scoped semantic success and retain
  their no-index-write and query-input assertions.

The independent quality reviewer inspected source and test contracts and reported
no remaining correctness blocker. The reviewer did not run tests; executed gates
are recorded separately below.

## Gate results

Validated code and tests: `189facea026d8fff512b4504883a71879a8c2814`.
The final accompanying commit changes evidence files only.

- Fourteen feature regressions and five comparison guards passed in CI; see
  `fixtures-after.txt` (19 unique tests, deduplicated across targets/configurations).
- All-feature library suite: 1,510 passed, zero failed, 27 ignored.
- Local quick gate passed. Local full-gate formatting, strict all-target/all-feature
  Clippy and no-default-feature checks passed. Test compilation was interrupted
  under memory pressure, then stopped when its source version was superseded by
  the corrected fixture. No completed local full-gate pass is claimed.
- Full CI suite at `78c43b1`: default features 2,590 passed; all features 2,592
  passed; zero failures; 62 ignored per configuration. These are summed test
  executions across 59 targets, not counts of unique behaviors. CLI help checks
  and the strict documentation build passed.
- Five focused comparison-helper tests passed locally after the test-only cleanup.
  They reject score changes, missing warnings, stable-reader facet omissions and
  empty result pairs; an explicit-drift positive case passes on either side.
- Final CI at `189face`: **22 checks passed, two intentionally skipped**. The full
  default-feature suite passed 2,600 test executions; all features passed
  2,602; zero failures; 62 ignored per configuration across 59 targets. CLI help
  checks, strict documentation build, formatting, strict Clippy and the
  no-default-features build passed. See [the full run](https://github.com/bpstr/codanna/actions/runs/36246880279/job/108417603844).
- `validation.json` records commit-specific outcomes, source/log hashes and cleanup
  evidence. The evidence-only commit does not alter the tested Rust files.

Local tests ran with an environment allowlist, offline Cargo/Hugging Face settings,
no provider credentials, and prepared transports. The local toolchain-update step
was suppressed so validation used the installed Rust 1.97.1 compiler. CI used its
normal isolated runners and the repository's deterministic tests.

## Disk hygiene

Only newly created, inactive files below `target/debug` and the standalone
test-helper binary were removed. Build paths were compared against the pre-session
manifest, and all cleanup candidates were checked for open processes.
Source, indexes, model caches, release output, pre-existing build files and all
verification evidence were preserved.

Three cleanup operations removed 2,985 files: 8,149,038,257 logical bytes
(7.59 GiB). Observed free-space increases during those operations totaled
6,145,851,392 bytes (5.72 GiB); logical sizes differ from physical APFS
recovery. The final data-volume reading was 22,983,110,656 bytes (21.40 GiB) free.
Exact removed paths and per-operation measurements are retained in
`cleanup-first.json`, `cleanup-final.json`, and `cleanup-helper.json` in the raw
evidence directory, with hashes in `validation.json`. The third operation removed
only the standalone test-helper binary after its five tests completed.

The data volume remains below the 50 GiB guideline. Pre-existing candidates include
`target/debug/deps` (8.8 GiB), `target/debug/incremental` (4.7 GiB),
`target/debug/build` (4.2 GiB), and `target/release` (4.7 GiB). Removing those would
expand cleanup beyond session-created artifacts and requires separate approval.
