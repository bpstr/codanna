# Embedding input limits and revisions

Codanna validates the complete input before inference. Document inputs include
heading breadcrumbs, separators and the exact source chunk; query inputs use the
complete query. The remote client sends accepted text unchanged. Its former
2,000-character truncation has been removed.

Document indexing refines oversized character chunks into smaller source slices
using the effective backend budget. Each input retains the complete heading
breadcrumbs and its exact body bytes. The refinements preserve the original
chunks' source coverage and overlap without adding overlap of their own. Lexical
indexing keeps the existing character chunking behavior.

If the heading context leaves no room for a complete body character, or bounded
tokenizer search cannot find a complete split, indexing returns an actionable
error and preserves the prior indexed generation. Shorten long headings or
configure a larger limit supported by the actual provider before retrying.
Code symbols and queries retain complete-input rejection for oversized inputs.
See [document token splitting](retrieval/embedding-followups/document-token-splitting.md)
for the search bounds and deterministic regression fixtures.

## Configure a remote model

```toml
[semantic_search]
enabled = true
remote_url = "http://127.0.0.1:8100"
remote_model = "my-embedding-model"
remote_dim = 768
model_revision = "deployment-2026-09-20"
max_input_tokens = 8192
tokenizer_path = "/absolute/path/to/tokenizer.json"
```

`max_input_tokens` must match the remote provider's supported context size. The
default is 8192; it is a configured input budget, not a claim that every endpoint
supports that context size. `tokenizer_path` is optional and points to the exact
model's Hugging Face tokenizer JSON already present on disk. Codanna reads this
file locally and does not download a tokenizer. Use an absolute path for stable
behavior across working directories.

With a tokenizer, Codanna counts the full token sequence after normalization and
special-token insertion. Counting disables tokenizer truncation and padding, so a
short reported length cannot hide an omitted tail. The supplied tokenizer must
match the provider, including its normalizer and special-token processing.

Without a tokenizer, each UTF-8 byte consumes one unit of the configured budget.
Diagnostics and the saved identity call this a `utf8-byte-budget-proxy`. This is
a conservative proxy for common tokenizers, not an exact count or universal upper
bound: normalization and provider-specific special tokens can expand input.
Configure the actual tokenizer when strict provider token accounting is needed.
The fallback accepts normal document chunks without the previous character cut
and leaves the complete accepted text intact, including multilingual text.

Remote URL, model, dimension and API key retain their existing
`CODANNA_EMBED_URL`, `CODANNA_EMBED_MODEL`, `CODANNA_EMBED_DIM` and
`CODANNA_EMBED_API_KEY` precedence. Revision, tokenizer path and input budget are
settings fields. Keep credentials in the API-key environment variable.

## Local models

Local FastEmbed generators and the shared model pool use the tokenizer already
loaded with the inference model. The effective input ceiling is that tokenizer's
configured maximum. `max_input_tokens` can reduce this ceiling; it cannot raise
it beyond the actual loaded model limit. `tokenizer_path` is remote-only.

All inputs also have a 1 MiB byte ceiling before tokenization. This bounds
tokenizer work for pathological single tokens independently of their token count.
Document refinement also divides source chunks above this ceiling; each resulting
complete input must satisfy both limits.

## Model changes and persisted vectors

`model_revision` is an explicit weight revision or deployment fingerprint. Change
it whenever the same provider alias begins serving different weights. Codanna
cannot discover an unannounced weight change behind an unchanged alias.

Both code and document vectors record the effective backend, model, an endpoint
digest, explicit model revision, tokenizer digest, budget and preprocessing
version (`complete-input-v2`). The endpoint URL and API key are not stored in this
identity. A change of identity requires reindexing even when vector dimensions
are identical. Legacy vectors without the complete identity remain readable for
metadata and lexical use, but cannot be combined with a current query embedding.

Code reindexing uses `codanna index <path> --force`. Document indexing must start
from a fresh document embedding index when the model or preprocessing identity
changes; the existing mismatch diagnostic identifies the incompatible index.
Preserve source documents and their collection configuration while rebuilding.
Document chunking fingerprints additionally include the splitting policy and
generator identity, so compatible policy changes reprocess unchanged sources once.
The embedding text format and content-addressed cache remain compatible when the
complete input is unchanged. Changing the configured budget or tokenizer still
changes the persisted embedding identity and requires a fresh document index.

The embedding cache is an optional accelerator scoped to the same identity and
preprocessing version. Cache files above 32 MiB are rejected before JSON decoding;
the reader also bounds bytes if a file grows during reading. Decoding and in-memory
reuse have separate entry, vector dimension and memory limits. Invalid caches
are treated as empty and cannot prevent a rebuild.
