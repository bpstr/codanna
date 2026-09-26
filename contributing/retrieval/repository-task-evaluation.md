# Repository-task relevance and coverage computation

T05/T06 follow-up to #52 for #43. The first source-grounded task measurement
exposes a gap hidden by the earlier deliberately vocabulary-matched fixtures.
The optimization in this batch preserves ranking; it does not fix that gap.

## Frozen corpus and judgments

Ten implementation-owner tasks each have an operational question and a
paraphrase, for **20 queries**. Five complete production Rust modules from
`ddb5ae61a72938d82cceaf42123dc7a88bfe3417` are indexed without rewriting comments
or function names. They produce **152 symbols**, including their original test
helpers and documented/undocumented functions. The owner judgments were
hand-curated after source inspection, not independently annotated by users.
This is neither a production Assign benchmark nor a held-out tuning set.

The oracle is [repository-tasks.json](evaluations/repository-tasks.json).
SHA-256 checks freeze each source file. Byte-identical copies live under
`tests/fixtures/repository_task_sources/*.rs.fixture`; the suffix prevents normal
repository indexing from discovering these duplicate Rust implementations.
Only the benchmark materializes them as `.rs` files in its temporary corpus.
Labels and query text stay outside that directory.

## Observed results

[Run 35734901934](https://github.com/bpstr/codanna/actions/runs/35734901934)
started at `51a6230ec63be90f3686317f49f2d3e61e40ba55`. It ran before and after the
reviewed coverage-computation change on exactly the same source/query bytes.

| Query family | Owners found in top five | MRR@5 |
| --- | ---: | ---: |
| Operational wording | **5 / 10** | **0.320** |
| Paraphrases | **1 / 10** | **0.100** |
| Combined | **6 / 20** | **0.210** |

Six additional owners appeared with a 200-result budget but not in the first
five. Eight were not retrieved even with that budget. The expanded output is
still coverage-ranked, not a raw Tantivy candidate-rank trace; absence there
does not establish that no matching candidate could exist beyond the cap.
Every expected owner was independently confirmed present by exact path/name.
Exact-identifier rank-one controls and the nonexistent-token control passed.

**A passing measurement test is not a passing relevance threshold.** These
results do not satisfy the original 0.90 Hit@5 / 0.75 MRR@5 proposal. The older
21/21 synthetic result remains true for its own easy corpus but does not show
that ticket-to-implementation retrieval is solved.

### Per-task observations

Ranks below are top-five ranks; `-` means not found in that output. Numbers in
parentheses are expanded coverage ranks with a 200-result budget.

| Task owner | Operational | Paraphrase |
| --- | --- | --- |
| `read_file` | 2 | - (22) |
| `is_modified` | - | - |
| `pair_relocations` | 1 | - (21) |
| `collect_all_files` | - (7) | - |
| `scope_for_root` | - (8) | 1 |
| `capture` | - | - |
| `render` | - (7) | - |
| cache `load` | 5 | - |
| cache `save` | 2 | - |
| `embedding_batch_size` | 1 | - (23) |

## Verified query-time optimization

The previous comparison function rebuilt/lowercased evidence and counted term
coverage repeatedly during sorting. The new helper computes one scalar coverage
key per candidate, moves result rows without cloning their strings, and keeps
the same raw-score/location/name/ID tie-break order.

The deterministic 128-candidate control reduced key evaluations from **1,762
to 128**. Full serialized result order stayed equal to the previous comparator;
NaN/infinity/signed-zero raw-score ordering and key-extraction failure behavior
are tested too. This is a work-count result, not a measured production speedup.
All **20 real-source query rankings remained identical before/after**. No
provider, schema change, embedding policy or automatic index rebuild is involved.

## Execution and source provenance

The staged run passed **47 selected tests**: 2 repository-task measurement and
identifier tests, 3 new optimization tests, 2 scoring diagnostic tests, 28
existing document-ranking tests, 4 symbol/MCP contracts, 2 earlier runtime
corpus/identifier tests, and 6 adversarial search/graph tests. Formatting and
strict all-target/all-feature Clippy passed.

Oracle SHA-256: `129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`.

| Candidate runtime file | SHA-256 |
| --- | --- |
| `src/storage/tantivy/query.rs` | `b1436905cabf74de3895d37257e4a3e0059684270d2532f26fd370b75aa3c8ac` |
| `src/storage/tantivy/mod.rs` | `c4cce7085fc75bf699cd42f14e44c1458960be2113936f3f22de8d55db257694` |
| `src/storage/tantivy/ranking_efficiency.rs` | `8268e9d9c80eaa483f1c3cb562d78c43ddbb8e0b05398f522efed428dc0e9296` |

The verified runtime blobs are committed through the connected GitHub app.
The final benchmark changes only include paths to byte-identical frozen copies;
its source hashes/queries/judgments are unchanged. Final read-only CI reruns that
committed state. The temporary blob-publication workflow is removed.

## What to investigate next

Do not respond to the miss rate by blindly increasing the overfetch cap or
re-embedding everything. Six misses already have retrievable owner candidates,
while other tasks lack usable query/implementation vocabulary in indexed fields.
Separate bounded query-clause/owner-diversity ablations from semantic-input
coverage. #55 already addresses implementation representations; #56 already
addresses bounded ticket fusion. They need their own integration and relevance
evidence before any real paid evaluation.

An independent, larger, multi-language annotation set, graded labels/nDCG,
repeatable cold/warm median/p95/RSS evidence and the combined-release gates
remain open. The new results are an honest diagnostic baseline, not a reason to
claim those gates completed.
