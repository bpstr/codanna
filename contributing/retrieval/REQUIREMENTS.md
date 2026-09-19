# Retrieval quality requirements and implementation audit

Inspected `main` at `f776840bf9a2a6241c0b62975ea2bf4e44e93e0d`.
Findings below come from source inspection, not a measured Codanna execution.
This change adds a corpus and evaluator; it does not repair the production engine.

## 1. Important retrieval paths and weak points

| Finding | Evidence in the inspected source | Cases |
| --- | --- | --- |
| Embedding-free document lookup is not a lexical search. Candidate selection filters by type/collection/path, never query text; fallback takes these candidates with zero similarity. | `src/documents/store.rs`: `search`, `get_filtered_candidates`, `enrich_results` (approximately 765-810 and 1248-1350). | D01-D04, D06 |
| Vector document search is limited to 10,000 preselected candidates; the cap precedes similarity scoring. No per-document diversity stage is visible. | `src/documents/store.rs`: `TopDocs::with_limit(10_000)`, `score_by_similarity`, `search`. | D06, M08 |
| Knowledge context has lexical substring seeds, a cap of 12 seeds and ID-ordered undirected breadth-first expansion. It is not semantic retrieval or relevance-ranked graph expansion. | `src/knowledge/context.rs`: `get`, `adjacency`. | C01-C05, S01-S02 |
| Explicit links handle a conservative Markdown subset. Reference-style links, setext headings and percent-encoded filenames are not resolved like normal inline links. Unknown inline literals deliberately are not errors. | `src/knowledge/links.rs`: `build`, `local_target`, inline-link/tick regexes. | G07-G15 |
| A WHY/NOTE comment before a declaration can attach to the file rather than the following symbol because ownership is determined by source-range containment. | `src/knowledge/links.rs`: `owners` selection and `rationale_comment`. | G05-G06 |
| A valid source `#L4-L7` link resolves to the file node, not a retained target line range. | `src/knowledge/links.rs`: line-fragment branch in `build`. | G11, M05 |
| Knowledge input reads code files represented by symbols plus document files. A symbol-free TOML/JSON configuration can be absent despite being linked from a document. | `src/knowledge/io.rs`: `input`, construction of `paths`. | G17, M06 |
| Query surfaces do not have identical mutation behavior. One-shot `mcp search_documents --json` auto-indexes collections; the persistent MCP handler explicitly avoids auto-index mutations. | `src/cli/commands/mcp.rs`: `search_documents_data`; `src/mcp/tools/search.rs`: `search_documents`. | M03 |
| Document unchanged-file detection trusts matching mtimes before hashing. Knowledge snapshots are explicitly not live source. | `src/documents/store.rs`: `detect_changes`; `src/knowledge/context.rs`: `freshness`; `src/knowledge/service.rs`: pinned graph. | M01-M02 |
| A symbol-doc search is not document-collection search. CLI JSON shapes and source coordinates must be evaluated separately. | `src/mcp/tools/search.rs`: `semantic_search_docs`, `search_documents`; `src/cli/commands/documents.rs`: `Search`. | D01-D06 vs S01-S03 |

These are bounded observations. They do not establish failure rates, latency,
multilingual quality or the behavior of uninspected branches. In particular,
parser alias/call resolution is evaluated from real dumps rather than inferred
from function names. The companion inherits static resolution and must not turn
semantic similarity into a confirmed dependency.

## 2. Corpus design

The primary workspace models attachment admission and replay-safe delivery. The
server boundary is eight mebibytes, while an old document says sixteen and one
mobile implementation still uses seven. The PHP and web guards encode the same
rule differently. A standalone configuration has a differently named setting but
no explicit runtime wiring. A deletion implementation has no docstring; its purpose
appears only in a linked policy document.

There are two unrelated `validate` functions, an imported alias, a direct call,
a call cycle, actual boundary/replay tests, decisions referenced inside and before
functions, duplicate headings, Unicode, a fact beyond a long document's prefix,
broken references and fenced examples that must not become edges. Ignored files
contain obvious synthetic exclusion canaries. The peer workspace repeats a symbol
and decision identifier but uses a different boundary and its own canary.

The target is a useful change answer: implementation + authoritative rationale +
validation references, with explicit uncertainty where the system lacks evidence.
Returning a plausible function name alone is not enough.

## 3. Executable contracts

| Cases | Contract |
| --- | --- |
| G01-G03 | Index real symbols; preserve a direct call and an import-alias call without confusing cross-language names. |
| G04-G06 | Follow decision-to-symbol references and rationale ownership in both comment positions. |
| G07-G10 | Preserve ambiguity; reject fabricated symbol/decision edges; exclude fenced examples; retain genuine broken references. |
| G11-G15 | Resolve relative links, duplicate and Unicode anchors, reference-style Markdown and encoded filenames. Unsupported forms remain visible targets. |
| G16-G18 | Exclude ignored content, discover explicitly referenced configuration, and keep independently indexed workspaces separate. |
| C01-C02 | Return code, policy and static test evidence within hard node/byte limits; no dangling edges after truncation. |
| C03-C05 | Retrieve intent without a supplied symbol, expand across all explicit entrypoints, and terminate cycles. |
| D01-D06 | Retrieve actual document evidence, late facts and multiple useful documents; honor collection scope and an absent lexical term. |
| S01-S03 | Find paraphrased behavior, undocumented code and the correct domain among colliding names. |

Every assertion uses a fresh graph or an actual CLI query against the copied
workspace. Numeric IDs may change. Graph checks name edge direction, relation,
method and basis where relevant. A name collision remains unresolved with
candidates rather than becoming an invented exact edge.

## 4. Initial minimums

These are proposed minimums to establish and improve against, not measured scores.

**Correctness:** 100% of invariant cases pass. No cross-workspace or ignored source
may appear. All returned evidence must map to valid UTF-8/source coordinates and
current fixture hashes. Graph endpoints must exist; candidate associations must
not be promoted to established context edges. Truncation must be visible and
respect the requested bound, including metadata and escaping.

**Ranked positive queries:** at most five results are examined. Hit@5 is one when
at least one required evidence item occurs in those results. MRR@5 is the mean of
`1 / first_relevant_rank`, or zero for a miss. Required-evidence recall is the
fraction of each query's gold evidence found in its first five results, averaged
across positive queries. Initial floors are 0.90, 0.75 and 0.90 respectively.
D06 requires both the authoritative policy and the operational guide. The corpus
is small, so percentages move in coarse steps; record counts as well as averages.

**Per-case assertions:** required facts must be in the returned chunk, not merely
elsewhere in that file. Some cases require rank <= 3. All required evidence,
diversity constraints and forbidden-result checks must pass independently of the
aggregate floors. A good average cannot excuse a hard failure. Empty-query or
unknown-collection controls are not included in positive-query averages.

**Errors and unmeasured work:** unavailable embeddings, startup failures, malformed
JSON and protocol errors cannot be scored as empty results or successful skips.
Failed positive retrievals count as zero in the denominator. Structural-only runs
report semantic metrics as unmeasured. Reports always retain the pending manual
scenarios and never claim complete product qualification.

**Performance:** record per-command wall time, including process/model startup.
No warm MCP latency or speedup is inferred from one-shot CLI timings. Before setting
latency gates, run fixed cold/warm repetitions on a documented machine, keep model,
corpus, configuration and candidate budget constant, and separate indexing time
from query time. M08 is the scale witness; this small fixture is not a throughput
benchmark. Do not add model downloads to a required GitHub CI gate.

## 5. Local mutation and protocol qualification

Use the retained `primary/` and `peer/` directories, never production projects.
Re-index explicitly between stages and retain each generation's raw responses.
M01-M10 in `cases.json` are the authoritative manual requirements.

For M01, query the eight-mebibyte boundary, edit the implementation and decision,
then rename or remove the linked source. Compare the old snapshot with refreshed
code, documents and knowledge. Stale IDs and chunks must not survive a successful
refresh as current evidence. `codanna-knowledge check --graph GRAPH --root
primary=ROOT --fail-on-review` can inspect knowledge drift; it is not a substitute
for refreshing the code/document indexes.

For M02, replace `eight` with `seven` in a disposable document, preserve its original
mtime with `os.utime`, and re-index without forcing. Compare contents, retrieved
fact and reported freshness. A metadata-only check cannot claim hash-verified
freshness. Do not hide this witness by always using `--force`.

For M03, compare standalone `documents search --json`, one-shot `mcp
search_documents query:... --json`, and a persistent MCP tool call. Record the
index tree before and after each query, not just the answer. Keep generation and
collection filters equal. Any intentional refresh difference needs a documented
contract, not a misleading parity claim.

For M04, use the workspace-routing implementation from PR #34 with independent
client roots/cwds. Query both directions, close/reopen sessions, and repeat identical
names. A shared MCP entry must not leak the other root's symbol, fact or canary.
Separately building two indexes (G18) is necessary but does not qualify routing.

For M05-M07, inspect source-fragment precision, ask for a twelve-mebibyte policy
change across the server/clients/config/tests, and repeat an intent query in
Hungarian. Distinguish an explicit policy association from a resolved call, and
an English embedding limitation from a broken Unicode path/span.

For M08, generate more than 10,000 distractor chunks in a disposable collection
and vary insertion order. Count actual indexed chunks, then verify whether the
late relevant item remains reachable. Do not merely generate 10,001 short files
and assume one chunk per file. Record candidate truncation and ranking variance.

For M09-M10, test outside-root symlinks/references, newly ignored files, invalid
limits/filters and an unavailable local model, followed by a healthy request.
No real secrets or external targets are needed; the peer is the outside-root
witness. Protocol errors and reader recovery are separate from relevance scoring.

## 6. Delivery sequence

1. Run structural and lexical profiles against the pinned binary, preserve the raw
   failure report, then run the local semantic profile when its model is available.
2. Repair query-aware lexical candidate selection and retrieval-surface parity
   first. Keep missing-model errors explicit and verify the negative controls.
3. Improve explicit Markdown/config/rationale evidence and alias resolution with
   narrow parser/graph tests. Preserve ambiguity and source provenance.
4. Add measured semantic-to-graph seed expansion, evidence-aware selection and
   document diversity. Never create graph edges merely because vectors are close.
5. Qualify refresh, true workspace routing, candidate saturation and stable warm
   latency using the manual scenarios. Promote deterministic regressions into
   existing Rust tests where useful; retain local semantic acceptance separately.

Do not edit production code, lower minimums, bless a failing output as golden, or
mark unsupported targets as passing merely to merge this fixture PR. Review target
scope explicitly when a capability is deliberately deferred.
