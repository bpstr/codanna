# Bounded related implementation evidence

Follow-up to #58/#59 task-relevance findings, stacked on #56. The original
20 questions and ten implementation-owner judgments remain unchanged.
No paid embedding, corpus rebuild, generated synonyms, or source annotations.

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
Warnings and retained evidence are bounded. No model/provider is initialized.

## Verification plan

- [ ] Exact local Calls with source/target identity and call-site evidence.
- [ ] Duplicate edges, multiple seeds, direct-result exclusion and cycles.
- [ ] Empty, missing seed, dangling target and exhausted edge budget states.
- [ ] Disabled/scoped requests do not traverse; generation mismatch discards evidence.
- [ ] Strict request schema, default compatibility and unchanged direct results.
- [ ] CLI/MCP contract and structured/text sidecar parity.
- [ ] Frozen task corpus reports direct Hit@5 separately from direct-or-related recall.
- [ ] Focused tests and wider compatibility checks on the committed source.

The 90% original quality target remains open. Recovering an owner somewhere in
five direct plus six related results is not the same metric as improving Hit@5.
This branch initially uses #56's recorded lexical baseline; it does not claim to
contain all the separately developed #59 ranking or #57 snapshot work.
