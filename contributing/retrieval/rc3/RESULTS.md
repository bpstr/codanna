# RC3 quality results — 2026-09-26

RC3 fixes the missing attachment-guide result without weakening the relevance
cutoff or changing index formats. With the existing body-aware code setting,
all 31 semantic acceptance cases pass. The full Assign reindex remains on hold:
the real-source probe and multi-source document query still expose quality gaps.
No Assign source or index was changed, and no paid inference was used.

Runtime candidate: `codanna 1.0.0-rc3 (25769a5)`, SHA-256
`96987648bd595d2ddbb2133df680b7413b2781d64d391ed85dafdbf50c3df49d`.
Baseline: checkout-built `1.0.0-rc2 (63b47fe)`, preserved before rebuilding.
The globally installed binaries and active repository MCP index were not replaced.
The committed [machine-readable evidence](results.json) contains binary/model
hashes, frozen corpus/query identities, per-query rows, failures, and command
records. Complete raw captures remain at
`/Users/bpstr/codanna-rc3-quality-20260926`; original RC2 evidence remains at
`/Users/bpstr/codanna-rc2-quality-20260926`.

## Acceptance comparison

All runs use the original fixture sources, case manifest, judgments, top-five
budget, and thresholds. The default and body profiles are reported separately.

| Runtime / code policy | Passed | Hit@5 | MRR@5 | Required-evidence recall@5 |
| --- | ---: | ---: | ---: | ---: |
| RC2 lexical | 29/29 | 1.000 | 1.000 | 1.000 |
| RC3 lexical | 29/29 | 1.000 | 1.000 | 1.000 |
| RC2 semantic, `doc_comment` | 29/31 | 0.857 | 0.714 | 0.786 |
| RC3 semantic, `doc_comment` | 30/31 | 0.857 | 0.714 | 0.857 |
| RC2 semantic, `symbol_body_v1` | 30/31 | 1.000 | 0.857 | 0.929 |
| RC3 semantic, `symbol_body_v1` | **31/31** | **1.000** | **0.857** | **1.000** |

Every invariant passes and forbidden evidence is zero in every row. Only the
RC3 body profile satisfies every semantic case and numeric minimum. The overall
qualification command intentionally exits nonzero because the unchanged default
profile still misses S02; this is not hidden by the passing body profile.

D06's attachment guide had cosine 0.42395, within the existing eligibility floor,
but a second chunk from ADR-004 occupied its slot. RC3 selects one eligible
passage per source before repeated sections, recovering the guide. Single-source
queries and sparse results can still fill their budget. Lexical ranking retains
its RC2 behavior: a review regression demonstrated why weaker one-term documents
must not displace a second high-coverage lexical passage.

S02's `erase_blob` has no documentation comment. It is ineligible under the
unchanged default policy. The existing opt-in `symbol_body_v1` setting recovers
it on both RC2 and RC3; this is a configuration/coverage improvement, not a newly
invented RC3 embedding algorithm. Changing representations requires a rebuild
with the corresponding cache identity. The acceptance model is cached MiniLM.

## Document model comparison

The same 14 English documents, 42 chunks, 10 queries, model-input budget, and
provisional source judgments were evaluated in all four cells. English has four
queries; Spanish and Hungarian each have three. Query/document input text was
not rewritten. Each capture has zero scoring/capture errors.

| Model | Runtime | nDCG@5 | Hit@5 | MRR@5 | Relevant-source recall@5 | Duplicate slots |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| MiniLM | RC2 | 0.546 | 0.700 | 0.633 | 0.483 | 24% |
| MiniLM | RC3 | 0.546 | 0.700 | 0.633 | 0.483 | 24% |
| E5 Small | RC2 | 0.756 | 1.000 | 0.850 | 0.817 | 0% |
| E5 Small | RC3 | 0.756 | 1.000 | 0.850 | 0.817 | 0% |

The improvement on this corpus is attributable to the model choice, not the RC3
ranking patch. The unchanged MiniLM results also show why diversity alone cannot
repair weak candidates outside the relevance margin.

| Language | MiniLM Hit@5 / MRR@5 | E5 Hit@5 / MRR@5 |
| --- | ---: | ---: |
| English | 1.000 / 1.000 | 1.000 / 0.875 |
| Spanish | 0.667 / 0.667 | 1.000 / 1.000 |
| Hungarian | 0.333 / 0.111 | 1.000 / 0.667 |

E5 is the stronger multilingual candidate here, but not a universal winner:
English nDCG falls from 0.848 to 0.758, English recall from 0.833 to 0.792, and
both models retrieve only one of q03's two grade-3 required sources. E5 also
returns more grade-zero sources on average (2.3 versus 1.6); removing duplicate
slots does not guarantee every replacement is useful.

MiniLM snapshot: `5f1b8cd78bc4fb444dd171e59b18f3a3af89a079`.
E5 snapshot: `614241f622f53c4eeff9890bdc4f31cfecc418b3`. Five public model assets
were downloaded once, pinned, hashed, and then copied into offline captures.
Downloads were bounded to 650 MiB/10 minutes and involved no inference service.
The actual download is recorded in `results.json`; generated vectors were local.

These measurements use Codanna's current input policy, which does not distinguish
E5 query and passage roles. The [model author's instructions](https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/README.md)
require role prefixes and warn that omission reduces performance. A future repair
must version the input/cache identity, budget the complete prefixed input, and
rebuild affected vectors; silently adding prefixes to queries or reusing old
passage vectors would invalidate the comparison. No prefix repair or default
model switch is claimed in RC3.

## Real repository probe

Eight exact source files were copied into a disposable workspace, including
chunking, document persistence, embedding budgets/cache, and code representation.
Their hashes and six exact symbol/path judgments were frozen in the probe before
retrieval. The corpus contains 461 symbols, 545 relationships, and 273 eligible
body-embedding owners. Existing inline Rust tests remain part of those source
files, so this is not a production-code-only collection or an independent holdout.

With MiniLM, exact-symbol Hit@5 is **5/6 (0.833)** and MRR@5 is **0.444**.
The missed symbol is `CodeEmbeddingPolicy::eligible`: results reach the correct
file and enum but not the exact implementation. The scorer does not count those
as a pass and rejects responses exceeding the five-result budget.

E5 is worse on the identical six code queries: **4/6 (0.667)** exact-symbol
Hit@5 and **0.347** MRR@5. It also misses `diversify`. Its six result lists remain
stable after rebuilding, and all three freshness checks pass. Keep MiniLM as the
measured code candidate; E5's document-language advantage does not justify a
global model replacement. Both code configurations fail this probe's all-six-hit
target. All 273 eligible owners have vectors; this does not establish full-body
coverage or correct semantic ranking.

All six result lists are identical after a force rebuild. CLI create, update,
and delete symbol-freshness assertions (dumped names) pass, the source copy returns to its original
hashes, and one actual stdio MCP semantic query matches the CLI's five paths and
names in order. These observations do not qualify watcher freshness, every MCP
tool, workspace routing, or generation consistency under concurrent mutations.
They also do not establish semantic freshness after a body-only edit. Timings are
retained as diagnostics; concurrent Rust builds/tests make them unsuitable for
production throughput, warm-latency, or memory-cost estimates.

The source-only preflight is read-only but reports `partial`: local tokenizer
identity and body-segment counts are deliberately unknown without loading the
model. The first probe stopped on that exit code; its failed evidence is retained.
The corrected probe permits only this specific fully parsed, local-model-unknown
case, preserves the partial report, and performs the explicitly bounded local
measurement. Blocked inventories and other partial cases still stop the probe.

## Verification and remaining work

The macOS evaluator regression moved from four failures/two errors to **38/38**
passing contracts with the default temporary directory. Workspace-parent aliases
are accepted; inner source symlinks, root reentry, and `..` remain rejected.
The acceptance harness has **27/27** passing contracts. The initial diversity
regression failed on RC2 and passes on the candidate. Independent source review
found three actionable issues, all corrected and re-reviewed with none remaining.

Both required repository gates pass: `quick-check.sh` and `full-test.sh`.
The full run reports **2,564 passed, zero failed, 63 ignored** Rust tests across
59 result groups, plus successful CLI smoke commands, the all-feature
documentation build, and scratch stdio MCP check. Ignored tests are not claimed
as verified. Strict Clippy covers all targets/features; executed tests use the
script's default-feature configuration. The installed compiler is Rust 1.97.1.
See [validation records](validation.json) for log hashes and exact scope.

The [rollout checklist](README.md#assign-rollout-gate) keeps the full Assign
reindex blocked. Prioritize the exact-symbol miss, q03's missing facet, native
language/answer-span review, and model-specific input handling. Then measure
source-aware candidate selection above 10,000 chunks, real working-set costs,
and long-lived transport/freshness behavior on a representative Assign subset.
Do not use the synthetic perfect acceptance score as evidence that these remaining
real-workload requirements are satisfied.

## Proposed next improvements

| Priority | Improvement hypothesis | Evidence and acceptance condition |
| --- | --- | --- |
| 1 | Add explicit model query/passage roles with versioned input identity. | E5 currently receives unprefixed inputs. Compare correct prefixes on a newly frozen corpus; enforce complete-input budgets and reject old incompatible vectors. |
| 2 | Qualify independent code/document model configuration and caches. | E5 improves Spanish/Hungarian documents but reduces exact code hits from 5/6 to 4/6. A single global model switch is not supported by this evidence. |
| 3 | Evaluate bounded lexical/semantic candidate fusion and implementation-level evidence. | Both models miss `eligible`; both miss one required q03 facet. Preserve these failures and add independently reviewed tasks before choosing weights or thresholds. |
| 4 | Measure source-aware candidate selection and lifecycle behavior at scale. | The current `4 * limit` candidate window can fill with chunks from one source before diversification. Test more than 10,000 chunks, relevant-source recall, relevance floors, RSS/disk limits, and concurrent generation changes. |

These are proposed follow-ups, not features claimed to ship in RC3. The small
dataset already rejects a blanket model replacement and an immediate full Assign
reindex; that is useful release-gate evidence, not a reason to relax the targets.

## Disk hygiene and retained candidate

After checking for open files, removed 18,172,290,073 logical bytes of inactive
session-created incremental caches, debug build outputs, and duplicate model
copies. The two cleanup phases observed 15,315,697,664 bytes of free-space gain;
APFS sharing and concurrent system activity make this distinct from logical file
sizes. Free space after cleanup was 31,483,625,472 bytes (**29.3 GiB**).

[Cleanup records](cleanup.json) list the 165 removed directories and bind the
complete 9,621-file manifest at
`/Users/bpstr/codanna-rc3-quality-20260926/cleanup-debug-files.json` by SHA-256.
Raw failures/results, fixture indexes, rescorable graded model snapshots, and
the canonical E5 download remain. The three exact tested RC3 release binaries
are retained under `/Users/bpstr/codanna-rc3-quality-20260926/bin/` and their hashes
are in `results.json`.

Free space remains below the 50 GiB hygiene threshold. The largest remaining
build candidates are `target/debug` (18 GiB) and `target/release` (4.7 GiB).
Their pre-existing artifacts were preserved; expanding cleanup to them requires
separate approval. No source, Git history, active index, or release evidence was
deleted.
