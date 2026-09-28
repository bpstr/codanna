# Semantic coverage and freshness status

This T08 slice makes semantic-index coverage observable without creating vectors,
loading a model, contacting a provider, or rebuilding an index.

## Eligibility

Code semantic indexing currently selects a symbol when the parser produced a
`doc_comment`. The status names this source policy
`doc_comment_present_v1`. It is deliberately different from the embedding
input-budget identity such as `complete-input-v2:...`.

The report separates:

- total current code symbols;
- symbols eligible under the source-input policy;
- actual semantic vector count;
- eligible symbols that currently have a vector;
- eligible symbols without a vector;
- vectors whose symbol ID is absent from the current code index.

Detailed overlap is available only when the semantic index is loaded. A
metadata-only reader can report the persisted vector count but does not invent
per-symbol overlap from a count.

## Provenance and unknowns are explicit

Version-5 semantic persistence records a source SHA-256 and durable Tantivy
commit opstamp for each newly published vector. Therefore:

- `code_generation` reports the current durable Tantivy commit opstamp;
- `vector_code_generation` reports a generation only when every vector has
  complete provenance at one generation;
- `generation_alignment` is `matched`, `stale`, or `unknown_untracked`;
- aggregate `freshness` is `generation_aligned`, `stale_generation`, or `unknown`.

Legacy and partially migrated indexes remain unknown. Timestamp proximity and
numeric symbol-ID membership are not used as evidence of alignment.

Likewise, the current persisted state cannot distinguish an eligible missing
vector as permanently skipped versus pending work. `skipped_symbols` and
`pending_symbols` remain null rather than splitting the aggregate
`eligible_without_vector` count by guesswork.

## Identity

When semantic metadata contains the backend identity, the status exposes:

- model/backend/dimension;
- SHA-256 of the complete embedding identity;
- the recorded embedding input-policy string.

The raw endpoint URL or credential is never part of the persisted identity and
is not returned by this status.

## MCP index info

`get_index_info` retains its text output and adds a structured payload:

```json
{
  "index": {
    "symbols": 3,
    "files": 1,
    "relationships": 0,
    "code_generation": 7
  },
  "semantic": {
    "state": "live",
    "eligible_symbols": 2,
    "vector_count": 2,
    "eligible_with_vector": 1,
    "eligible_without_vector": 1,
    "vector_without_current_symbol": 1,
    "vector_code_generation": 7,
    "generation_alignment": "matched",
    "freshness": "generation_aligned"
  }
}
```

The exact code generation is runtime-specific. Per-definition `find_symbol`
diagnostics additionally report the reader-local reload generation and compare
the indexed hash, a bounded current-source observation and the vector-producing
hash. Only matching hashes plus matching commit opstamps report
`freshness: verified`.

## Remaining T08 work

Code and semantic directories are published separately; provenance detects and
reports their alignment without claiming a cross-directory atomic snapshot.
Persisting skip/pending reason counts, real-content model-quality evaluation and
model replacement remain separately authorized work.
