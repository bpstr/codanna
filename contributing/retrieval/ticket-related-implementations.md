# Bounded related implementation evidence

Follow-up to #58/#59 task-relevance findings, stacked on #56. The original
20 questions and ten implementation-owner judgments remain unchanged.
No paid embedding, production corpus rebuild, generated synonyms, or changed
source annotations are introduced by this feature or its tests.

## Contract

`search_ticket_context` gains `include_related_code`, a strict boolean defaulting
to false. It does not change direct code candidates, rank-fusion scores, ordering,
or requested code limits. Related results are separate evidence, not upgraded
relevance scores or newly invented direct matches.

An opted-in request examines outgoing indexed `Calls` from at most three distinct
returned direct matches, with a hard 32-edge budget per seed and six returned
related symbols. It traverses one hop only, deduplicates target IDs, excludes
already-returned direct IDs, and records seed rank/identity plus call-site
coordinates when available. A relation demonstrates an indexed call, not
ownership, semantic relevance, source completeness, or current source freshness.

Scope must never silently broaden. Until graph filtering shares the verified
subtree machinery, a request with `code_path_prefix` reports
`not_run_scoped_graph_unsupported`. No graph queries run for that case. Disabled
requests retain the previous response shape and perform no graph work.

A single pinned GraphView supplies relationship reads and symbol hydration.
Reader generation observations bind the expansion to the direct-query generation;
changes discard related results. Unhydrated endpoints, unavailable storage,
budget exhaustion, and legitimate empty neighborhoods are distinct outcomes.
Warnings and retained evidence are bounded. This expansion initializes no
model/provider; existing document search keeps its own configured backend policy.

## Use

MCP arguments:

```json
{
  "query": "kill a stalled conversation search subprocess after its deadline",
  "code_limit": 5,
  "include_related_code": true
}
```

Direct CLI invocation against an already indexed workspace:

```bash
codanna mcp search_ticket_context --args '{"query":"kill a stalled conversation search subprocess after its deadline","code_limit":5,"include_related_code":true}' --json
```

No reindex is needed to enable this query-time option. Existing indexed edge
coverage determines what can be returned. It cannot reconstruct missing Calls
or recover a query with no direct seeds by inventing a connection.

The normal `code.items` remain unchanged. Opted-in structured output additionally
contains `code.related_code`, with a status, bounded per-seed probe outcomes,
related items and `via` provenance. Text renders the same related identities and
available call-site evidence in a separate section. Related rows deliberately
have no `fusion_score` or confidence field. Their order follows direct seed rank
and deterministic source location, not a new semantic relevance score.

## Executed verification

The staged run executed 38 selected tests, including the seven new library
contracts, two actual CLI tests and one 20-query handler measurement. It then
passed formatting and strict all-target/all-feature Clippy. Exact-source hashes
and before/after metric interpretation are in
[ticket-related-implementations-results.md](ticket-related-implementations-results.md).
The final committed tree uses those tested blobs; the retained workflow reruns
it without a patch or write permission.

- [x] Exact local Calls with source/target identity and call-site evidence.
- [x] Duplicate edges, multiple seeds, direct-result exclusion and cycles.
- [x] Empty, missing seed, dangling target and exhausted edge budget states.
- [x] Disabled/scoped requests short-circuit; pre-expansion generation mismatch has no related rows.
- [x] Strict boolean schema, default compatibility and unchanged direct results.
- [x] CLI/MCP success, error and unsupported-scope contracts.
- [x] Frozen task corpus reports direct Hit@5 separately from direct-or-related recall.
- [ ] Deterministic concurrent replacement during this collector's full traversal.
- [ ] Scoped graph expansion sharing the verified workspace snapshot filter.
- [ ] Combined #57/#59 and all-branch release qualification.

The original 90% retrieval quality target remains open. On this recorded base,
direct Hit@5 remains 6/20; finding the owner in up to five direct plus six related
results reaches 9/20. The three extra operational owners are `collect_all_files`,
`capture` and `render`; paraphrase retrieval does not improve. This larger
related-code surface is not renamed as a better Hit@5 result.

This branch uses #56's recorded lexical baseline; it does not claim to contain
all separately developed #59 ranking or #57 snapshot work. Results from those
independent branches must not be added together as a combined-build metric.
