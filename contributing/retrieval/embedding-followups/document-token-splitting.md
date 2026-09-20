# Document token-budget splitting

Final local verification passed **16 new fixtures and one revised fixture**, plus
nine retained backend controls. Complete default/all-feature suites passed
**2,354 / 2,356 tests** across 37 completed test-result groups per suite, with
62 preexisting ignored tests and one unchanged socket fixture excluded because
this environment denies AF_UNIX creation. The unmodified full script also
encountered one existing debounce assertion that passed unchanged in both complete
reruns. Formatting, strict Clippy, no-default-feature compilation, strict docs,
CLI/MCP smoke and 25 lexical evaluator contracts passed.

Each completed test-result group is one `test result:` summary. Separate doctest
groups count separately even when they share a Cargo target heading.

The unchanged lexical corpus remains **29/29**, with **18/18 invariants**, mean
required-evidence recall@5 **1.0**, and zero forbidden evidence. See the
[validation report](document-token-splitting-validation.json),
[lexical results](document-token-splitting-results/lexical-report.json), and
[complete test logs](document-token-splitting-results/). Real-model relevance is
unmeasured. These results do not qualify the separately documented manual cases.

Document indexing first applies `HybridChunker` with the validated character
settings. When embeddings are enabled, it refines each resulting `RawChunk` using
the generator's effective `InputBudget` before assigning IDs or staging metadata
and embedding inputs. `index_collection` and `reindex_file` share this path.

Every refinement is an exact, nonempty UTF-8 slice of the original body. The
original heading hierarchy is copied to each slice, and the same breadcrumb
prefix is used for budget accounting and inference. Slices partition each parent
chunk without additional overlap; source-byte coverage, including the original
character overlap, is unchanged. Existing paragraph trimming remains part of
`HybridChunker`. This change does not claim to add previously omitted whitespace.

The public `EmbeddingGenerator::document_input_ranges` hook defaults to the
original body range (or no ranges for an empty body). Configured local and remote
generators, and `FastEmbedGenerator`, supply their actual budget. Store code checks
that hook results form a complete ordered UTF-8 partition before using their
offsets. Third-party generators can opt into the hook without depending on a
private tokenizer type. Their `cache_identity` must cover any budget/preprocessing
settings that change the hook's output.

## Budget accounting and bounds

Without a tokenizer, the UTF-8 byte proxy directly fills the available capacity
after subtracting the full breadcrumb prefix. Boundaries move backward to complete
Unicode scalar values. Heading context that leaves no body capacity, or too little
capacity for the next complete scalar, is an explicit error.

Exact tokenizers count the composed input after normalization and special-token
insertion with truncation and padding disabled. A fitting input stays intact.
Otherwise the search tries smaller prefixes, followed by other prefix boundaries
when no initial candidate fits. Each selected candidate is checked in full. The
search never treats token counts as monotonic: a single character can expand into
several tokens while a longer prefix merges into fewer tokens. It also never
rejects exact-tokenizer breadcrumbs based only on their standalone token count.

The search is conservative and greedy. It does not promise the longest fitting
prefix or discover every possible partition. It can fail when an untried partition
would succeed; diagnostics describe a bounded search failure rather than proving
that segmentation is impossible. Safety bounds are:

- A composed probe is at most 1 MiB, including breadcrumbs.
- At most 128 candidate probes are considered at a body offset.
- Total bytes submitted to the counting tokenizer for a parent character chunk
  cannot exceed `max(16 KiB, 64 * (prefix bytes + body bytes))`.
- A parent character chunk emits at most 65,536 parts. Result ranges consume
  bounded space, and one reusable input buffer holds candidate text.
- Repeated breadcrumbs and body text emitted by byte-proxy splitting share the
  same linear byte allowance as exact tokenizer work. This bounds heading-copy
  amplification when a large heading leaves room for only a tiny body.
- The store enforces the per-parent output bounds for custom generator hooks too,
  then caps all refinements of one source file at 64 MiB of composed input and
  65,536 chunks before copying headings. This also bounds accumulated output from
  overlapping character chunks under the same large heading.

The byte proxy does not invoke a tokenizer. Exact budgets retain the normal
backend batch preflight before inference. Neither mode discards body evidence or
averages vectors. Tiny budgets and oversized headings fail explicitly. Code
symbols and queries continue to reject oversized complete inputs.
Local document batches retain nonempty whitespace-only partitions; code and query
handling keep their existing empty-text rules.

## Persistence and compatibility

The entire indexing transaction, including all refined chunks of a document,
rolls back on splitting or embedding errors. A late invalid heading cannot publish
earlier chunks. A later failed embedding batch cannot publish earlier vectors or
metadata. Tests also verify other collections and reopened readers.

Semantic document chunking fingerprints include `document-budget-splits-v1` and
the generator identity. Existing compatible source files are processed once after
this policy is introduced, then unchanged files skip normally. Lexical-only
fingerprints and chunking stay unchanged. The existing `document-input=2` text
format and content-addressed cache are compatible for identical complete inputs.
Budget, tokenizer, model, revision and endpoint changes retain the existing
fail-closed embedding-identity behavior; they require a fresh document index.

## Deterministic verification

All fixtures use local tokenizer definitions, deterministic vectors, temporary
files, or a loopback mock transport. They require no credentials, downloads,
external services, real models or paid inference.

```bash
cargo test --lib embedding_input::tests::document_
cargo test --lib documents::store::token_splitting_tests
cargo test --lib semantic::pool::tests::document_batch_
cargo test --test document_backend_regressions
git diff --check
```

The input-budget fixtures cover exact NFKC expansion, special tokens,
nonmonotonic BPE prefixes, prefix/body token merging, tiny budgets, byte capacity,
work/part limits and a long unbroken source above 1 MiB. Store fixtures cover
Unicode, combining marks, CRLF, code fences, tables, long lines, exact source-byte
and heading preservation, overlap multiplicity, collection/watcher parity, stable
skips, fingerprint migration, invalid custom-generator ranges and rollback after
late heading or embedding-batch failures. The configured backend regression
asserts that actual mock-transport document inputs obey the configured budget and
reconstruct the full original bodies after their breadcrumb prefixes are removed.
Memory-bound fixtures include a public hook that amplifies heading copies, a
large overlapping heading, and a file requiring too many tiny fragments. Local
batch selection is verified to retain whitespace-only document partitions while
preserving code input filtering.

## Fixture checklist

The [machine-readable fixture inventory](document-token-splitting-fixtures.json)
contains exactly **16 new tests and 1 revised test**. Each row counts one executable
Rust test function; parameterized subcases, repeated invocations, helpers, and
unchanged tests selected by broad targets do not increase those counts. Integration
test names below are root-level Cargo harness names scoped by
`--test document_backend_regressions`; all other names are scoped by `--lib`.

Every row **passed final-source focused verification**. The linked inventory records
the exact log and source identity for each fixture; the raw focused logs are in
[document-token-splitting-results](document-token-splitting-results/).

- [x] **DTS01 (new)** — [embedding_input::tests::document_byte_splits_fill_budget_and_preserve_utf8](../../../src/embedding_input.rs#L455). Split multilingual body bytes under a 12-byte budget with breadcrumbs; also try an empty body, undersized scalar budget, and oversized heading. **Oracle:** Exact ranges 0..6, 6..15, 15..22 cover the original UTF-8 body and validate; impossible byte capacities fail explicitly.

- [x] **DTS02 (new)** — [embedding_input::tests::document_splits_count_normalization_and_special_tokens_in_every_input](../../../src/embedding_input.rs#L475). Refine NFKC-expanding Arabic text with heading context and model special tokens, then try a one-token budget. **Oracle:** The original input exceeds the budget, every nonempty source slice validates, and the tiny budget returns a bounded-search diagnostic naming special tokens.

- [x] **DTS03 (new)** — [embedding_input::tests::document_splits_try_nonmonotonic_normalized_prefixes](../../../src/embedding_input.rs#L491). Use local NFKC plus BPE where the first scalar expands to three tokens but a longer prefix merges to one. **Oracle:** The singleton fails, the longer prefix passes, and exact ranges 0..4 and 4..5 cover the source without relying on monotonic counts.

- [x] **DTS04 (new)** — [embedding_input::tests::document_splits_validate_composed_heading_instead_of_prefix_alone](../../../src/embedding_input.rs#L526). Use local BPE merges across the heading separator into the body. **Oracle:** The standalone heading exceeds the budget while the composed heading/body passes and remains one exact body range.

- [x] **DTS05 (new)** — [embedding_input::tests::document_splitting_bounds_tokenizer_work_and_output_parts](../../../src/embedding_input.rs#L554). Exhaust exact-tokenizer work, request too many byte-proxy parts, and amplify a large heading beside tiny bodies. **Oracle:** Each case returns its specific tokenizer-work, part-count, or repeated-heading output allowance error.

- [x] **DTS06 (new)** — [embedding_input::tests::document_splitting_handles_long_unbroken_inputs_above_byte_ceiling](../../../src/embedding_input.rs#L574). Refine one unbroken token longer than the 1 MiB byte ceiling. **Oracle:** Two exact source ranges cover the entire body and each complete input passes the normal budget validator.

- [x] **DTS07 (new)** — [documents::store::token_splitting_tests::document_budget_splitting_preserves_source_bytes_headings_and_existing_overlap](../../../src/documents/token_splitting_tests.rs#L156). Index multilingual text, emoji, combining marks, CRLF, fenced code, tables, and a long line under a 64-byte budget. **Oracle:** Stored chunks equal their source slices; per-byte coverage matches original overlap multiplicity; headings match parent chunks and complete recorded inputs fit the budget.

- [x] **DTS08 (new)** — [documents::store::token_splitting_tests::document_budget_splitting_matches_collection_and_watcher_and_skips_unchanged_files](../../../src/documents/token_splitting_tests.rs#L203). Replace the same source through collection and watcher indexing, repeat unchanged indexing, and reopen both stores. **Oracle:** Both paths produce equal evidence; unchanged runs preserve IDs and avoid embedding calls; reopened chunks match the persisted results.

- [x] **DTS09 (new)** — [documents::store::token_splitting_tests::document_budget_late_heading_failure_preserves_both_collections_and_reopen](../../../src/documents/token_splitting_tests.rs#L324). Place an impossible heading after many otherwise valid replacement chunks; exercise collection and watcher updates. **Oracle:** No new embedding calls occur; original IDs, bodies, vector scores, file state, generation, and the other collection survive failure and reopen.

- [x] **DTS10 (new)** — [documents::store::token_splitting_tests::document_budget_later_embedding_failure_rolls_back_splits_and_can_retry](../../../src/documents/token_splitting_tests.rs#L353). Fail the second embedding batch of a large replacement through collection and watcher paths. **Oracle:** Both prior collections and reopen state remain exact after failure; retry restores full source coverage and leaves no unembedded chunks.

- [x] **DTS11 (new)** — [documents::store::token_splitting_tests::document_budget_invalid_generator_ranges_reject_without_publishing_partial_evidence](../../../src/documents/token_splitting_tests.rs#L402). Return invalid UTF-8 boundaries, gaps, overlap, or a missing tail from the public generator hook through both indexing paths. **Oracle:** Each range error occurs before embedding calls; prior evidence, IDs, vector scores, the other collection, and reopened state remain intact.

- [x] **DTS12 (new)** — [documents::store::token_splitting_tests::document_budget_policy_migration_reprocesses_unchanged_source_once](../../../src/documents/token_splitting_tests.rs#L439). Replace a stored semantic fingerprint with the previous character-only fingerprint without changing the source. **Oracle:** One run reprocesses with new IDs and compatible cache reuse; the new fingerprint persists and the reopened next run skips without new embeddings.

- [x] **DTS13 (new)** — [documents::store::token_splitting_tests::document_budget_bounds_custom_generator_heading_amplification_before_cloning](../../../src/documents/token_splitting_tests.rs#L499). Return many valid tiny ranges from a custom hook beneath a large heading. **Oracle:** The store rejects repeated-heading output amplification before embedding, while preserving prior evidence and the other collection.

- [x] **DTS14 (new)** — [documents::store::token_splitting_tests::document_budget_bounds_total_file_refinement_before_publication](../../../src/documents/token_splitting_tests.rs#L521). Refine a large overlapping heading and a separate source that needs too many tiny fragments. **Oracle:** The aggregate 64 MiB or 65,536-chunk guard fails before embedding; the prior generation remains exact both immediately and after reopening.

- [x] **DTS15 (new)** — [semantic::pool::tests::document_batch_retains_whitespace_partitions_without_changing_code_filtering](../../../src/semantic/pool.rs#L519). Partition an unheaded body into byte-budget slices that include whitespace-only fragments, then apply local batch selection. **Oracle:** Document selection retains every nonempty source slice and reconstructs the original body; code selection retains only nonblank items, and empty document inputs remain excluded.

- [x] **DTS16 (new)** — [configured_remote_splits_document_bodies_to_the_effective_budget](../../../tests/document_backend_regressions.rs#L329). Index a headed document against a loopback mock backend with an 80-byte configured budget. **Oracle:** Four complete inputs keep the full breadcrumb prefix, each fits 80 bytes, and removing prefixes reconstructs the original heading body and paragraph body.

- [x] **DTS17 (revised)** — [remote_document_budget_includes_heading_breadcrumbs_and_rolls_back](../../../tests/document_backend_regressions.rs#L281). Revise the previous heading-budget fixture to a 40-byte budget that leaves no body capacity after full breadcrumbs. **Oracle:** The actionable error identifies budget, heading breadcrumbs, and no truncation; no indexed evidence or document-embedding transport request is published.
