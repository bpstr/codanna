# Semantic cache admission during rebuilds

This follow-up to PR #47 addresses a bounded-cache ordering failure discovered by
the rebuild-cost pressure experiments in PR #49.

## Reproduced baseline

The end-to-end fixture indexes 4,500 synthetic documented Rust symbols through the
real `codanna index --force` CLI and a loopback embedding endpoint. The first run
fills the content-addressed cache, whose two-dimensional fixture capacity is 4,096
entries. The next force rebuild processes the same source order.

Baseline commit `ac4a2e15f2a4bfcdbde6cb428e9a5464bf151759` sent **4,500 source
inputs again**, although 4,096 compatible entries existed before the run. Five
pre-existing rebuild-reuse cases passed and the new pressure case failed with
`left: 4500, right: 404`. No provider credential or production index was used.

The cause is ordering, not identity validation: the semantic embed stage previously
looked up one memory-sized inference chunk and immediately admitted its misses.
Early misses therefore evicted compatible entries needed by later chunks in the
same 5,000-symbol collector batch.

## Runtime change

The embed stage now probes/reuses cached vectors for the **entire collector batch
before admitting any generated miss**. Only the remaining misses are then embedded
in memory-bounded inference chunks. This preserves the existing memory-pressure
guard and cache capacity while preventing new misses from destroying still-needed
hits in the same batch.

The regression performs a third rebuild too. New misses must still be admitted, so
the cache continues learning; capacity means 404 source inputs remain misses in this
4,500/4,096 fixture rather than freezing an old favorable snapshot.

This is intentionally not a cache-size increase and not a free-rebuild promise.
Separate collector batches can still rotate a bounded cache, and realistic vector
dimensions can reduce capacity below 4,096. PR #49's offline preflight remains the
tool for estimating current input/cache pressure before a real rebuild.

## Verification

Candidate runtime commit: `b4de1cb7c42ad3a275d7d16c77327b372993f7f9`.
Third-rebuild assertion commit: `0a6dd5b3742b64a955ae8efd0ab03cd7d01f5850`.
Repository formatting was subsequently applied mechanically.

Focused candidate and wider gate results are recorded in PR #50 only after GitHub
CI completes. The baseline failure above is observed evidence; this document does
not infer a passing candidate before execution.
