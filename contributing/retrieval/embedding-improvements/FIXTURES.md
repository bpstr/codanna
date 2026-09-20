# Follow-up executable fixture inventory

This inventory compares the follow-up source with `dd8e247c62dcfd61dd1cdb7feaf23fe3777a469d`. Its machine-readable source is [fixtures.json](fixtures.json). It currently contains **68 new test functions, 7 revised existing tests, one subprocess helper, and 19 process-termination subcases**. Imported baseline tests and repeated invocations are excluded from the new-test count.

The final source passes every changed fixture in the complete local suites. These scenarios use deterministic local sources, mock transports or fixed vectors. They do not qualify a real embedding model or the ten manual acceptance scenarios. The source hash and complete gate results are recorded in [validation.json](validation.json).

Final verification records 75 new/revised functions passing and 0 pending. [LEXICAL-RESULTS.md](LEXICAL-RESULTS.md) records the separate, unchanged-corpus lexical acceptance run.

Each table names the exact Cargo target and module prefix; append the function name shown in a row to that prefix to select one test with `cargo test --locked TARGET FULL_NAME -- --exact`. For example, `cargo test --locked --test knowledge_source_evidence_regressions reference_links_use_active_definition_and_retain_both_source_spans -- --exact`.

## Embedding identity

Source: [src/documents/embedding.rs](../../../src/documents/embedding.rs). Cargo target: `--lib`.

Module prefix: `documents::embedding::tests::`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `identity_tracks_endpoint_and_model_without_persisting_url_credentials` | Revised | Vary backend endpoint, model and explicit model revision. Identity changes for each input, includes the input policy version, and contains neither URL credentials nor host text. | Passed — `default-tests-complete.log` |

## Document generation durability

Source: [src/documents/generation_tests.rs](../../../src/documents/generation_tests.rs). Cargo target: `--lib`.

Module prefix: `documents::store::generation_tests::`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `process_termination_reopens_one_matching_generation_at_every_publication_boundary` | New | Terminate a child process at each update, file removal and collection deletion publication boundary. Each of 19 exits returns code 86; reopen exposes one complete old or new generation, retry succeeds with fresh IDs, unrelated data survives, and staging is cleaned. | Passed — `default-tests-complete.log` |
| `failed_second_embedding_batch_keeps_generation_and_reserves_ids_across_reopen` | New | Fail the second embedding batch of a replacement exceeding 64 chunks. Original metadata, text and vectors remain queryable; reserved IDs survive reopen, successful retry uses higher IDs, and no chunks remain unembedded. | Passed — `default-tests-complete.log` |
| `compaction_bounds_one_hundred_updates_and_preserves_pinned_queries` | New | Replace one source 100 times while keeping a query snapshot pinned to its original generation. Live vectors remain 2, physical vectors at most 4, segments at most 8; at least 60 updates copy no old vectors, cumulative copied bytes at most 1200, and pinned/reopened queries return their own generations. | Passed — `default-tests-complete.log` |
| `committed_generation_repairs_missing_and_stale_state_mirrors` | New | Remove state.json or replace it with a stale mirror after a committed source update. Reopen returns committed replacement text and vectors and repairs the mirror to the matching content hash. | Passed — `default-tests-complete.log` |
| `failed_state_mirror_publication_preserves_committed_results_and_safe_retry` | New | Make state.json a directory so mirror publication fails after the durable commit. The error is visible while current and reopened queries return the complete committed generation; removing the obstruction permits an idempotent skipped-file retry. | Passed — `default-tests-complete.log` |
| `opening_committed_queries_does_not_wait_for_embedding_work_or_collect_its_staging` | New | Open another reader while a build lock and unpublished vector staging segment exist. The reader immediately sees committed evidence and leaves the active writer's unpublished staging intact. | Passed — `default-tests-complete.log` |
| `rejects_generation_traversal_duplicates_and_oversized_reservations` | New | Supply a traversal generation name, duplicate vector segment entries, and an oversized ID reservation file. Each malformed state is rejected when loading or reopening instead of being trusted. | Passed — `default-tests-complete.log` |
| `missing_vector_ids_are_reported_even_when_obsolete_records_hide_the_count_gap` | New | Corrupt a vector ID into another source's existing ID without changing the physical record count. Diagnostics report missing embeddings despite equal aggregate counts; reindex processes the affected source and restores matching evidence. | Passed — `default-tests-complete.log` |
| `corrupt_nonfinite_vector_is_rejected_instead_of_ranked` | New | Persist a NaN coordinate in an otherwise valid vector segment. Reopened search returns a non-finite-vector error rather than ranking corrupt data. | Passed — `default-tests-complete.log` |
| `document_store_excludes_its_own_files_from_broad_and_explicit_sources` | New | Use a broad JSON collection and an explicit path to metadata inside the document store, then change that metadata during three scans. Only the intended external JSON source is indexed; later scans skip that unchanged source and never add managed metadata or extra chunks. | Passed — `default-tests-complete.log` |
| `configured_custom_index_metadata_is_never_a_document_source` | New | Use the settings factory with a custom index path lacking ignore rules, broad JSON discovery and explicit code/document metadata paths. Three scans index only intended source; snapshots retain the managed-source exclusion, and reopen still skips unchanged source without ingesting metadata. | Passed — `default-tests-complete.log` |

## Document store compatibility

Source: [src/documents/store.rs](../../../src/documents/store.rs). Cargo target: `--lib`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `documents::store::tests::test_chunk_id_allocation` | Revised | Allocate IDs through the now-fallible reservation API. Three allocations succeed with unique sequential IDs; the preexisting allocation oracle is retained. | Passed — `default-tests-complete.log` |
| `documents::store::tests::test_state_persistence` | Revised | Reserve IDs and save state, then reopen the document store. Collection identity and the next ID survive reopen; only fallible allocation setup changed. | Passed — `default-tests-complete.log` |
| `documents::store::workspace_tests::hardening_workspace_document_snapshots_reject_foreign_hits_after_binding` | Revised | Contaminate a later generation with another workspace after pinning a root-scoped query. The old snapshot returns only safe original-root evidence, a fresh contaminated query fails, and reopening cannot bind the mixed store to the original workspace. | Passed — `default-tests-complete.log` |

## Embedding cache bounds

Source: [src/embedding_cache.rs](../../../src/embedding_cache.rs). Cargo target: `--lib`.

Module prefix: `embedding_cache::tests::`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `oversized_cache_is_ignored_before_reading_or_decoding` | New | Create a sparse malformed embedding-cache file larger than the file-byte limit. Loading returns an empty cache before reading or decoding its apparent oversized contents. | Passed — `default-tests-complete.log` |
| `bounded_decoder_rejects_large_vectors_and_excess_entries` | New | Provide one excessive-dimension vector and separately too many cache entries. Both persisted caches are rejected to an empty cache by bounded decoding. | Passed — `default-tests-complete.log` |
| `in_memory_capacity_accounts_for_vector_bytes` | New | Construct a cache at maximum vector dimension and attempt an over-dimension insertion. Capacity accounts for vector bytes within the total budget and invalid dimensions cannot be inserted. | Passed — `default-tests-complete.log` |

## Embedding input budgets

Source: [src/embedding_input.rs](../../../src/embedding_input.rs). Cargo target: `--lib`.

Module prefix: `embedding_input::tests::`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `byte_proxy_labels_estimate_and_rejects_utf8_without_slicing` | New | Apply a remote byte-proxy budget at a multilingual UTF-8 boundary. A 12-byte input passes, a 16-byte input fails with an explicitly labelled proxy/no-truncation message, and over-budget ASCII also fails. | Passed — `default-tests-complete.log` |
| `exact_count_includes_normalization_expansion_and_special_tokens` | New | Count a local fixture tokenizer whose normalization expands one Arabic presentation character. The expanded text plus special tokens is counted as six tokens and rejected against a five-token budget. | Passed — `default-tests-complete.log` |
| `exact_count_disables_existing_truncation_and_budgets_breadcrumbs` | New | Use an already-truncating tokenizer with a body that fits alone but not with heading ancestry. Validation disables truncation, counts all seven composed tokens, and reports heading breadcrumbs in the error. | Passed — `default-tests-complete.log` |
| `tokenizer_identity_is_stable_on_reload_and_changes_with_policy` | New | Serialize/reload a tokenizer, then vary its budget and normalizer. Identical serialization preserves identity; changed budget or normalization changes identity. | Passed — `default-tests-complete.log` |
| `oversized_single_token_is_rejected_before_tokenization` | New | Submit a single repeated token larger than the independent input-byte safety limit. Validation rejects it before tokenization and reports that input was not truncated. | Passed — `default-tests-complete.log` |

## Semantic context compatibility

Source: [src/indexing/facade_retrieval_tests.rs](../../../src/indexing/facade_retrieval_tests.rs). Cargo target: `--lib`.

Module prefix: `indexing::facade::retrieval_context_regressions::`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `semantic_context_preserves_results_when_one_impact_exceeds_budget` | Revised | Run semantic context with one result whose reverse impact exceeds the graph traversal budget and another isolated result. Both matches remain; their impact states are budget_exceeded without a fabricated count and complete with count zero. Setup now supplies matching embedding identity. | Passed — `default-tests-complete.log` |

## Semantic identity

Source: [src/semantic/simple.rs](../../../src/semantic/simple.rs). Cargo target: `--lib`.

Module prefix: `semantic::simple::tests::`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `semantic_identity_survives_reopen_and_rejects_equal_dimension_changes` | New | Persist semantic vectors, reopen without a model, then vary model revision and input-policy identity at the same dimension. Matching identity succeeds; each change requires reindexing despite unchanged vector dimension. | Passed — `default-tests-complete.log` |
| `legacy_semantic_vectors_require_identity_before_inference` | New | Present existing semantic vectors without recorded embedding identity. Inference validation fails, and existing vectors cannot simply be relabelled with a new identity. | Passed — `default-tests-complete.log` |

## Document watcher batching

Source: [src/watcher/document_batching_tests.rs](../../../src/watcher/document_batching_tests.rs). Cargo target: `--lib`.

Module prefix: `watcher::unified::document_batching_tests::`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `document_burst_reduces_measured_collection_scans_and_file_checks` | New | Deliver 64 settled source events and compare actual prior per-event scans with collection batching. Measured work drops from 64 scans/4096 file checks to one scan/64 checks while all 64 chunks remain indexed; elapsed time is diagnostic, not a threshold. | Passed — `default-tests-complete.log` |
| `high_cardinality_document_events_keep_only_configured_collections_pending` | New | Deliver 4096 distinct removal events across two configured document collections. Pending work contains only two collection entries, the shared path queue stays empty, and dispatch scans exactly those two collections. | Passed — `default-tests-complete.log` |
| `continuous_document_events_respect_quiet_window_and_maximum_delay` | New | Deliver events every 100 ms with a 500 ms quiet window. No premature scan occurs during the stream; maximum batch delay triggers exactly one scan and clears pending work. | Passed — `default-tests-complete.log` |
| `root_recreation_ignore_and_source_changes_share_one_truth_scan` | New | Delete/recreate a watched root while adding a subtree, source files and ignore policy, then remove policy and a source. Each burst uses one truth scan; root watches recover, ignored evidence stays absent, and final indexed paths match the settled filesystem. | Passed — `default-tests-complete.log` |
| `nested_root_ancestor_ignore_policy_is_observed` | New | Change an ancestor .codannaignore above a nested document root. One reconciliation removes excluded evidence and a later policy removal restores the same source. | Passed — `default-tests-complete.log` |
| `failed_collection_retries_on_idle_dispatch_after_backoff` | New | Make the first mock embedding call fail and dispatch again without another filesystem event. Committed state stays empty after failure, no early retry occurs, and an idle dispatch at the deadline succeeds and clears pending work. | Passed — `default-tests-complete.log` |
| `events_and_overflow_during_reconciliation_survive_for_next_dispatch` | New | Pause embedding while another event and an overflow signal arrive. The in-flight generation remains coherent; the next dispatch processes both current sources and clears the retained overflow signal. | Passed — `default-tests-complete.log` |
| `overlapping_source_handler_retains_modification_and_deletion_events` | New | Register another handler for the same document path and deliver create/delete events. Both handlers receive their modifications/deletions while document reconciliation scans once per settled change and removes deleted evidence. | Passed — `default-tests-complete.log` |
| `pending_retry_does_not_delay_cooperative_shutdown` | New | Cancel the watcher with document retry delayed 30 seconds. Cooperative shutdown completes inside the two-second timeout without waiting for retry eligibility. | Passed — `default-tests-complete.log` |
| `code_and_document_collections_reconcile_the_same_source_independently` | New | Register a Rust source in both code and document collections plus an independently indexed sibling code root, then remove the shared directory. Creation indexes its code symbol and document; removal clears both independently with one document scan while exactly one unrelated sibling-root symbol survives. | Passed — `default-tests-complete.log` |
| `observed_root_removal_does_not_treat_a_dangling_symlink_as_absence` | New | On Unix, replace an indexed source directory with a dangling symlink before processing observed-root removal. Removal reports false and retains the existing symbol while the symlink occupies the path; after removing the symlink, removal reports true and deletes that symbol. | Passed — `default-tests-complete.log` |
| `document_watch_roots_do_not_expand_code_indexing_on_reload` | New | Refresh watchers before a new document subtree event containing a Markdown file and an out-of-scope Rust file. The document is indexed, but refreshing document roots does not add the Rust file to code inventory. | Passed — `default-tests-complete.log` |
| `relative_code_actions_use_workspace_paths_and_keep_notification_paths` | New | Execute relative-path code creation/removal actions from a different process working directory. Workspace-relative resolution creates and removes the right symbol while notifications retain the original relative path. | Passed — `default-tests-complete.log` |
| `ignored_index_artifacts_cannot_requeue_broad_document_collections` | New | Deliver repeated created/modified/deleted generation-artifact events to a broad JSON document collection protected by ignore rules. Generated artifacts enter neither pending collection work nor the shared path queue and cause zero scans; policy changes and valid reincluded source edits still reconcile. | Passed — `default-tests-complete.log` |
| `custom_managed_index_events_do_not_enter_document_or_source_queues` | New | Deliver directory creation plus file modification/deletion events beneath a custom managed index root. Neither document nor source queues gain work and dispatch performs zero scans; an external ignore-policy edit still queues reconciliation. | Passed — `default-tests-complete.log` |
| `retry_backoff_is_capped_and_preserves_newer_pending_work` | New | Retry repeatedly, then merge a newer pending collection configuration. Backoff caps at 30 seconds, newer configuration wins, and work becomes ready exactly at the deadline. | Passed — `default-tests-complete.log` |

## Document watcher compatibility

Source: [src/watcher/unified.rs](../../../src/watcher/unified.rs). Cargo target: `--lib`.

Module prefix: `watcher::unified::document_collection_tests::`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `document_watcher_indexes_new_and_recreated_files_with_collection_overrides` | Revised | Create, delete and recreate a document using collection-specific chunking. After explicit batched dispatch, chunk counts remain 4, 0 and 1 and an initially empty directory stays watched. | Passed — `default-tests-complete.log` |
| `document_watcher_catches_moved_subtrees_and_nested_ignore_changes` | Revised | Move a subtree in, toggle nested ignore policy, remove it, and recreate the configured root. Explicit batched events preserve the existing evidence/removal/reinclusion oracle and the root's parent remains watched. | Passed — `default-tests-complete.log` |

## Embedding backend integration

Source: [tests/document_backend_regressions.rs](../../../tests/document_backend_regressions.rs). Cargo target: `--test document_backend_regressions`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `remote_embedding_preserves_relevant_evidence_beyond_old_character_limit` | New | Place relevant evidence beyond character 2000 in a document sent to a local mock provider. The complete original input reaches transport and the tail evidence affects the returned vector ranking. | Passed — `default-tests-complete.log` |
| `remote_document_budget_includes_heading_breadcrumbs_and_rolls_back` | New | Index chunks that fit individually but exceed the input budget after adding heading ancestry. The composed input is rejected without truncation or indexing partial results; no invalid document input reaches transport. | Passed — `default-tests-complete.log` |
| `explicit_remote_revision_rejects_equal_dimension_document_vectors` | New | Reopen document vectors with the same model alias and dimension but a different explicit revision. Persisted state records revision and input policy; reopening rejects the changed embedding identity. | Passed — `default-tests-complete.log` |
| `remote_preflights_all_batches_and_preserves_multilingual_inputs` | New | Put an oversized input after 64 valid inputs, then submit full multilingual text and a one-byte-budget case. All inputs are checked before any embedding batch reaches transport; valid Unicode remains complete and the small-budget probe/input path stays usable. | Passed — `default-tests-complete.log` |
| `configured_tokenizer_counts_complete_remote_input_and_special_tokens` | New | Configure a deterministic tokenizer with special tokens and submit long complete text followed by added heading text. The 503-token composed limit passes complete text, the 504-token variant fails, and only probe plus valid input reach transport. | Passed — `default-tests-complete.log` |
| `code_revision_change_rejects_reopened_vectors_before_query_embedding` | New | Reopen indexed code vectors against a changed equal-dimension model revision. Backend preparation rejects incompatible identity, marks semantic state incompatible, and allows only its dimension probe through transport. | Passed — `default-tests-complete.log` |
| `failed_code_hot_reload_blocks_direct_and_mcp_queries_without_transport` | New | Fail a semantic hot reload because an equal-dimension generation has incompatible identity, then load a compatible generation. Direct and MCP semantic queries stay blocked without provider query traffic after failure; an explicit compatible reload restores queries. | Passed — `default-tests-complete.log` |

## Document diagnostics

Source: [tests/document_diagnostics_regressions.rs](../../../tests/document_diagnostics_regressions.rs). Cargo target: `--test document_diagnostics_regressions`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `document_stats_reports_shared_vector_health_without_loading_a_model` | New | Run collection-scoped CLI document stats after two embedded collections and one lexical-only collection. Collection counts stay scoped while shared diagnostics report two live/physical vectors, one unembedded chunk, segments, generation and identity, without loading a model. | Passed — `default-tests-complete.log` |

## Knowledge source evidence

Source: [tests/knowledge_source_evidence_regressions.rs](../../../tests/knowledge_source_evidence_regressions.rs). Cargo target: `--test knowledge_source_evidence_regressions`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `reference_links_use_active_definition_and_retain_both_source_spans` | New | Use full, collapsed and shortcut Markdown references with duplicate definitions. The first active definition resolves every use; separate use/definition edges preserve exact source lines and hashes, and unused definitions remain inert. | Passed — `default-tests-complete.log` |
| `multiline_reference_use_and_definition_preserve_their_containing_lines` | New | Spread a reference use and its destination definition across separate lines. Usage spans cover lines 2–3, definition spans cover 5–6, and the spaced local filename resolves. | Passed — `default-tests-complete.log` |
| `parser_excludes_images_code_html_remote_and_unused_reference_definitions` | New | Mix one real link with images, nested image text, inline/fenced/indented code, HTML, escapes and remote/unknown references. Only the real local link becomes an edge and no fabricated unresolved destinations appear. | Passed — `default-tests-complete.log` |
| `local_paths_decode_once_and_keep_filename_delimiters_distinct_from_url_parts` | New | Link encoded spaces, Unicode, literal percent escapes, hash/question filename characters, escaped parentheses and source line anchors. All seven targets resolve; percent decoding happens once and URL delimiters remain distinct from filename characters. | Passed — `default-tests-complete.log` |
| `decoded_traversal_and_ambiguous_host_paths_never_resolve_as_local_files` | New | Try encoded root traversal, absolute/drive/UNC/backslash paths, controls, malformed escapes and query strings. Every unsafe target is rejected while an encoded parent step remaining inside the repository resolves normally. | Passed — `default-tests-complete.log` |
| `decoded_missing_anchors_and_out_of_range_lines_remain_unresolved` | New | Link to an encoded absent heading and a source line past EOF. Both retain unresolved original references and neither creates a link edge. | Passed — `default-tests-complete.log` |
| `comment_reference_definitions_respect_block_boundaries_and_source_lines` | New | Use one reference/definition inside a comment block and reuse its label in another declaration's block. Only the first block resolves; ownership, source line numbers and source hash remain exact. | Passed — `default-tests-complete.log` |
| `declarations_own_only_adjacent_comments_and_keep_real_rationale_spans` | New | Combine adjacent leading comments, in-body rationale, blank-line gaps, intervening statements and explicit file metadata. Only adjacent/in-body rationale attaches to the declaration; the other rationale stays with the file and preserves source spans/hashes. | Passed — `default-tests-complete.log` |
| `leading_comments_do_not_cross_scope_or_choose_between_colliding_declarations` | New | Put a leading comment at a different indentation and another before colliding declaration owners. Both rationales retain file ownership instead of crossing scope or guessing between declarations. | Passed — `default-tests-complete.log` |
| `inner_documentation_and_file_metadata_keep_file_ownership` | New | Use Rust inner documentation and a fileoverview block before declarations. Both rationales continue to explain the file rather than the following symbols. | Passed — `default-tests-complete.log` |
| `only_explicitly_linked_visible_configuration_is_ingested_without_symbols` | New | Reference a spaced configuration file and a configuration-to-configuration link while also supplying ignored/unreferenced/example/binary sources. Only permitted explicit configuration evidence loads with real content/hash and no symbols; removing document references removes those configuration sources on rebuild. | Passed — `default-tests-complete.log` |
| `linked_configuration_cannot_bypass_symlink_or_directory_ignore_boundaries` | New | Reference an escaping symlink, ignored configuration directory and encoded outside-root path. No configuration source is read or linked and all three references remain unresolved. | Passed — `default-tests-complete.log` |
| `linked_configuration_obeys_existing_file_byte_limits` | New | Reference a configuration file one byte above the existing per-source limit. Input construction fails with the byte-limit error instead of reading or truncating oversized evidence. | Passed — `default-tests-complete.log` |

## Retrieval ranking

Source: [tests/retrieval_ranking_regressions.rs](../../../tests/retrieval_ranking_regressions.rs). Cargo target: `--test retrieval_ranking_regressions`.

| Executable test | Change | Scenario and oracle | Root verification |
| --- | --- | --- | --- |
| `context_matches_word_forms_without_synonyms_or_implicit_traversal` | New | Search exact/inflected terms with unrelated, substring-only and foreign-repository controls at depth zero. Exact coverage wins, strong word-form overlap is recovered, foreign/unrelated/absent-term results stay absent, and output is deterministic. | Passed — `default-tests-complete.log` |
| `context_exact_entity_precedes_changed_file_and_small_budgets_remain_bounded` | New | Combine explicit entity and file seeds under a one-node, 2048-byte budget. The explicit entity wins, output fits the byte budget and truncation is reported. | Passed — `default-tests-complete.log` |
| `lexical_diversity_reaches_other_sources_beyond_a_long_matching_source` | New | Search a corpus dominated by one source with 200 repeating matching paragraphs. Top five retain the strongest source, recover at least three sources including complementary evidence, use at most two chunks per source and preserve real ranges/determinism. | Passed — `default-tests-complete.log` |
| `lexical_reranking_covers_full_question_before_repeated_partial_headings` | New | Compare full-question coverage, an inflected match and repeated partial heading terms. Full coverage ranks first, inflection second and repeated partial terms last; stemming alone never invents absent-term candidates. | Passed — `default-tests-complete.log` |
| `one_matching_source_keeps_evidence_and_document_collection_and_limit_filters` | New | Search one source with 12 matching sections across limits and document/collection filters. Valid single-source evidence fills the requested budget without duplicates; zero limits, missing terms and excluded scopes return no results. | Passed — `default-tests-complete.log` |
| `overflowing_finite_vectors_return_an_explicit_error_not_empty_or_partial_hits` | New | Use individually finite vector coordinates whose cosine arithmetic overflows alongside an ordinary valid candidate. Unfiltered search returns a structured non-finite-cosine error with corrective guidance; filtering to the valid source returns its finite expected cosine score. | Passed — `default-tests-complete.log` |
| `zero_vectors_keep_defined_finite_cosine_scores` | New | Query a corpus containing both an ordinary vector and a zero vector, then query with a zero vector. Ordinary/zero results score 1 and 0, and a zero query returns finite zero scores for both candidates. | Passed — `default-tests-complete.log` |
| `semantic_diversity_recovers_close_positive_sources_without_weak_fill` | New | Use fixed vectors with repeated strongest chunks, two close complementary sources and weak/zero/negative controls. Top five recover at least three strong sources with original cosine scores and no weak fill; one-source and zero/small-limit queries retain their valid results. | Passed — `default-tests-complete.log` |
| `semantic_nonpositive_cutoff_does_not_expand_to_worse_vectors` | New | Request two fixed-vector results whose original cutoff is zero. Positive and zero results remain, while diversification does not add a worse negative vector. | Passed — `default-tests-complete.log` |
| `semantic_scores_and_filters_remain_cosine_with_stable_ties` | New | Search tied fixed vectors repeatedly and apply document/unknown-collection filters. Scores remain true cosine values, tie order stays by chunk ID, repeated results match exactly and filters remain enforced. | Passed — `default-tests-complete.log` |

## Process-termination subcases and helper

All 19 rows belong to `--lib documents::store::generation_tests::process_termination_reopens_one_matching_generation_at_every_publication_boundary`. Each child exits without destructors at the selected boundary. The parent requires exit code 86, reopens the store, checks matching metadata/text/vector state and preservation of the unrelated collection, retries successfully with fresh IDs, and checks bounded storage plus staging cleanup.

| Action | Termination boundary | State required after reopen | Root verification |
| --- | --- | --- | --- |
| `update` | `before_vector_publication` | Original generation | Passed — `default-tests-complete.log` |
| `update` | `after_vector_publication` | Original generation | Passed — `default-tests-complete.log` |
| `update` | `before_metadata_commit` | Original generation | Passed — `default-tests-complete.log` |
| `update` | `after_metadata_commit` | Replacement generation | Passed — `default-tests-complete.log` |
| `update` | `before_state_publication` | Replacement generation | Passed — `default-tests-complete.log` |
| `update` | `after_state_publication` | Replacement generation | Passed — `default-tests-complete.log` |
| `update` | `after_embedding_batch` | Original generation | Passed — `default-tests-complete.log` |
| `remove` | `before_vector_publication` | Original generation | Passed — `default-tests-complete.log` |
| `remove` | `after_vector_publication` | Original generation | Passed — `default-tests-complete.log` |
| `remove` | `before_metadata_commit` | Original generation | Passed — `default-tests-complete.log` |
| `remove` | `after_metadata_commit` | Source/collection absent | Passed — `default-tests-complete.log` |
| `remove` | `before_state_publication` | Source/collection absent | Passed — `default-tests-complete.log` |
| `remove` | `after_state_publication` | Source/collection absent | Passed — `default-tests-complete.log` |
| `delete_collection` | `before_vector_publication` | Original generation | Passed — `default-tests-complete.log` |
| `delete_collection` | `after_vector_publication` | Original generation | Passed — `default-tests-complete.log` |
| `delete_collection` | `before_metadata_commit` | Original generation | Passed — `default-tests-complete.log` |
| `delete_collection` | `after_metadata_commit` | Source/collection absent | Passed — `default-tests-complete.log` |
| `delete_collection` | `before_state_publication` | Source/collection absent | Passed — `default-tests-complete.log` |
| `delete_collection` | `after_state_publication` | Source/collection absent | Passed — `default-tests-complete.log` |

`documents::store::generation_tests::crash_child` is a new `#[test]` helper in the same library target. It is **not ignored**: an ordinary run returns immediately when its test-only environment is absent. The helper’s standalone pass is not an additional crash fixture or subcase. Only the parent’s 19 checked subprocess exits establish the termination results.

## Imported and retained existing integration tests

The source-evidence target’s 25 passing invocations comprise 13 new fixtures and 12 imported unchanged knowledge tests. The original ranking target run’s 26 passes comprise eight new fixtures and 18 imported unchanged tests; any later ranking additions are listed separately above. The backend target retains three unchanged integration tests. The 12 knowledge functions execute in both new integration targets and are counted once as existing functions when discussing coverage.

| Existing function | Targets including it | Retained oracle |
| --- | --- | --- |
| `knowledge::io::tests::incomplete_dumps_fail` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Reject missing or inconsistent dump begin/summary/count records. |
| `knowledge::io::tests::dump_end_at_column_zero_excludes_the_following_line` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Convert half-open zero-column dump ends without claiming the next source line. |
| `knowledge::io::tests::replacement_roundtrip_and_size_limit` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Round-trip atomic graph replacement and enforce bounded reads. |
| `knowledge::io::tests::symlink_escape_is_rejected` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Reject a source symlink resolving outside the repository root. |
| `knowledge::links::tests::links_rationale_and_reverse_edges` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Preserve explicit symbol, rationale and decision reference edges. |
| `knowledge::links::tests::ambiguity_is_not_an_edge` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Keep ambiguous symbol mentions unresolved with both candidates. |
| `knowledge::links::tests::fences_do_not_create_fake_headings_or_links` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Exclude headings and references inside fenced code. |
| `knowledge::links::tests::replacement_drops_deleted_links_and_keeps_identity_after_line_shift` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Drop deleted-document edges while retaining shifted symbol identity. |
| `knowledge::links::tests::path_escape_is_rejected` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Reject traversal paths and unsafe repository identifiers. |
| `knowledge::links::tests::stable_output_and_roundtrip` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Build deterministic graph output and validate its serialization round-trip. |
| `knowledge::links::tests::corrupt_schema_and_dangling_edges_fail` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Reject unsupported schemas and missing edge endpoints. |
| `knowledge::links::tests::duplicate_headings_are_addressable` | `--test knowledge_source_evidence_regressions`, `--test retrieval_ranking_regressions` | Resolve the ordinal suffix of a duplicate heading anchor. |
| `context::tests::context_is_bounded_and_deterministic` | `--test retrieval_ranking_regressions` | Keep bounded graph context deterministic. |
| `context::tests::multibyte_and_escaping_count_toward_budget` | `--test retrieval_ranking_regressions` | Account for UTF-8 and serialized escaping in the byte budget. |
| `context::tests::no_match_is_not_silent_coverage` | `--test retrieval_ranking_regressions` | Report missing context coverage for unmatched queries. |
| `context::tests::scope_and_limit_validation` | `--test retrieval_ranking_regressions` | Validate repository scope and depth/node/byte limits. |
| `context::tests::path_is_shortest_and_depth_bounded` | `--test retrieval_ranking_regressions` | Return shortest bounded graph paths. |
| `context::tests::candidate_edges_are_not_traversed` | `--test retrieval_ranking_regressions` | Keep candidate-only relationships out of graph traversal. |
| `disabled_document_embeddings_never_initialize_configured_backend` | `--test document_backend_regressions` | Disabled document embeddings do not initialize even a configured remote backend. |
| `configured_remote_documents_use_model_dimension_and_heading_input_consistently` | `--test document_backend_regressions` | Configured remote documents use the provider dimension and composed heading/body input consistently. |
| `one_shot_document_search_reads_index_without_reembedding_changed_sources` | `--test document_backend_regressions` | One-shot document search reads committed evidence without reindexing or embedding changed source files. |

Other unchanged library tests are grouped by log and observed result in `unchanged_tests_in_root_library_filters` in the JSON. Cargo’s `Running` headers distinguish the library from binary and integration targets; this group contains only actual `--lib` invocations. Their passes retain baseline controls and do not inflate the count of new fixtures.

## Recorded root-run summaries

The complete default/all-feature suites exclude only the unchanged Unix socket test whose socket creation is denied in this environment. The unmodified full-test script records that environment failure; all changed fixtures are enabled and pass. The test remains enabled in CI. See validation.json for commands, counts and log hashes.

| Log | Recorded result |
| --- | --- |
| `full-test-script.log` | FAILED. 1375 passed; 1 failed; 27 ignored; 0 measured; 0 filtered out; finished in 16.14s |
| `all-feature-tests.log` | ok. 1377 passed; 0 failed; 27 ignored; 0 measured; 1 filtered out; finished in 16.69s |
| `all-feature-tests.log` | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s |
| `all-feature-tests.log` | ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s |
| `all-feature-tests.log` | ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s |
| `all-feature-tests.log` | ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s |
| `all-feature-tests.log` | ok. 62 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.66s |
| `all-feature-tests.log` | ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.59s |
| `all-feature-tests.log` | ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 39.97s |
| `all-feature-tests.log` | ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.51s |
| `all-feature-tests.log` | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s |
| `all-feature-tests.log` | ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s |
| `all-feature-tests.log` | ok. 36 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.23s |
| `all-feature-tests.log` | ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.06s |
| `all-feature-tests.log` | ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.17s |
| `all-feature-tests.log` | ok. 125 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 0.41s |
| `all-feature-tests.log` | ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s |
| `all-feature-tests.log` | ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.85s |
| `all-feature-tests.log` | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.42s |
| `all-feature-tests.log` | ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `all-feature-tests.log` | ok. 393 passed; 0 failed; 9 ignored; 0 measured; 0 filtered out; finished in 0.16s |
| `all-feature-tests.log` | ok. 50 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s |
| `all-feature-tests.log` | ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.35s |
| `all-feature-tests.log` | ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.35s |
| `all-feature-tests.log` | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `all-feature-tests.log` | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.32s |
| `all-feature-tests.log` | ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `all-feature-tests.log` | ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `all-feature-tests.log` | ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `all-feature-tests.log` | ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.33s |
| `all-feature-tests.log` | ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.62s |
| `all-feature-tests.log` | ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s |
| `all-feature-tests.log` | ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s |
| `all-feature-tests.log` | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.97s |
| `all-feature-tests.log` | ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.24s |
| `all-feature-tests.log` | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s |
| `all-feature-tests.log` | ok. 11 passed; 0 failed; 14 ignored; 0 measured; 0 filtered out; finished in 0.01s |
| `all-feature-tests.log` | ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `default-tests-complete.log` | ok. 1375 passed; 0 failed; 27 ignored; 0 measured; 1 filtered out; finished in 16.40s |
| `default-tests-complete.log` | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s |
| `default-tests-complete.log` | ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s |
| `default-tests-complete.log` | ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s |
| `default-tests-complete.log` | ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s |
| `default-tests-complete.log` | ok. 62 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.51s |
| `default-tests-complete.log` | ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.57s |
| `default-tests-complete.log` | ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 38.99s |
| `default-tests-complete.log` | ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.56s |
| `default-tests-complete.log` | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s |
| `default-tests-complete.log` | ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s |
| `default-tests-complete.log` | ok. 36 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.22s |
| `default-tests-complete.log` | ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.06s |
| `default-tests-complete.log` | ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.88s |
| `default-tests-complete.log` | ok. 125 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 0.38s |
| `default-tests-complete.log` | ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s |
| `default-tests-complete.log` | ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.90s |
| `default-tests-complete.log` | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.42s |
| `default-tests-complete.log` | ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `default-tests-complete.log` | ok. 393 passed; 0 failed; 9 ignored; 0 measured; 0 filtered out; finished in 0.17s |
| `default-tests-complete.log` | ok. 50 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s |
| `default-tests-complete.log` | ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.37s |
| `default-tests-complete.log` | ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.32s |
| `default-tests-complete.log` | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `default-tests-complete.log` | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.38s |
| `default-tests-complete.log` | ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `default-tests-complete.log` | ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `default-tests-complete.log` | ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `default-tests-complete.log` | ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.39s |
| `default-tests-complete.log` | ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.59s |
| `default-tests-complete.log` | ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s |
| `default-tests-complete.log` | ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s |
| `default-tests-complete.log` | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.96s |
| `default-tests-complete.log` | ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.28s |
| `default-tests-complete.log` | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.52s |
| `default-tests-complete.log` | ok. 11 passed; 0 failed; 14 ignored; 0 measured; 0 filtered out; finished in 0.01s |
| `default-tests-complete.log` | ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s |

Existing source-boundary and privacy assertions remain active. The revised pinned-snapshot privacy test explicitly checks both the safe old generation and rejection of a contaminated current generation; it does not relax cross-workspace isolation. The retrieval acceptance corpus, acceptance assertions and archived reports are unchanged by this inventory.
