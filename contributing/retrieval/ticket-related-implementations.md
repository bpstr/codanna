# Bounded related implementation evidence

`search_ticket_context` accepts `include_related_code`, a strict boolean that
defaults to false. It does not change direct code candidates, rank-fusion scores,
ordering or requested code limits. Related results are separate evidence, not
upgraded relevance scores or proof of implementation ownership.

## Contract

An opted-in request examines outgoing indexed Calls from at most three distinct
returned direct matches, with a hard 32-edge budget per seed and six returned
related symbols. It traverses one hop, deduplicates target IDs, excludes direct
IDs and records seed rank/identity plus available call-site coordinates. It does
not recursively follow calls or award extra relevance votes for duplicate edges.

The #62 integration supports `code_path_prefix` for related results through the
same registered-file scope machinery as lexical search. Seeds and targets must
belong to the requested workspace subtree on the pinned graph snapshot. External
checkouts, missing file registrations and prefix/file-name neighbors cannot
silently broaden scope. Explicit `.` means the configured workspace. Budgets
are checked before filtering and are not relaxed to refill excluded results.

Pinned relationship reads, file-scope resolution and hydration use one GraphView.
Reader-generation observations bind expansion to direct retrieval; invalidation
discards related results. Unhydrated endpoints, scope exclusions, missing seeds,
unavailable storage, budget exhaustion and legitimate empty neighborhoods remain
distinguishable. Warnings and retained evidence are bounded.

The graph option initializes no model/provider; existing document search keeps
its own configured backend policy. It does not opt into semantic code search.

## Use

```json
{
  "query": "conversation timeout",
  "code_limit": 5,
  "code_path_prefix": "src",
  "include_related_code": true
}
```

```bash
codanna mcp search_ticket_context --args '{"query":"conversation timeout","code_limit":5,"code_path_prefix":"src","include_related_code":true}' --json
```

Omit `code_path_prefix` for unscoped discovery. No reindex is needed to enable
this query-time option, but existing indexed edge coverage determines what can
be returned. It cannot invent missing Calls or expand a query with no direct
seeds. Scope is a retrieval constraint, not an authorization boundary.

The normal `code.items` remain unchanged. Opted-in structured output adds
`code.related_code`: status, per-seed probes, related items and `via` provenance.
Scoped output also includes `path_prefix` and `excluded_by_scope` counts. Text
renders the same scope, identities and available call-site evidence. Related
rows deliberately have no fusion/confidence score; ordering follows direct
seed rank and deterministic source location.

## Verification

The original #61 baseline and its 38-test staged verification are retained in
[ticket-related-implementations-results.md](ticket-related-implementations-results.md).
Those results use the older lexical base and originally disabled scoped graph
expansion. They must not be read as current combined-build metrics.

Current integration and scoped before/after evidence are recorded in
[scoped-ticket-relevance.md](scoped-ticket-relevance.md). The actual combined
build retrieves the owner directly in 8/20 questions, and somewhere among five
direct plus up to six related results in 10/20. All direct arrays stay unchanged
by opt-in. Paraphrase performance remains 1/10. The expanded surface is not
improved Hit@5 and does not meet the original 90% quality target.

The scoped candidate passed 66 selected contracts, formatting and strict Clippy.
Its five new public-handler cases improved from 1 passing / 4 failing to all
five passing against unchanged fixture bytes. Final-head CI remains a separate
record on #62. Independent labels, controlled latency/RSS and all-branch release
qualification remain open; nothing here authorizes a production rebuild.
