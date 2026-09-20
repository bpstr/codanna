# Document embedding review

The reviewed baseline is `1968a6fc14c4d5c080ac91b5e74799a4b0aa0cc2`.
Sixteen deterministic integration scenarios were executed against that clean
source: 15 failed and one unchanged/deletion control passed. The final integrated
repair run passed all 16. No real model inference or provider credentials were
used. The normal regression target is `tests/document_embedding_regressions.rs`.
Additional configured-backend, watcher and candidate-count tests also passed;
their individual outcomes are in [ADDITIONAL-CHECKLIST.md](ADDITIONAL-CHECKLIST.md)
and their final suite results are in [validation.json](validation.json).
The separate [lexical acceptance run](LEXICAL-RESULTS.md) measured document
Hit@5 and MRR@5 of 1.0, with required-evidence recall of 0.875 against a 0.90 floor.
Missing source diversity remains an observed retrieval-quality gap.

## How the document pipeline works

Collection discovery walks configured paths, applies collection globs and
`.codannaignore`, and deliberately does not apply Git's tracking exclusions.
Individual configured files are explicit includes. Paths are canonicalized and
deduplicated. One source belongs to one collection; an overlap is rejected with
the existing owner's name instead of silently omitting the second collection.

Each scan hashes the actual UTF-8 content. A saved chunking fingerprint includes
the merged collection configuration and chunker algorithm version. Changed bytes,
changed chunking policy, a forced collection, or missing embeddings trigger
replacement. An mtime match cannot suppress a content check. Chunking retains
exact source byte spans and derives heading ancestry from real headings in source
order, excluding fenced code. Embedding input adds heading breadcrumbs to the
chunk body; stored source evidence remains an exact source slice.

CLI indexing, CLI queries and MCP loading use one configured backend factory.
Disabled semantic search produces lexical BM25 results from content and headings
and loads no embedding model. Enabled search uses the configured local or remote
model, actual vector dimension and backend identity. Remote requests use the
shared configuration and environment precedence. Persisted identity includes a
digest of the endpoint, effective model name, and document-input version; it never
stores the URL or API key. A different identity or dimension is rejected before
mixing old corpus vectors with a new query model. Legacy vectors without recorded
identity require a fresh document index, while their metadata remains usable for
lexical search. An unchanged alias can still hide changed provider weights; use a
pinned model revision and consider an explicit revision/fingerprint config field.

For an index mutation, Tantivy updates stay uncommitted until all embedding batches
succeed. Vector appends are staged in a sibling file, so a failed batch leaves the
previous vector mapping intact. Handled errors roll back metadata and source
tracking, preserve monotonic IDs and permit retry in the same process or after
reopening. Successful vector publication, Tantivy commit and atomic JSON state
replacement happen in that order. These are individually published artifacts,
not one crash-atomic generation; the process-kill followup below remains open.

Document query surfaces read the indexed snapshot. JSON one-shot search no longer
walks and embeds collections as a side effect. The persistent server watcher uses
the same collection discovery and transaction, including collection overrides,
new/recreated files, moved subtrees and `.codannaignore` edits. A relevant event
currently scans the affected collection; this is a correctness-first cost to
measure before introducing finer incremental discovery. Live configuration edits
to document collection definitions require a server restart.

## Reproduced scenarios and repair expectations

| Scenario | Baseline observation | Required behavior and implemented repair |
| --- | --- | --- |
| Run force twice on the same source | Duplicate live chunks | Replace by source without forgetting ownership. |
| Force collection A with B present | B loses file tracking | Mark selected collections dirty; retain every collection's state. |
| Include one path in A and B | Successful B can contain no chunks | Reject the overlap explicitly. |
| Include a directory and its child file | Same source is discovered twice | Canonicalize, sort and deduplicate discovered files. |
| Change bytes with the same mtime | Old text remains searchable | Compare content hashes, including preserved timestamps. |
| Reduce chunk size on unchanged bytes | Old chunk boundaries remain | Fingerprint merged settings and algorithm version. |
| Enable embeddings after lexical indexing | Existing metadata suppresses vector creation | Track embedding completeness per source and backfill. |
| Switch between equal-dimension models | Old corpus and new query vectors are mixed | Validate persisted backend/model/input identity or fail clearly. |
| Fail an embedding batch, then retry in memory | Retry skips incomplete work | Keep metadata and file state transactional until embeddings succeed. |
| Fail an embedding batch, reopen and retry | Duplicate metadata or reused IDs | Roll back metadata and persist monotonic retry state. |
| Search for absent terms without a model | Arbitrary chunks with zero score | Use literal analyzed terms with BM25 and collection/source filters. |
| First-line heading followed by a subheading | Wrong/missing heading ancestry | Process headings in source order. |
| Hash heading inside a code fence | Code text becomes document structure | Recognize backtick/tilde fence lengths and ignore fenced headings. |
| Merge whitespace-separated paragraphs | Stored text differs from reported source span | Keep borrowed source slices through merging and splitting. |
| Highlight text with Unicode case expansion | Invalid UTF-8 slice panics | Map normalized match positions back to original character spans. |
| Unchanged source and then deletion | Existing positive control | Reuse embeddings and hide deleted chunks from retrieval. |

`tests/document_backend_regressions.rs` adds a disabled-backend control, a bounded
local HTTP embedding fixture that verifies configured model/dimension and heading
input across indexing/query/reopen, and a CLI test that changes the source after
indexing and proves one-shot search does not rewrite corpus state. Watcher units
drive actions directly with local files, without sleeps or OS event timing:
`document_watcher_indexes_new_and_recreated_files_with_collection_overrides` and
`document_watcher_catches_moved_subtrees_and_nested_ignore_changes`.

The baseline already had complete semantic candidates via Tantivy's
`DocSetCollector`; the old 10,000-candidate limit was not a current defect. Its
existing `filtered_candidates_are_not_capped_at_ten_thousand` test retains 10,001
candidates. The new `lexical_search_finds_relevant_chunk_after_ten_thousand_nonmatches`
test puts the only relevant chunk after 10,000 nonmatches and requires retrieval.
Chunker and preview unit controls additionally cover CRLF, skipped heading levels,
closing hashes, Unicode expansion/shrinkage, adjacent matches and exact overlap.

## Remaining work, in priority order

1. **Crash recovery across artifacts.** Introduce an immutable generation manifest
   or recovery journal binding Tantivy commit, vector segment and source state.
   Inject termination immediately before/after vector publication, metadata commit
   and state replacement. Reopen each case and assert one live chunk generation,
   unique IDs, matching vectors and a safe retry. Current handled-error tests do
   not prove power-loss or process-kill recovery.
2. **Model-aware input budgets and explicit revisions.** The shared remote client
   currently truncates input after 2,000 characters. Large document chunks or long
   heading breadcrumbs can therefore omit the tail from the vector. Local model
   tokenizers also impose model-specific limits. Add an explicit preprocessing
   policy and token-aware budget that includes breadcrumbs, validate oversized
   input, and persist its version. A local mock should place the only relevant
   phrase beyond the limit and assert it is split/preserved or rejected clearly.
   An explicit model revision/fingerprint must invalidate equal-dimension vectors.
3. **Vector compaction and write cost.** Deleted/replaced vectors remain physical
   records and semantic scoring scans them. Staging currently copies the vector
   file for a changed transaction. Benchmark repeated edit/delete cycles, report
   physical/live vector ratio and bytes copied, then implement immutable segments
   and compaction that preserve active query snapshots. Assert stable live results
   and bounded storage across 100 deterministic update cycles.
4. **Retrieval quality and diagnostics.** Evaluate lexical/semantic rank fusion,
   section-aware diversity and neighbor expansion on a curated labeled fixture
   corpus. Include heading-only concepts, exact identifiers, repeated boilerplate,
   paraphrases, multiple languages and large filtered collections. Compare recall
   at k and reciprocal rank before changing defaults. Fixed test vectors establish
   correctness; they do not establish real-model relevance improvements. Surface
   incomplete-vector state and the active backend/input revision in diagnostics.
5. **Cache loading and watcher scale.** Bound cache bytes before JSON decoding,
   then measure a recency-aware eviction policy against current bounded entries.
   Batch document event bursts into one affected-collection scan and measure scan
   bytes/latency. Keep policy-file changes and source deletion in the same truth
   reconciliation path. Add config-reload tests before claiming live updates to
   document collection definitions.

## Local validation

Run from the repository with provider credentials and `CODANNA_EMBED_*` overrides
removed from the test process. All transports in the new tests bind loopback and
use fixed vectors. Do not opt into ignored model/provider tests.

```bash
cargo test --locked --test document_embedding_regressions
cargo test --locked --test document_backend_regressions
cargo test --locked --lib documents::
cargo test --locked --lib watcher::unified::document_collection_tests
```

The repository's normal formatting, clippy and full-suite gates remain required.
No generated indexes, model files, fixture credentials or runtime caches belong
in the commit.
