# Ticket-aware code candidate fusion

## Finding and dependency boundary

`search_context` currently asks separate lexical-code, document, and conversation searches the same query. Separate evidence sections are useful, but they are not lexical/semantic code fusion or document-assisted implementation discovery.

This PR addresses finding B. It is stacked on #53 (`fix/symbol-path-scope`, audited head `fd9161ecbac450d12d4864431e363f64923fb1c6`) to preserve subtree filtering and #52's candidate ranking. #55 separately changes semantic source representations; #44 handles graph evidence metadata, and #46/#48 handle extractor correctness. None of those changes is duplicated or merged here.

## Contract

Keep legacy lexical retrieval as the default. Add an explicitly requested hybrid code mode; it may issue one query embedding against an already configured semantic index, but it never rebuilds or generates code summaries. Keep code, document, and conversation evidence sections separate.

Collect bounded lexical and semantic candidate lists, then use reciprocal-rank fusion rather than adding incomparable raw lexical and cosine scores. Deduplicate by real symbol identity, aggregate before the final output limit, preserve provenance, and use deterministic tie-breaking. A ranking score is not confidence or verified task relevance.

Document assistance is a bounded second stage. Only identifiers actually present in retrieved document previews can become anchors, and they must resolve to exact indexed code identifiers before admission. Do not pass arbitrary document text into query syntax. Conversations never generate anchors. Route/config/event text is not verified merely because a document mentions it: admit it only when an indexed code field supplies literal evidence, otherwise leave it unverified.

Subtree constraints apply to every admitted code candidate. If semantic candidates are scoped after the vector pool is collected, report that candidate coverage is bounded/post-filtered; do not present it as the pre-ranking lexical scope guarantee from #53.

## Initial hard limits

- At most 64 lexical candidates and 64 semantic candidates.
- At most three document previews and 8 KiB of preview text total.
- At most eight document anchors, bounded literal lengths, and bounded candidates per lookup.
- Code/document/conversation output limits retain the existing 1–10 validation.
- One retrieval pass and one bounded anchor pass; no recursive retrieval and no LLM synthesis.

## Acceptance checklist

- [ ] Opt-in mode and unchanged legacy default with schema/request parsing tests.
- [ ] Deterministic rank fusion and parent deduplication before output limiting.
- [ ] Exact document identifier verification, bounded scanning, and injection-shaped decoys rejected.
- [ ] Code subtree filtering on all evidence channels without claiming complete semantic scoped recall.
- [ ] Independent availability/failure status and fallback to available evidence.
- [ ] Structured evidence provenance alongside the human-readable sections.
- [ ] Provider-free fixed-candidate tests and handler regressions; no production relevance claims.

## Evaluation boundary

Synthetic contracts test mechanics, not Assign ticket quality. Production acceptance still requires held-out tickets, expected owners, ablation of lexical/semantic/anchor channels, top-k metrics, failure analysis, and a separately approved query-cost cap. #52's synthetic results do not establish the quality of this new fusion policy.
