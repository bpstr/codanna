# Assign indexing performance review — September 30

## Observations and limits

The Assign assurance reports `2026-09-30-codanna-index-health.md` (duration
section) and `2026-09-28-codanna-retrieval-performance.md` (V2 follow-up) record
36m for the forced code rebuild and 29m 43s for documents: 65m 43s combined.
Refreshes raised cumulative command time to 73m 22s. These are observations on
an active workstation, not a controlled before/after benchmark.

Body-v2 retained 21,853 eligible symbols and 31,491 vectors after refresh.
Documents held about 37,768 live vectors. Live code/document/search directories
totaled about 416 MiB; a separate, pre-existing 682 MiB failed document store is
not new live data or a memory measurement. CPU samples used roughly six cores;
memory percentages are point samples, not peak RSS. No paid inference was used.
Keep the current `symbol_body_v2` policy, exact input bytes, eligibility, source
ranges, model identity and persisted schema. No rebuild or activation is part of
this work.

## Existing fork proposals

Reviewed against fork main `c5f0a93ad50babab5cae5788370baee56ec8a21d`:

| PR / inspected head | Purpose | Qualification boundary |
| --- | --- | --- |
| [#87](https://github.com/bpstr/codanna/pull/87), `690fed37b7e5cf14d28f2352dcd625fa6192ef1a` | Shared native ORT CPU pool and spinning controls | Opt-in resource control, not a whole-process quota; actual thread/energy effects remain unmeasured. |
| [#88](https://github.com/bpstr/codanna/pull/88), `1745582c671e6407e74ffc314a55306c0bbbcba2` | Cross-symbol batches and prepared body-v2 ranges | Most direct candidate for code inference throughput; bounded prepared strings do not bound all collector memory. |
| [#89](https://github.com/bpstr/codanna/pull/89), `39f9905a53876cd573b45f4c3979de5e12cd81ec` | Indexed Rayon scheduling and throttled usage logs | Small overhead reduction; no measured full-run speedup. |

All three are pending drafts, not shipped fixes. The review included complete
diffs, comments and recent fork merges #79–#86. No proposal changes v2 schema.
The current dirty RC4 remediation tree was inspected for overlap and left intact;
its pipeline recovery/project-binding changes do not implement document cache
pinning. GitHub parent/source remain `bartolli/codanna`, with `main` defaults.
Upstream main resolved to `58295d290f70c2ca46a5ace48fe2aab8687d7d6e`;
this was a fork performance review, not a renewed full upstream issue audit.

## Additional improvement: preserve document cache hits

A document refresh can exceed the 4,096-entry embedding cache. Early misses
admit new vectors and evict old entries before later chunks use them. Protect
the cache present at the start of each embedding run, while continuing to look
up new within-run results in the live cache. Apply this to both the collection
spool and single-file batch path.

The snapshot shares vector `Arc`s. It retains at most one starting cache plus
the existing live cache, bounded by the existing entry/vector limits; hash and
queue metadata are cloned. It is not a corpus-sized cache, a change to persisted
cache format, or a guarantee that all 37,768 document vectors can be reused.
The benefit is avoided inference for starting-cache hits that would otherwise
be evicted. Actual Assign time savings remain unmeasured.

## Next improvements, in priority order

1. Add separate counts for cache hits, unique inference inputs, batch requests,
   preparation/inference/publication durations and process peak RSS. The current
   “embedded chunks” progress includes reuse and does not explain the report's
   1,245-chunk six-second refresh. Never infer backend throughput from it alone.
2. Extend #88's prepared transport integration coverage across window boundaries,
   cache eviction and partial backend failures before integration. Validate exact
   inputs and ranges; prepared vectors do not establish model quality.
3. Diagnose code generation alignment independently: equal source hashes with a
   `stale_vector` label need a publication/freshness witness, not another expensive
   rebuild or an unconditional freshness override.
4. Consider larger document reuse only after measuring misses and memory. Reuse
   from published generations needs exact input/model identity and publication
   safety; do not increase the cache unboundedly or change v2 source coverage.
5. Treat natural-language ranking misses as separate retrieval work. A positive
   body-only retrieval and two remaining query misses do not justify changing
   the model, schema or eligibility policy in this performance change.

## Validation

Prepared document regression exercises both embedding entry paths with a full
starting cache, earlier admissions, a late cached vector and a newly generated
duplicate. It verifies inference input counts and exact stored vectors. No model
download, paid endpoint, Assign index mutation or fixture recording is needed.
The fixture failed before the fix (65 inferred inputs instead of 64). After the
fix, `cargo test --lib documents::` passed 69 tests with no failures or ignored
tests, including generation recovery and compaction fixtures. Tests used a clean
environment without provider configuration. Other gate results are recorded in
the PR; unexecuted native/model benchmarks remain unqualified.
