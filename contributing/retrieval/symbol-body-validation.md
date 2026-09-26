# Symbol body representation rollout

The runtime and synthetic regression changes are committed on PR #55. The source policy remains opt-in:

```toml
[semantic_search]
enabled = true
code_representation = "symbol_body_v1"
```

Omitting `code_representation` retains `doc_comment`. The selected policy is part of the full backend identity; semantic loading rejects a mismatched source policy before query-backend initialization. A query never rebuilds or migrates an index. Only an explicitly approved force rebuild should adopt the new policy.

## Implemented mechanics

The native and generic parse paths capture eligible function/method/type source from the parse snapshot. Legacy comments remain unchanged. Bounded representation fields identify the current repository workspace, relative path, available module/owner/name, kind, signature and documentation. Literal routes, events and configuration keys remain in retained implementation text, not generated summaries.

Retained source is capped at 16 KiB per symbol and 1 MiB per file including representation overhead. The collector also flushes bounded source batches. Backend input validation splits retained source against the actual complete-input budget; at most eight head/tail segments are selected. Every segment retains its real parent symbol and available byte range. Excluded middle sections and incomplete parser ranges are not represented as complete source coverage.

Each selected segment has an independent vector. Search takes the maximum segment similarity per parent before top-k selection; a long symbol cannot occupy multiple result slots. Incremental/watch indexing uses the same processing stage. Replacing or deleting a parent also replaces/removes its segment set.

Format 4 checkpoints store segment groups under the atomic semantic journal, with checksums, bounded artifacts and validated dimensions/counts. Snapshot readers retain the old generation while writers update the next one. The legacy source policy continues to write format 3. Vector count and represented-parent count are distinct.

## Verification status

The implementation was applied from a digest-checked source patch and formatted in the repository runner. That establishes delivery, not test success. The read-only `Symbol representation contracts` workflow must execute the new contracts and strict lints before acceptance. Its artifacts record the exact checked source SHA.

Tests cover representation determinism, undocumented implementations, Unicode ranges, body/path/signature edits, retained-tail retrieval, parent deduplication, language filtering, snapshot replacement, checkpoint/delta reopen, tombstones, invalid vectors, corrupt/missing checkpoints, source-policy mismatch and eligibility reporting. The parse/collector fixture checks that a filesystem edit after parsing cannot change the input snapshot.

No production Assign workspace has been reindexed or queried. No provider credentials or paid inference are used by these fixed-vector contracts. The current legacy-source planner from #49 is not a body-policy cost estimate. A combined build with the cache, ranking and graph PRs still requires separate verification and a capped, explicitly approved dogfood evaluation.
