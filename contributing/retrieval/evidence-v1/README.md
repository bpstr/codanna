# Evidence retrieval profiles

The ticket retriever shares candidate identities, evidence and family-level rank
fusion through `src/retrieval`. Existing tools and the default `relevant` profile
remain available. Semantic queries do not rebuild indexes. All scores describe
retrieval contributions, not confidence or completeness.

## Scoped semantic retrieval

`include_semantic_code: true` now works with `code_path_prefix`. The engine derives
allowed parent symbol IDs using registered file identity in a pinned Tantivy view,
restricts semantic candidates before top-K, and hydrates from the same view.
Language/segment aggregation stays parent-based. A missing backend is reported as
unavailable, preserving other channels. Code/vector source-generation alignment
remains unknown; a pinned reader is not proof of current source freshness.

## Profiles and facets

```sh
codanna mcp search_ticket_context --args '{"query":"avatar","code_path_prefix":"src","profile":"implementation_owner","include_semantic_code":true}' --json
```

- `relevant`: existing exact-name preference and reciprocal-rank fusion.
- `implementation_owner`: expand incoming Calls/References; prefer public
  declarations reached through References, then public callers. These are owner
  candidates, not proven page owners. Private callers are not promoted solely
  because they call the result. Framework route/render ownership remains unknown.
- `impact`: prioritize indexed dependents by distinct matched seeds, then relevance.
- `coverage`: expand both directions and page the complete bounded candidate pool.
- `evidence_strength`: prioritize explicit links/resolved paths over similarity alone.

Graph expansion examines up to 64 seeds and 128 Calls/References per direction
per seed. Endpoint scope and hydration share one view. Responses retain relation,
direction and source line; missing endpoints, omitted edges/seeds and exclusions
are reported. Existing `include_related_code` remains the legacy outgoing Calls
appendix. New profile expansion participates in candidate fusion and ordering.

Every hydrated candidate includes independently observed `kind`, `language` (when
indexed), and `visibility` facets with source location and extractor version.
There is no inferred timezone/security/business classification. Missing facets
mean unknown. Explicit `facet_filters` require all selected values and also admit
up to 128 matching inventory candidates before fusion:

```json
{"query":"format","facet_filters":[{"facet":"language","value":"rust"},{"facet":"visibility","value":"public"}]}
```

No filter is implied by ordinary prose. The inventory fails explicitly above
100,000 scoped symbols; narrowing the scope is the recovery action.

## Persistent documentation links

Create a snapshot explicitly using the existing companion:

```sh
codanna-knowledge index --root . --repo my-repository --out .codanna/knowledge.json
```

Use `include_knowledge_links: true` and the exact `knowledge_repo` identity from
`.codanna/knowledge.json`. Queries never create or rebuild that file. The bridge
finds document/section seeds using their persistent labels/excerpts independently
of displayed previews. It considers up to 16 matching sections and 128 explicit
or resolved `references`, `describes`, or `implements` edges to code symbols.
Candidate edges are excluded. It validates snapshot structure, local source hashes,
repository identity, exact path/declaration start/name and the pinned code file hash.
Ambiguous, missing or stale mappings are reported instead of guessed. Repeated
links contribute one family vote. A hash check observes source at query time; it
cannot prevent a subsequent filesystem edit. Snapshot source collection and parser
completeness remain qualified.

## Coverage contract

```json
{"query":"avatar","profile":"coverage","coverage_limit":25,"coverage_offset":0}
```

`code.coverage` contains a stable fingerprint, total candidate count, page items,
next offset and limitations. Repeat the same request with `coverage_snapshot` and
`coverage_offset` for the next page. Changed candidates, query/scope or reader
identity require restarting pagination. The page budget is 1–100; it does not use
the interactive top-ten limit. Each item records path, structural directory family,
direct/consumer responsibility, unverified ownership and linked document IDs.
Test-source labels are path conventions, not executed test coverage.

This enumerates the bounded retrieved candidate pool and its indexed neighbors.
`repository_complete` is always false. Exhausting pages does not prove that every
implementation surface was found. Candidate-channel and traversal budgets, stale
links, missing endpoints and unknown parser/source coverage remain visible.

## Representation diagnostics and source selection

`get_index_info` includes configured and recorded source policy, match/mismatch
status and an explicit recovery action in JSON and text. Missing legacy policy is
unknown; documentation-only vectors do not imply implementation coverage.

The new opt-in policy is:

```toml
[semantic_search]
code_representation = "symbol_body_v2"
```

V2 retains head, middle and tail source fragments within the same 16 KiB/symbol,
1 MiB/file and eight-segment limits. Final segment selection also includes the
middle. Its context fields contain parser-observed language and declaration role,
with an explicit observation basis. It makes no model-generated behavior claims.
Each segment keeps the actual source byte range. V1 inputs and identities remain
unchanged; V2 has a new policy identity and requires an explicit rebuild before
use. Neither policy guarantees complete function coverage.

## Validation

See `RESULTS.md` for executed evidence. Tests use prepared local data and vectors,
not paid inference, provider grading or model downloads. Structural fixture recall
and coverage are not measurements of real-model relevance or production readiness.
