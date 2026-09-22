# Ticket-aware code candidate fusion

## Scope and dependency boundary

Finding B distinguishes multi-source context from ticket-aware code fusion. This implementation is stacked on #53 and uses its pre-ranking lexical subtree filter and #52 ranking. #55 owns body representations; #44/#46/#48 own graph reporting/extraction. No dependency is merged here.

`search_context` remains unchanged. The explicit companion tool is `search_ticket_context`, registered in the generated MCP router, typed tool catalog, read authorization, direct CLI dispatcher and workspace reader. Existing scope parameters are included in the catalog vocabulary so schema/catalog checks cannot silently miss them.

```json
{
  "query": "calendar account preferences",
  "code_limit": 5,
  "document_limit": 3,
  "code_path_prefix": "src/active",
  "include_semantic_code": false,
  "include_conversations": false
}
```

```sh
codanna mcp search_ticket_context query:calendar code_limit:5 --json
```

Code/document/conversation limits accept integers in 1–10. Query text is trimmed and limited to 512 UTF-8 bytes. Unknown fields and escaping/absolute/empty subtree paths are rejected before retrieval. A subtree applies only to code; `collection` independently selects document scope.

## Retrieval contract

The new tool collects bounded lexical code candidates and optionally semantic code candidates. It deduplicates real symbol IDs, then adds reciprocal-rank contributions with `k = 60`. Raw scores are diagnostic only. Repeated evidence from one channel cannot amplify that channel's contribution. Exact lexical identifier matches retain priority. Remaining ties use path, line and symbol ID; the final output limit applies after fusion.

Document previews may contribute single-backtick bare ASCII identifiers. Only an exact indexed name match can admit a code candidate. Arbitrary prose, query syntax, fenced code, escaped backticks, qualified expressions and route/config/event strings are not promoted to queries. Conversations never generate anchors. Every admitted anchor records document rank, path and byte offsets into the returned preview; these are not original-document offsets or proof of current ownership.

Initial budgets are 64 lexical candidates, 32 semantic candidates, eight unique anchors and 16 candidates per anchor lookup. Anchor scanning reads at most three returned previews and 8 KiB total. Each returned document preview is bounded to 4 KiB on UTF-8 boundaries; up to the requested ten documents can be returned. Indexed signatures, headings, names and paths are bounded in the response. These are byte budgets, not token estimates.

Semantic code retrieval and conversation recall default off. Opting into semantic code can prepare the existing index's configured query backend and perform one logical query embedding; backend retry behavior is not a one-HTTP-request guarantee. It does not rebuild source vectors or generate summaries. Document search independently uses its existing configured query backend and may perform its own embedding operation.

The current semantic API has no pre-ranking subtree filter. When `code_path_prefix` is present, semantic code is explicitly `not_run_scoped_semantic_unsupported`, with no global-search fallback or backend preparation. Lexical and exact-name anchor searches retain #53's pre-ranking scope semantics.

## Failure and evidence boundaries

The response contains separate code, document and optional conversation sections plus structured provenance. Missing/failed semantic retrieval preserves lexical and anchor results. Document search errors or a busy document snapshot preserve code results. Blocking-worker failure is an unavailable channel, not a fabricated empty result.

Reader-generation observations can disclose a code reader change, but are not source revisions. Cross-source snapshots are not atomic. This tool does not traverse the graph: graph status is `not_run`, coverage/freshness remain `unknown`, and indexed source revision is null. No document mention creates `Calls`, `Uses`, or other graph edges. Rank-fusion scores are not confidence, correctness, or verified task relevance.

Workspace document-store initialization still follows the existing workspace reader's strict failure boundary before handler dispatch. That boundary is not the same as a document search failure inside a successfully initialized handler and requires separate transport-level failure coverage.

## Implementation and acceptance

Implemented:

- [x] Companion tool, unchanged legacy handler, typed validation, CLI/JSON and workspace routing.
- [x] Deterministic rank fusion, parent-ID deduplication, exact lexical pinning and bounded output.
- [x] Exact document identifier verification, bounded scanning and preview-local provenance.
- [x] Code subtree filtering on lexical/anchor channels; scoped semantics explicitly unsupported.
- [x] Independent handler channel statuses and structured evidence.
- [x] Provider-free helper, real-index, direct-handler, CLI and workspace-client regression tests authored.

Acceptance gates (test authorship is not execution evidence):

- [ ] Exact-source deterministic contracts, including nonempty test selection.
- [ ] MCP catalog/schema/guidance parity and legacy scope regressions.
- [ ] Formatting and strict all-target/all-feature Clippy.
- [ ] Additional workspace facility-loading failure injection.
- [ ] Route/event/config literal and qualified-name indexing before admitting those anchor types.
- [ ] Capped, explicitly approved held-out Assign ticket evaluation.

The `Ticket code fusion` workflow records the tested source SHA and logs. A filtered test command that runs zero tests must fail the contract gate. Synthetic mechanics are not production relevance evidence. No production rebuild, provider-cost experiment, or change to upstream is part of this PR.
