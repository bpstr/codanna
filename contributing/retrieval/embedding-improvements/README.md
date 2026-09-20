# Embedding and lexical retrieval improvements

This follow-up implements the next steps from the
[document embedding review](../adversarial/document-embedding-review.md) and the
[lexical acceptance findings](../adversarial/LEXICAL-RESULTS.md). Its baseline is
`dd8e247c62dcfd61dd1cdb7feaf23fe3777a469d`, the head of PR #37. The original reports
remain historical evidence; this directory records the new comparison.

The changes make document publication recoverable after process termination,
bound active vector storage across edits, validate complete embedding inputs,
isolate incompatible model revisions, and reconcile document event bursts by
collection. Lexical retrieval gains Markdown reference evidence, safe local
filename decoding, linked configuration evidence, better prose-to-symbol
matching, and source/section diversity.

The final frozen build passes **29/29 lexical acceptance cases**, up from 23/29,
with required-evidence recall@5 rising from **0.875 to 1.0** on unchanged inputs.
All **68 new fixtures and 7 revised tests pass**, including **19 subprocess
termination scenarios**. The complete local default/all-feature suites pass
**2,338 / 2,340 tests**, with 62 preexisting ignored tests and one unchanged Unix
socket fixture excluded because this environment denies socket creation. That
fixture remains enabled in CI. [validation.json](validation.json) records exact
commands, counts, source identity, and gate results.

## Review and reproduce

- [Complete fixture checklist](FIXTURES.md) and [machine-readable inventory](fixtures.json)
- [Complete verification results](validation.json) and [evaluation provenance](evaluation-context.json)
- [Lexical acceptance comparison](LEXICAL-RESULTS.md)
- [Embedding storage, recovery and diagnostics](EMBEDDING-STORAGE.md)
- [Input budgets, tokenizer configuration and revision compatibility](../../embedding-inputs.md)
- [Watcher batching and retry behavior](../adversarial/DOCUMENT-WATCHER-BATCHING.md)
- [Measured repeated-edit storage comparison](STORAGE-MEASUREMENTS.md), [probe](document_churn.rs), [baseline](churn-baseline.json) and [updated measurements](churn-final.json)

Run the normal repository gates from the repository root:

```bash
./contributing/scripts/quick-check.sh
./contributing/scripts/full-test.sh
cargo test --locked --all-features --no-fail-fast
```

The focused integration targets use fixed vectors or loopback mock transports:

```bash
cargo test --locked --test document_embedding_regressions --test document_backend_regressions
cargo test --locked --test document_diagnostics_regressions
cargo test --locked --test knowledge_source_evidence_regressions --test retrieval_ranking_regressions
cargo test --locked --lib documents::
cargo test --locked --lib watcher::unified::
```

Run these commands without provider credentials or inherited `CODANNA_EMBED_*`
overrides. Do not add `--ignored`: ignored model/provider tests are outside this
deterministic evaluation.

## Inspect a document index

```bash
codanna documents stats my-collection --json
```

The existing collection statistics stay at the top level. The additive
`embedding_index` object describes the shared document index across **all**
collections: generation, backend/input identity, live and physical vectors,
segment count, chunks lacking embeddings, and vector payload bytes written or
copied during the last publication. Reading statistics does not load an embedding
model. Counts describe the active generation; older queries can keep previous
mapped files alive until those queries finish.

## Scope of the evidence

The unchanged acceptance corpus measures lexical/structural behavior. Fixed
vectors test semantic selection and model compatibility, but do not measure a
real model's relevance. Process-termination fixtures test publication ordering;
power-loss durability also depends on the filesystem honoring atomic replacement
and flush operations. The storage probe reports actual vector records and file
bytes. Its wall-clock samples are descriptive measurements of one local run.

Further work includes a labeled real-model relevance evaluation before choosing
rank fusion or neighbor expansion, cache eviction-policy measurement, and live
reload of document collection definitions. Collection-definition changes still
require restarting the server. Oversized embedding inputs fail clearly; automatic
token-budget chunk splitting is a separate improvement.
