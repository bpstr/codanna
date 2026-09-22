# Bounded multi-concept symbol ranking

This implementation follows the measured T05/T06 investigation in PR #51. It
changes lexical **symbol** discovery only; document ranking, exact symbol lookup,
semantic vectors and index schema are unchanged.

## Why the candidate pool changes

The frozen seven-query calendar/integration fixture showed that current
`TopDocs(limit)` was dropping useful owners before any higher-level relevance
logic could inspect them:

| Query family | Current Hit@5 | Labeled candidate rank in a 200 pool |
| --- | ---: | ---: |
| calendar settings | miss | 51 |
| integration binding | miss | 53 |
| workspace preferences | miss | 50 |

Across the seven cases, R0 was 4/7 Hit@5. Test-local distinct query-term coverage
with 4x or 8x overfetch remained 4/7; **16x reached 7/7**, as did a 200-candidate
pool. The implementation therefore chooses 16x rather than the larger fixed pool.

A subsequent 21-query TypeScript/Rust/Go evaluation recorded R0 at 11/21 Hit@5
and MRR@5 0.524, while the 80-candidate 16x coverage strategy reached 20/21 and MRR@5
0.952. The remaining owner was candidate rank 98, so the selected runtime keeps
16x but raises the small-query floor to 128. Its seven-query holdout was 7/7,
MRR@5 1.000. These synthetic metrics meet
the proposed PR #43 minimums but are not a production relevance score.

## Runtime policy

Only conservative simple multiword discovery text is reranked. Explicit Tantivy
syntax and single-term identifier lookup keep the existing order. For eligible
queries the lexical candidate limit is:

- 16 times the requested limit;
- at least 128 for small discovery requests;
- at most 200;
- never below the user's requested limit.

Filters remain part of the Tantivy query before candidate collection.

Candidates are ordered by the count of distinct meaningful query terms found in
their stored name, documentation, signature, context, module or file path. The
existing Tantivy score is the secondary tie-break, followed by deterministic
location/name/id fields. The requested limit is applied after reranking.

No learned scorer, provider request, embedding lookup or new index field is
introduced.

## Score interpretation

The `SearchResult.score` value remains Tantivy's raw lexical **candidate** score.
After coverage reranking it is not necessarily monotonic in displayed order and
must not be read as confidence. MCP text preserves the existing `Score:` /
`[score ...]` surface while labeling it as raw lexical candidate evidence, and
adds `Distinct query-term coverage: matched/total` for discovery queries.

## Compatibility controls

The regression corpus checks crowded generic names, limit=1 discovery, exact
identifier rank, absent terms, language filters, explicit query syntax bypass and
MCP diagnostics. Existing document-ranking and adversarial code-search tests run
alongside it.

The baseline/evaluation corpus remains in PR #51 so the implementation cannot
silently rewrite its own before-state.

## Remaining relevance work

This is the bounded R1 candidate/rerank change, not completion of every T05-T08
item. The evaluation still needs repeated latency/candidate-cost reporting and
the one remaining broader-corpus miss should be classified before further
ranking knobs are selected. Typo/camel/snake query-family reporting, explicit
path/subtree scope (T07), semantic eligibility/freshness (T08), and any later
lexical/semantic rank fusion remain separate measured changes.
