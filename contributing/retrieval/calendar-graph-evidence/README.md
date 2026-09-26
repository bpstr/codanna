# Calendar graph evidence regressions

A small implementation slice of [PR #43](https://github.com/bpstr/codanna/pull/43):
T01's graph-contract reproduction and T02's misleading empty answers. This is
not the full lexical, semantic, JSX, routing, or lifecycle acceptance corpus.

## Frozen input

[cases.json](cases.json) records the source revision, SHA-256 digests, selectors,
and expected contracts. Only its `corpus_root` is indexed; this document and the
oracle remain outside it. The fixture is hand-authored, not copied from Assign.
Semantic indexing is disabled and no model, provider, or transcript is used.

Resolve each symbol by relative path, exact name, and kind in the current index.
The duplicate `isolatedCalendarToken` in `reference/` has a caller; the active
one does not. A global-name or stale-ID lookup must not mix those definitions.
The browser call is syntactically present but its external target is not indexed.
Empty Calls evidence must not claim that the source contains no calls.

## Verification state

The fixture checkpoint has not been executed. `execution_status: not_run`,
unknown binary digest, and unknown generation are deliberate, not passing
evidence. The reviewed runtime source is pinned to
`ddb5ae61a72938d82cceaf42123dc7a88bfe3417`; reading it is not a runtime baseline.

The expected pre-fix defects are source-confirmed in `src/mcp/tools/symbols.rs`:
empty calls say the function does not call anything, and empty impact says no
symbol would be affected. These are not recorded as newly observed responses.

Subsequent regression tests must exercise the public MCP methods, distinguish
missing/ambiguous symbols and budget errors from successful empty traversals,
and retain source-coverage/freshness uncertainty. The full T01 baseline, JSX
resolution, persistence/watch parity, and ranking experiments remain open.
