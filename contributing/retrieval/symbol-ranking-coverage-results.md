# Symbol ranking coverage verification

Date: 2026-09-22. Runtime implementation follows the T05/T06 evidence in PR #51.
PR #52 remains a draft; no production index or upstream repository was modified.

## Selected policy and earlier baseline

Simple multi-concept discovery uses 16x requested candidates, a 128-candidate
floor for small requests, and a 200-candidate expansion ceiling. A requested limit
above 200 is not reduced. Filters stay in the candidate query. Scalar scores remain
raw lexical evidence, not confidence or the final coverage-based sort key.

The seven-query baseline was 4/7 Hit@5. Relevant owners existed at lexical ranks
50-53; 4x and 8x overfetch still missed them, while 16x recovered 7/7. In the
21-query corpus, R0 achieved 11/21 Hit@5 and MRR@5 0.524; the 80-candidate
experiment achieved 20/21 and 0.952. The remaining owner was candidate 98,
motivating the 128 floor rather than another scoring weight.

The selected runtime produced 21/21 Hit@5, MRR@5 1.000, including the seven
reserved queries. These are synthetic lexical fixtures with deliberately
high query/document vocabulary overlap, not a production Assign benchmark or
proof of generalization to paraphrases, undocumented code, or mixed workspaces.

## Clause-level evidence

`src/storage/tantivy/ranking_diagnostics.rs` creates 52 stored symbols, including
48 same-named generic `settings` symbols and `useAccountPresentation`. This is a
storage/scoring fixture, not an end-to-end parser fixture. It evaluates
`calendar settings` on the same index with each clause family separately.
The 200-row drain is complete because the entire fixture has only 52 documents.

| Clause ablation | Matching candidates | Owner rank | Owner score | Generic settings score |
| --- | ---: | ---: | ---: | ---: |
| All analyzed fields | 49 | 49 | 2.074614 | 2.169549 |
| Analyzed name only | 48 | absent | absent | 2.008271 |
| Documentation only | 49 | 1 | 2.074614 | 0.069879 |
| Signature only | 48 | absent | absent | 0.091399 |
| Whole-query fuzzy n-gram | 0 | absent | absent | absent |
| Whole-query fuzzy name | 0 | absent | absent | absent |

The complete R0 query, including the mandatory symbol clause, scored the generic
candidate 2.179028 and the owner 2.084093. The owner ranked **49 before coverage
reranking, 1 afterward**. Here the generic name contribution dominates despite
much stronger owner documentation. The whole-query fuzzy clauses contribute no
candidates for this multiword query. This does not imply that fuzzy matching can
be removed: the separate `ArchivService` typo control retrieves `ArchiveService`.

Twelve query-family controls exercise exact identifiers, typo, snake_case,
camelCase, topic, phrase, Boolean, exclusion, explicit field, short token, code
fragment and absent-query paths. Every returned runtime raw score is checked
against an independently assembled R0 query, so clause drift cannot silently
invalidate the explanation experiment. These controls are intentionally small;
they are not a complete query-language compatibility suite.

## Score-explanation limitation, not a successful full explanation

Run [35714264735](https://github.com/bpstr/codanna/actions/runs/35714264735),
head `3b897dd66ededa8ce65fea8fb640596d1a9a71cb`, reproduced a Tantivy 0.26.1
phrase-scorer assertion (`target >= self.doc()`) in the diagnostic target.
The ordinary query-family score checks passed in that same execution.

The instrumented follow-up isolates this to explaining the owner's combined
query. The generic candidate has a complete explanation tree; the owner's full
tree is recorded as `unavailable_dependency_panic`, not an empty/zero score.
A separate matching documentation-term explanation is available and numerically
checked. Field-ablation scores and ordinary candidate search remain available.

Only this test-only optional explanation probe contains the specifically observed
panic. A different panic or an unexpected returned error still fails. There is no
production panic wrapper, swallowed search error, dependency upgrade, or change to
the ranked runtime algorithm. The pinned diagnostic limitation remains open; a
passing evidence-collection test does not claim the dependency bug is repaired.

## Executed follow-up

Run [35716142547](https://github.com/bpstr/codanna/actions/runs/35716142547),
head `3412e21b07d607c1fe9f9690655a4a42bbf19cf2`, merge checkout
`dda00e337bf0e01237a3a0dba29356997bb5e7de`, executed **42 selected tests**:

| Group | Passed | Failed |
| --- | ---: | ---: |
| Clause/explanation evidence and query-family raw-score parity | 2 | 0 |
| Existing document retrieval-ranking contracts | 28 | 0 |
| Crowded symbol discovery and direct MCP contracts | 4 | 0 |
| Broader runtime corpus and cross-language identifier controls | 2 | 0 |
| Existing adversarial search/graph contracts | 6 | 0 |

The runtime corpus again reported 21/21 Hit@5, MRR@5 1.000; reserved subset
7/7, MRR@5 1.000. The known explanation panic is explicitly recorded as unavailable
within those diagnostic results; it is not counted as a repaired full explanation.

| Input | SHA-256 |
| --- | --- |
| `src/storage/tantivy/query.rs` | `fb4d95482c33fd9fa433de6bd85eeb6f973451bf4411174267c95d9a3febb1e2` |
| `src/storage/tantivy/ranking_diagnostics.rs` | `fe4051249691abed63c6e82f0f6738d79a35f67eaa321ad55a64202dc77d2b20` |
| `tests/symbol_ranking_regressions.rs` | `623ce11440fade657114b268e8b8118b19f293c008c2b7ad70fbbb303b7c8e10` |
| `tests/symbol_ranking_runtime_holdout.rs` | `e1c621789604fc6cf699cb86cbc81e1bf8e8df85b7ae3320c87f87ebf1fc1f3b` |

Rust 1.98.1, x86_64-unknown-linux-gnu. The run's `symbol-ranking-coverage` artifact
contains machine-readable clause rows, full available trees and all test output.
No executed-binary digest is claimed. Earlier native ONNX linker-cache failures
stopped compilation and are not counted as ranking failures or reproductions.

## Wider gates and remaining work

The pinned follow-up above passed behavioral tests but its formatting gate failed.
Subsequent formatter commits are preserved. Current-head formatting, strict Clippy,
full-suite and combined-release status must be checked separately; the table is
not an overall green-workflow claim.

No new ranking rule was selected from these diagnostics. Remaining work includes
independent realistic relevance labels/paraphrases, source/owner diversity,
large-index allocation and latency costs, clause explanations for other miss
families, and the scoped-search integration in #53. Reader-local generation IDs
and synthetic perfect Hit@5 are not substitutes for a real-index validation run.
The query-time ranking change itself does not need re-embedding.
