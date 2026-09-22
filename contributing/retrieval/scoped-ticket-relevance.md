# Combined ticket relevance and workspace scope

This integration retains the histories of #61 (`c22eb98`), #57 (`43c15e1`) and
#59 (`c6f0a6a`) on a separate branch. No main/upstream merge, provider inference,
production reindex or embedding-policy change is involved.

## Executed combined baseline

[Integration run 35765600618](https://github.com/bpstr/codanna/actions/runs/35765600618)
merged only those pinned heads locally, ran the actual combined source, and
passed **59 selected tests**, formatting and strict all-target/all-feature
Clippy. The tested source blobs were then committed through the GitHub app.
The temporary integration script/workflow is removed from the committed tree.

Same five frozen Rust source files, 152 symbols, 20 questions and ten path/name
owner judgments; oracle SHA-256 remains
`129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`.

| Evidence surface | Operational | Paraphrase | Combined |
| --- | ---: | ---: | ---: |
| Direct owner Hit@5 | 7/10 | 1/10 | **8/20** |
| Owner in up to five direct plus six related results | 9/10 | 1/10 | **10/20** |

These are measured together, not summed from different branches. Direct
MRR@5 is 0.433 operational and 0.100 paraphrase (0.267 combined). Opting in
leaves all 20 direct result arrays unchanged. One explicit reader-generation
invalidation was discarded and logged; the existing bounded measurement retry
succeeded. Missing owners and ordinary errors were not retried.

The related surface has a larger evidence budget and is not improved Hit@5.
Paraphrases remain weak, the original 90% target remains unmet, and these
source-inspected questions have influenced selection rather than forming an
independent holdout. Input availability is not semantic model quality.

## Next checkpoint

Explicit scoped graph expansion is still disabled at this combined-baseline
checkpoint. Its qualification must use #57's registered-file identity on the
same pinned graph reader, filter both seed and target, and retain all edge and
result budgets. New controls must reject external-root namesakes, prefix/file
neighbors and unregistered endpoints without global fallback. Scope filters
remain retrieval constraints, not authorization boundaries.
