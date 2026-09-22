# Task relevance recovery: lexical evidence, not a model-quality claim

Follow-up to #58 for the original #43 relevance priority. The ten inspected
implementation-owner labels and 20 questions remain byte-identical. Corpus:
five unchanged Rust source modules, 152 symbols. No real workspace rebuild,
embedding-provider call, generated synonym list or new index field.

## Measured alternatives

The baseline run [35741072409](https://github.com/bpstr/codanna/actions/runs/35741072409)
at `c2181a2a63656d125592a6fb976856ec98721b12` ran the real index with the old
runtime while measuring alternative orders over its existing 200-result pool.
The original current small-limit output remained 6/20. These experiments do
not manufacture absent candidates or rewrite source documentation.

| Alternative | Operational Hit@5 | Paraphrase Hit@5 | Combined Hit@5 | Combined MRR@5 |
| --- | ---: | ---: | ---: | ---: |
| Current substring coverage | 5/10 | 1/10 | 6/20 | 0.210 |
| Raw lexical score only | 4/10 | 1/10 | 5/20 | 0.110 |
| Two results per file before deferral | 4/10 | 1/10 | 5/20 | 0.200 |
| Literal token coverage without paths | 4/10 | 1/10 | 5/20 | 0.185 |
| Literal tokens including paths | 5/10 | 1/10 | 6/20 | 0.210 |
| English stems without paths | 6/10 | 1/10 | 7/20 | 0.250 |
| English stems including paths | 7/10 | 1/10 | 8/20 | 0.267 |

The file-diversity control loses an existing success; it is not selected.
Keeping path/module evidence is useful on this corpus. No test/archive
exclusion or symbol-kind preference was introduced.

## Selected query-time change

Only conservative multiword discovery uses the new coverage key. It matches
whole normalized words, splits camelCase/snake_case identifier parts, and
applies Tantivy's existing English stemmer to both the query and stored
candidate evidence. Repeated terms/inflections count once. `redraw` is not a
match for the word `raw`. The score is lexical evidence, not confidence.

Single identifiers, short queries, explicit field/phrase/Boolean expressions,
exclusions and copied code syntax bypass the discovery reranker. The existing
raw Tantivy scores, kind/module/language filtering, one-key-per-candidate sort,
128 candidate minimum and 200 expansion ceiling remain unchanged. Larger
explicit caller limits are not reduced. No tokenizer is changed in the stored
index and no re-embedding or schema migration is required.

The existing displayed distinct-query-term coverage now describes normalized
English word stems, not literal substring counts. Model/backend policy and
semantic search behavior are unchanged. English stemming does not supply
synonyms or solve multilingual semantic intent.

## Executed candidate

[Run 35742404348](https://github.com/bpstr/codanna/actions/runs/35742404348)
started at `d929cbd865b99a9b05b07769060fa4b5e87613b8`, validated the original
query/module blob hashes, and tested the exact runtime blobs committed in this
batch. **45 selected tests passed**: three policy controls, two frozen task
measurement/identifier tests, 28 existing document-ranking tests, four symbol/MCP
contracts, two previous runtime corpus tests and six adversarial search/graph
cases. Strict all-target/all-feature Clippy passed.

Actual new runtime: **7/10 operational, 1/10 paraphrases, 8/20 combined**;
MRR@5 0.433 operational, 0.100 paraphrases, 0.267 combined. Every earlier
successful owner remains in the first five. The recovered operational owners
are `collect_all_files` (previously rank 7) and `scope_for_root` (previously 8).
Four other misses remain retrievable at 200; eight remain absent from that
bounded output. The easier 21-query synthetic corpus remains 21/21.

Oracle SHA-256 remains
`129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`.
The final retained workflow is read-only; the temporary blob handoff is removed.
Final-head reruns are distinct from this staged execution record.

## Limits and next diagnosis

This does **not** meet the original 0.90 Hit@5 / 0.75 MRR@5 quality target.
These questions have already influenced selection and are not a new holdout.
A regression guard preserves the eight known successes without relabeling the
remaining misses as acceptable quality. The one-pass timing totals are not a
controlled median/p95 comparison or a production latency guarantee.

Recorded Calls neighborhoods show that some useful lexical hits directly call
missing implementations, including `conversation_context` -> `capture` and
the foreign-reply test -> `render`. Those are possible related-code evidence,
not proof that blindly boosting all callees improves first-five relevance.
No graph expansion is enabled by this change.

The four undocumented owners (`capture`, `render`, cache `load` and `save`)
are ineligible under the old comment-only embedding policy. #55's body policy
must pass source-capture/persistence checks before any paid relevance test.
#56's rank fusion cannot recover nonexistent candidates merely by recombining
scores; document anchors or genuinely available semantic evidence are needed.
Independent holdout labels, graded relevance and controlled latency/memory
qualification remain open.
