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
`before.txt`. After implementation both tests pass.

Additional fixtures compare old and new behavior on identical local data:

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

Verification in progress. See the final `validation.json` for command outcomes,
source hashes, retained log hashes and disk cleanup evidence.
