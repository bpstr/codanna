# Definition lookup by ID

`find_symbol` accepts a positive `symbol_id` directly. Use an ID returned by
discovery in the same workspace to request the corresponding definition:

```json
{"symbol_id": 123}
```

Name queries and the legacy `{"name":"symbol_id:123"}` form remain supported.
Language and pagination filters apply to both forms. Unknown IDs and mismatched
languages return zero matches; they never fall back to same-name definitions.
Conflicting targets, zero IDs and requests with no target are invalid.
IDs belong to one index/workspace and are not stable identities across rebuilds.
An ID with multiple live definitions is an index error with a rebuild instruction.
Name results retain their selected definition and omit context that cannot be
verified against that definition. Exact-name pages retain bounded ordering state
and report the filtered total; restart paging after reindexing.
Responses contain indexed signature, documentation, locations and relationship
context, rather than complete implementation bodies. Indexed coordinates can be
stale; ID disambiguation does not establish source freshness.

The deterministic `tests/find_symbol_id_contracts.rs` suite covers duplicate
names, typed/legacy equivalence, ID and language exclusions, pagination, malformed
requests and schema discoverability. CLI fixtures cover both text and JSON
rendering, including an explicitly empty name alongside a typed ID. Semantic
search is disabled; no model or provider is required.

## Remaining findings

The code-semantic endpoints now add a structured `retrieval` record to successful
and empty responses. It identifies the code-symbol corpus, requested/effective
floor, configured floor, post-top-K filter stage and unassessed support. Omitting
a floor preserves existing nearest-neighbor behavior; the configured value is
not implicitly applied. Graph counts describe distinct indexed symbols, with
call-site totals and unresolved/external-call coverage explicitly unreported.
Impact metadata remains present alongside the new record. A deterministic unit
test verifies the distinction between omitted and explicit score floors. MCP
responses expose this record as `structured_content.retrieval`; one-shot CLI JSON
exposes it as `meta.retrieval`, including empty results. Prepared endpoint and CLI
fixtures cover omitted and explicit floors without invoking a paid provider.

The object-method emission fix is tracked separately. Other open work needs
independent evidence before broad changes:

| Area | Prepared next step | Acceptance boundary |
| --- | --- | --- |
| Document support | Add literal/phrase controls and explicitly separate similarity from support | Negative controls must not certify nearest neighbors as authoritative evidence |
| Lazy components | Synthetic dynamic import, promise cache, exported-member projection and JSX consumers with decoys | Composition evidence must remain separate from immediate Calls |
| Interface dispatch | Go interface/provider fixtures with multiple implementations | Candidate dependencies must not assert definite runtime callees |
| Python protocol usage | Imported protocol inside cast with shadowing and aliases | Type/reference evidence must not masquerade as Calls |
| Freshness | Publication/edit/reopen fixtures, per-file hashes and generation observations | Do not enable semantic watching by simply relaxing the existing mode gate |
| Representation | Deterministic body/comment inputs and prepared query vectors | No model change or full corpus rebuild until coverage and retained-input costs are measured |
| Resolver bindings | Isolated multi-project roots and real resolver fixtures | Preserve deliberate exclusions and workspace boundaries |
| Storage/scaling | Frozen corpus and explicit future resource budget | No compaction, storage reclamation or scaling claim from duplicate stored/live totals alone |

Validation uses temporary indexes and prepared data. Existing user indexes,
watcher modes and model configuration remain unchanged.
