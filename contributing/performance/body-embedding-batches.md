# Bounded cross-symbol body embedding (proposed; qualification pending)

This change retains the current code representation policy, source byte ranges, model,
tokenizer budget, cache identity and storage format. It supports both existing body policies;
it does not reduce v2's head/middle/tail coverage to obtain a faster benchmark.

## Execution

Before admitting any new vector, prepare every body candidate once using the existing
`SymbolSource::inputs` implementation. Pin matching resident cache vectors for the complete
collector batch, as before. Retain only source-fragment indexes, byte ranges, input lengths
and hashes; release the temporary strings. Immutable source references already belong to
the collector batch, so this does not copy or reread the source corpus.

Materialize exact input bytes in windows bounded to 64 selected segments and 8 MiB of
prepared text. The byte limit admits the existing worst-case single symbol (8 validated
inputs of at most 1 MiB each), so there is no unbounded oversized-item exception and no
coverage-reducing rejection for valid inputs. This bounds prepared text, not total process
RSS, original collector queues, cache pins, descriptors, or native tensor allocations.

Probe pinned/live caches and deduplicate exact missing text across the window. Feed useful
cross-symbol batches to the existing adaptive backend and scatter responses through
request-local ordinals. Preserve each parent's original language and range mapping even
when a vector is shared. Validate response cardinality, ordinals, dimensions and finite
values before installing results. Publish only complete parent representations after the
window's inference succeeds. Do not hold the semantic lock across inference.

Partitioning and source-policy selection are performed once, but exact strings are
reassembled later from the descriptors. Backend input validation and inference tokenization
are deliberately retained; this is not an unsafe bypass of the complete-input contract.
A reusable memory sampler replaces per-candidate sampler allocation, with preparation
checks every 32 symbols and fresh checks before every inference batch.

## Qualification

Added unit fixtures cover v2 Unicode/source-range round trips, header-only inputs,
cross-symbol deduplication, out-of-order responses, malformed/incomplete results, cache-only
windows and count/byte bounds. Run:

```sh
cargo test --lib indexing::pipeline::stages::semantic_embed
```

Also run existing body/cache-admission/source-provenance and retrieval fixtures, followed by
the repository quick/full gates. Neither those tests nor Mac benchmarks were executed in
the editing environment; fixture addition is not a passing qualification.

Compare frozen v1/v2 body-policy inputs and stored range mappings before/after, including
collector batches larger than cache capacity, warm/cold caches, duplicates crossing windows,
provider failures, low-memory batches and incremental body-only edits. Measure inference
request sizes, tokenizer preparation time, CPU time, peak RSS and elapsed time separately.
Validate numerical/retrieval tolerances when batching changes, especially for dynamically
quantized models whose outputs can depend on the batch. No measured speedup is claimed.

The existing RC4 interrupted-code-index recovery limitation is unchanged. Perform all
qualification in isolated staging, not by interrupting and resuming the only working index.
