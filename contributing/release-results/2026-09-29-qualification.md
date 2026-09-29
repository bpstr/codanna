# September 29 RC4 qualification: no-go

**Decision: qualification failed. Do not promote RC4 or activate the staged Assign index.** No installed executable was replaced during this qualification. The current installation remains `codanna 1.0.0-rc4 (940abaa2)`. Assign's active settings and index were preserved.

This extends the [RC4 receipt](2026-09-29-rc4.md) and follows the [release manual](../release-testing.md). Runtime source is unchanged from `940abaa2b028da69748cc03405d595d9b4a1c716`; subsequent repository changes are documentation and a standalone qualification witness. Existing exact-runtime CI/build evidence remains applicable. No new full Cargo build or suite is claimed here.

Private raw evidence, source snapshots, independently frozen judgments, commands, resource samples, failed attempts and retained indexes are under:

`/Users/bpstr/Github/codanna-release-evidence/2026-09-29-qualification`

## Release-blocking code recovery failure

The deterministic witness creates 205 Rust files with 100 functions each and 99 calls per file. It kills the indexer only after observing a changed Tantivy publication while the process is still running, then resumes ordinary incremental indexing and compares a forced rebuild.

Two initial runs reproduced the same result:

| State | Files | Symbols | Relationships |
| --- | ---: | ---: | ---: |
| Incremental restart after SIGKILL | 205 | 20,500 | 7,425 |
| Forced rebuild | 205 | 20,500 | 20,295 |

This is not just a counter discrepancy. In the confirmation run, six of nine sampled functions returned an empty callee list after incremental restart, while the forced rebuild returned the expected exact callees. The resumed index is retained separately before rebuilding. The witness does not instrument the precise internal crash phase and does not establish coverage of every nonretryable failure or captured incoming edge. It does establish that successful incremental restart can leave missing call relationships.

The reusable [code-recovery witness](../retrieval/qualify-code-recovery.py) reproduced the same counts and six missing sampled edges in a third run. It records hashes, commands, publication evidence, both indexes' counts and individual call edges. A failing exit status is a failed release gate, not a successful qualification because the harness ran. Repair durable recovery and verify outgoing and captured incoming edges before promotion. Do not attempt to recover the only working Assign index with a plain incremental retry after interruption.

## Retrieval quality

Fixed acceptance results reproduce RC4's previous measurements: lexical **29/29**, comment semantics **30/31**, body semantics **31/31**. The graded document set remains Hit@5 **0.700**, MRR@5 **0.633**, nDCG@5 **0.546**, with **24% duplicate slots**. The missing undocumented implementation and multilingual document weaknesses remain.

An independent reviewer froze 11 Assign implementation owners, paired English/Hungarian questions, distractors, exact source ranges and 14 whole-file hashes before observing retrieval. Eight owners have no declaration doc comment. The oracle and copied private source remain local. The original acceptance targets are Hit@5 ≥0.90 and MRR@5 ≥0.75; no threshold was lowered after measurement.

| Code representation | English Hit@5 / MRR@5 | Hungarian Hit@5 / MRR@5 | Combined Hit@5 / MRR@5 |
| --- | --- | --- | --- |
| Comments | 3/11 / 0.273 | 1/11 / 0.018 | 4/22 / 0.145 |
| Body | 11/11 / 0.955 | 4/11 / 0.318 | 15/22 / 0.636 |

Both combined profiles fail. These requests use **unfiltered top-five nearest-neighbor retrieval**: settings contain threshold 0.6, but this route does not apply that configured floor when the request omits an explicit threshold. They are not measurements of a 0.6 result floor. Independent adjudication confirmed that the permitted nested-owner alternative does not change any miss.

Body-aware results matched all 22 queries over actual stdio and again after restarting the server. Subsequent-query median latency was approximately **8 ms** in both sessions; first-query latency was 150 ms and 101 ms, reported separately. A disposable watcher probe passed create, body-only semantic update, removal of the old embedding and deletion. This is one local lifecycle witness, not full failure-injection coverage.

The alias/configuration witness is recorded separately in `oracle-adjudication.json` and its A12 evidence. A unique matching name alone cannot prove governing-project configuration: TypeScript project resolution is opt-in, and the initial subset omitted explicit config files. Missing package-extended configuration is also a fixture limitation, not automatically a product defect. A self-contained configured fixture correctly separated competing web/mobile aliases, but changing only the web alias target followed by ordinary incremental indexing retained the old call edge. A forced rebuild followed the new target while preserving the mobile edge. This is a failed incremental configuration-change gate; fresh-build resolver collision behavior passed. The fixture does not qualify real Expo package-extends resolution.

## Performance and capacity

Three alternating baseline/RC4 trials used identical frozen corpora, cached MiniLM and separate indexes. Graph totals matched at 383 files / 11,548 symbols / 18,357 relationships; the semantic corpus matched at eight files / 505 symbols / 607 relationships. Cold indexing changed **−0.2% graph-only / +6.9% semantic**. Force rebuild changed **+0.6% / +2.8%**. Create, body-edit and deletion measurements stayed inside the 15% time tripwire, and no measured memory/index-size increase exceeded 20%.

Initial no-op medians failed at +80.0% graph-only and +57.3% semantic. Investigation found that the harness's `subprocess.run(..., timeout=...)` with file outputs adds polling granularity to subsecond wall times; desktop variance was also substantial. Seven additional alternating trials used pipe-driven completion while retaining a timeout and the same inputs. Median graph time was **21.01 → 21.72 ms (+3.4%)**; semantic time was **182.88 → 182.72 ms (−0.1%)**. This closes the observed no-op regression tripwire for these corpora, not a universal latency guarantee. Both original failures and corrected measurements remain in evidence.

A deterministic document corpus recovered from SIGKILL after observed vector staging growth to **11,501 chunks / 102 files**, with **11,501 physical and live vectors**, zero unembedded chunks and no vector-storage growth on a no-op. Literal queries during resumed indexing took **20–41 ms** including CLI startup. Returned source byte ranges, not echoed query text, prove the anchor and recovery hits. The descriptive duplicate/dominance query returned five distinct sources and no slots from the deliberately longer document; this synthetic observation does not establish real-corpus relevance. Peak sampled process-tree RSS during recovery was approximately **217 MiB**.

The isolated full-size Assign **code** stage completed in **63.18 seconds**, with **2,811 files, 128,934 symbols, 59,075 relationships and 2,421 embeddings**. Peak sampled process-tree RSS was approximately **471 MiB**. Source discovery listed 4,807 files; indexed-code counts are not a claim that every discovered generic/document file became code. The earlier source-only generic-grammar omissions are not relabeled as a complete inventory.

The real Assign document stage discovered **36,560 chunks**. It was deliberately stopped at the last recorded **8,402 embeddings**, approximately 347 seconds into the invocation, after the independent code-recovery failure was confirmed. It did not finish and did not exceed its 15-minute trial limit. Its observed throughput projected a longer cold run; no completed cold-capacity or real-document recovery pass is claimed. Small correctness witnesses ran during this stage, so its throughput is a desktop/load observation. Peak sampled process-tree RSS was approximately **943 MiB**.

## Rollback, isolation and retained failures

An APFS-cloned snapshot of the active index was verified against every original file hash and reopened using its matching `1928f68` executable. Index information, exact-owner lookup, document statistics and lookup in a second process passed. Original index hashes/mtimes remained unchanged. This is a paired snapshot reopening rehearsal, not an actual production cutover-and-rollback exercise.

The unqualified stage was moved out of the prepared index path and retained at:

`/Users/bpstr/Github/assign/.codex/tmp/codanna-rc4-stage/unqualified-index-2026-09-29`

Any future staging attempt must start fresh. The original active index and current installed binaries remain unchanged. No MCP consumer was restarted and no paid API, proxy, provider credential or `.secrets` was used.

Harness failures were preserved and corrected narrowly: prepublication progress must be observed in vector staging rather than final collection state; query echoes cannot prove retrieval; no-op generation IDs and per-run counters are not stable storage invariants; deletion polling must allow a transient stale row before requiring its disappearance. A later passing correction does not erase an earlier failure.

The evidence and retained diagnostic indexes occupy 2.31 GiB. No release evidence or databases were deleted. Final disk inspection is recorded in `disk-final.json`; the data volume retained **55.28 GiB free**. Removed paths: none; reclaimed bytes: 0. No Rust build/cache cleanup was necessary in this qualification-only run.
