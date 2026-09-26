# Compatible embedding reuse during code-index rebuilds

Implementation: [PR #47](https://github.com/bpstr/codanna/pull/47), following
[the calendar investigation](https://github.com/bpstr/codanna/pull/43).
This change reduces avoidable embedding work. It is not an embedding-quality
improvement, a new model/input policy, or a claim that every rebuild is free.

## Why this matters for the graph fixes

Parsing symbols, extracting JSX, resolving imports, and rebuilding relationship
edges are local operations. Remote embedding requests are a separate step.
The current code embedding collector uses documentation-comment input, so a
parser/graph repair or body-only change need not change that embedding input.
Document-chunk embeddings have their own lifecycle; this repair targets code.

The force-rebuild lane clears the lexical index but leaves the semantic cache
file. Previously, `enable_semantic_search` created a fresh semantic instance
without restoring that cache. A second same-identity validation also cleared
cached inputs while the active vector generation was empty. Together, these
made retained, compatible input vectors needlessly miss during a rebuild.

The repaired lane binds the real backend identity, restores only the optional
content-addressed cache, and keeps it when the same identity is revalidated.
A cache hit attaches its vector to the newly collected symbol ID and language.
It does not restore an old symbol mapping, an old active vector generation,
or deleted symbols. Existing source parsing and relationship repair still run.

## Compatibility and cost boundaries

Reuse requires exact input bytes plus the cache's complete embedding identity,
dimensions, and preprocessing version. Backend endpoint, model, pinned revision,
and input-budget policy changes must not silently reuse incompatible vectors.
Equal model names or dimensions alone are insufficient. The configured identity
must change when a mutable provider model changes; this repair cannot detect an
unannounced provider-side change behind the same identity.

The cache remains an optional bounded accelerator: at most 4,096 entries,
a 16 MiB in-memory vector budget, and a 32 MiB serialized-file read bound.
Evicted, missing, oversized, corrupt, or incompatible entries are misses.
Larger corpora can have much lower reuse than the small regression corpus,
including eviction churn in a sequential rebuild. This change does not enlarge
those budgets or guarantee full-corpus coverage.

The remote backend still performs an initialization embedding probe. Therefore
zero repeated source inputs is **not** a zero-token promise. New/changed inputs
and cache misses still reach the configured backend. Local embeddings do not
create a remote provider bill; they still use local compute. No pricing or real
workspace token saving is inferred from the synthetic tests below.

Preserve the existing semantic cache when adopting this change. Deleting the
whole `.codanna` directory or semantic directory discards that opportunity.
Do not confuse the tested `codanna index ... --force` lane with deleting and
recreating every store. A missing cache is safe, but requires embedding misses.

## Executed before/after evidence

[Run 35668960463](https://github.com/bpstr/codanna/actions/runs/35668960463)
records baseline revision `cad8c4770975e5f89a5f1daabf62d65cdaa2bbb2` and runtime
candidate `9907866128ac06fac1bc1b9e4ec29eb691026984`.
The same formatted CLI test file was used on both sides, SHA-256
`b694dcbd59974891f0f3069026ee2abdd750dcc7b21704f66279042468545cdc`.

The five CLI regression groups moved from **3 passed / 2 failed** to
**5 passed / 0 failed**. The two baseline failures directly observed unnecessary
repeated source inputs, rather than only asserting that a new metadata field
was missing. Five new cache-lifecycle unit tests, sixteen existing document
embedding regressions, and six index-lifecycle regressions also passed at the
candidate. These are GitHub CI executions on Rust 1.98.1 / Linux x86-64, not
claimed local Rust runs or a completed repository-wide release gate.

| Synthetic scenario | Before: source inputs | After: source inputs | Initialization probes |
| --- | ---: | ---: | ---: |
| Initial index of two documented symbols | 2 | 2 | 1 each run |
| Force rebuild with unchanged comment inputs, changed body and shifted IDs | 2 | 0 | 1 each run |
| Force rebuild after changing one comment | 2 | 1 | 1 each run |
| Delete one symbol and rebuild the remaining cached source | Not reached after the earlier baseline assertion | 0 | 1 candidate probe |
| Missing/corrupt cache or changed compatibility identity | 2 | 2 | 1 each run |

The CLI is spawned as separate real processes with an empty environment and
disposable HOME. Only an explicitly configured loopback HTTP fixture receives
synthetic text; no provider credentials or private source are used. The tests
inspect persisted vectors after each rebuild, checking current IDs, dimensions,
alpha/beta vector identity, and removal of deleted symbols. Probes and source
inputs are counted separately; these counts are not tokenizer or monetary data.
The temporary branch-only patch runner is removed after committing the source
repair. The retained regression workflow is read-only.

## Rebuild decision

Do not repeatedly rebuild a real workspace after every small parser commit.
Integrate the approved graph fixes and this cache-lifecycle fix, verify their
combined runtime, and then perform one controlled upgrade/reindex. Retain the
old store or a recoverable backup, record actual backend/model/input policy,
and inspect current cache eligibility before making a cost estimate. A paid
real-workspace run still requires its own approved scope, cap, and stop condition.

Remaining opportunities: a read-only rebuild preflight reporting eligible inputs,
cache hits/misses and bounded payload estimates; eviction/admission experiments
on corpora larger than cache capacity; combined-release verification with the
JSX fixes; and the separate frozen ranking/semantic-quality work from PR #43.
None is implied complete by the passing reuse fixtures.
