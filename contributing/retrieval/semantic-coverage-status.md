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

## Unknowns are explicit

The current semantic persistence format does not record the code/Tantivy
generation that produced a vector generation. Therefore:

- `code_generation` reports the current Tantivy reader generation;
- `vector_code_generation` is null;
- `generation_alignment` is `unknown_untracked`;
- `freshness` is `unknown`.

Timestamp proximity is not used as evidence of alignment.

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
    "vector_code_generation": null,
    "generation_alignment": "unknown_untracked",
    "freshness": "unknown"
  }
}
```

The exact code generation is runtime-specific. Vector presence is evidence that
a stored vector exists for an ID, not that the vector is relevant, fresh, or
generated from the current source bytes.

## Remaining T08 work

A future persistence change may atomically bind semantic and code generations
and may persist skip/pending reason counts. Until then those fields must remain
unknown. Real-content model-quality evaluation, expanding code embedding inputs
beyond documentation comments, and model replacement remain separately
authorized experiments.
