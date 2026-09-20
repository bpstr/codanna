# Live document collection reload

Final combined local verification passes **17 new reload fixtures and one revised
workspace fixture**, together with all **17 splitting fixtures** and **18 retained
watcher controls**. The complete default/all-feature suites pass **2,371 / 2,373
tests** across 37 test-result groups, with 62 preexisting ignored tests and one unchanged Unix
socket fixture excluded because local AF_UNIX creation is denied. The revised
workspace-isolation fixture passes in both full suites. Formatting, strict Clippy,
no-default-feature compilation, strict docs, CLI/MCP smoke and 25 lexical evaluator
contracts pass.

The unchanged lexical acceptance corpus remains **29/29**, with **18/18 invariants**,
mean required-evidence recall@5 **1.0**, and zero forbidden evidence. See the
[combined validation report](COLLECTION-RELOAD-VALIDATION.json),
[all 35 changed Rust fixtures](COMBINED-RUST-FIXTURES.json),
[complete test and gate logs](collection-reload-results/), and
[lexical report](collection-reload-results/lexical-report.json). Real-model
relevance remains unmeasured; this packet records deterministic implementation
and lexical acceptance evidence.

The [unfiltered CI verification](CI-VERIFICATION.json) passes **2,372 default / 2,374 all-feature tests**, with zero filtered out. All 35 changed Rust fixtures, the unchanged Unix socket fixture and the revised workspace fixture pass in both runs. CI also passes strict docs, CLI checks, and the Linux/macOS watcher jobs. Each full suite prints 37 result summaries: 35 test-binary targets and two doctest groups. The archived CI log and Git tree identity bind these results to the tested source; later documentation commits do not change that source.

The persistent server's watcher now applies document collection settings edits without waiting for a source file event. This follow-up extends the collection watcher and durable document generations introduced before it. Earlier embedding reports remain historical records.

## Behavior

An edit to the watched `settings.toml` proposes a new configuration. The watcher validates the proposal, installs the required document watches, and reconciles added or changed collections in one document store transaction. It then publishes the matching document settings through the running `IndexFacade` and updates document event admission.

Supported edits include collection additions and removals; directory or explicit-file roots; glob patterns; default and collection-specific chunking; search preview settings; and `documents.enabled`. Relative collection roots resolve against the running workspace. Existing aliases resolve to canonical ownership, and the previous policy retains the canonical roots captured when it was accepted. Missing roots retain an ancestor watch; unresolved symlinks are rejected.

Removing a collection deletes its derived chunks, source ownership, fingerprints and live embedding references from the new index generation. Disabling documents clears the attached document store's indexed collections. Source documents remain on disk. Re-enabling an already attached store immediately reconciles the configured collections. Queries that already acquired a snapshot may finish against their pinned previous generation; subsequent snapshots observe the newly published generation.

Changing a glob or root preserves unchanged sources, chunk IDs and embeddings. Sources leaving a collection are retired before any new ownership is admitted, so a single settings edit can transfer a source to a differently named collection. Effective chunking changes replace affected chunks. Overrides that keep a collection's effective settings unchanged retain its existing chunks.

Pending source events and failed collection retries are synchronized with accepted configuration. Successful removal or disablement clears retired work, while unchanged collections retain their pending work. Previously captured actions cannot recreate a removed collection. Observing a newer settings event immediately cancels a stale proposal, before the newer edit's debounce window can expire. Configuration read failures also cancel stale proposals during native event overflow recovery. Reading a specific settings file requires that exact file; a deletion or read failure never substitutes default settings. Optional startup configuration discovery keeps its existing semantics.

## Validation and failure boundaries

Missing or unreadable settings files, malformed TOML, invalid globs, invalid merged chunking, unresolved symlinks, changed workspace or index roots, and changes to `semantic_search` settings are rejected before changing live document policy or configured code roots. ACL-scoped servers validate roots and canonical discovered sources against the original workspace boundary before source content reaches the embedding backend. Unrestricted local collection roots can remain outside the workspace. Managed index artifacts remain excluded from source discovery and event admission.

Document additions, replacements, ownership transfers and removals publish as one `DocumentStore` transaction. A discovery, source read, embedding, or pre-publication failure preserves the prior generation and live configuration. Configuration retries use the normal watcher dispatch with exponential backoff from 250 ms to a maximum 30 seconds, without requiring another file event. Source work remains coalesced while configuration publication is pending. Proposed watches are removed on a failed apply.

Tantivy's committed generation is authoritative. If publication succeeds and updating the redundant state mirror subsequently fails, the watcher accepts the matching configuration and reports the cleanup warning. It must not restore an old policy over newly committed documents. If a committed generation has not reached the live reader, configuration application first recovers it before applying the latest valid settings. Recovery then treats the complete configured collection set as authoritative, including retiring recovered collections absent from that set; even a reverted or otherwise unchanged settings proposal reconciles recovered data. Recovery checks the original workspace boundary before exposing or cleaning up recovered sources. A recovery or subsequent reconciliation failure restores the previous live metadata, vector handles and pinned searcher without changing the durable commit. Every retry or superseding proposal must therefore recover and fully reconcile again; chunk-ID reservations never rewind. Generation checks prevent mutations using stale in-memory state.

Code directory catch-up retains its existing independent commit semantics. A settings edit that changes code roots and document collections validates both before applying documents, publishes accepted roots after document success, and catches up code separately. This is not a transaction spanning both code and document indexes. The older directory catch-up pipeline currently assumes the process working directory is the workspace for relative source reads. This follow-up's overlap fixture seeds existing code through the absolute single-file lane and verifies that document policy changes preserve subsequent code updates; it does not claim to repair directory catch-up from another working directory.

A server started without an attached document store cannot enable document indexing through reload. This includes startup with documents disabled or with no existing document index. The watcher reports an explicit setup/restart error and preserves its running configuration. Enable and initialize the document index with `codanna documents index`, then restart the server. The watcher does not construct an unattached store or initialize an embedding model on this path.

Embedding backend settings, tokenizer configuration, model identity, workspace root and index root remain restart boundaries. The public legacy `WatchAction::ReloadConfig { added, removed, current }` constructor remains available for custom code-root handlers; the standard settings handler uses the additional `ReloadSettings` proposal variant. Custom exhaustive action matches must account for this additional variant.

## Exact fixture inventory

[COLLECTION-RELOAD.json](COLLECTION-RELOAD.json) provides 17 new reload fixtures and one revised retained workspace fixture (18 cataloged fixtures total), with exact test names, scenarios and machine-readable oracles.

The 17 new fixtures in the first table are defined in `src/watcher/collection_reload_tests.rs`. The fixture workspace contains only temporary local source files, settings, and derived indexes. `ReloadModel` returns fixed two-dimensional vectors and optionally a deterministic failure; it does not open a provider transport or load a model. Assertions inspect actual persisted documents and the running facade's settings snapshot.

| Fixture | Evidence |
| --- | --- |
| `collection_reload_add_remove_disable_and_reenable_publish_query_settings` | Settings-only add, remove, disable and re-enable; persisted search visibility; source preservation; query preview settings; obsolete watches; relative collection and index paths. |
| `collection_reload_globs_and_roots_preserve_retained_embeddings` | Newly admitted glob/root inputs are the only new mock embedding inputs; retained source IDs survive; shrinking roots removes retired sources. |
| `collection_reload_default_and_collection_chunking_replace_only_affected_chunks` | Default changes replace affected chunks while an effective override preserves another collection; later override edits replace that collection. |
| `collection_reload_invalid_settings_and_identity_changes_keep_working_policy` | Malformed TOML, invalid glob/default/override, workspace/index changes, and embedding identity or budget changes preserve live document settings, indexed code roots and durable chunk IDs. |
| `collection_reload_failed_batch_preserves_all_collections_then_retries_without_event` | A mocked embedding failure rolls back a simultaneous removal and addition; idle retry publishes the replacement; failed proposal does not enter the active watch registry. |
| `collection_reload_removal_prunes_pending_retry_and_rejects_captured_old_actions` | A pending failed collection and a captured old action cannot recreate removed data after a later dispatch. |
| `collection_reload_transfers_source_ownership_atomically_and_preserves_pinned_queries` | Transfer succeeds despite collection-name ordering; a pinned query retains its old generation; invalid overlapping ownership preserves the accepted configuration. |
| `collection_reload_committed_generation_wins_over_failed_state_mirror` | A local `state.json` directory obstructs redundant mirror publication after commit; accepted policy and durable query data still agree. |
| `collection_reload_managed_roots_and_code_document_overlap_keep_separate_admission` | Shared Rust source reaches both indexes; removing documents retains code updates; explicit managed roots admit no generated document. |
| `collection_reload_without_attached_store_requires_restart_without_loading_a_model` | Enabling through a watcher without an attached store leaves current settings and indexed data intact. |
| `collection_reload_alias_missing_roots_and_dangling_links_respect_workspace_boundary` | Unix alias normalization, missing-root creation, dangling symlink refusal, foreign ACL root refusal, and canonical file-symlink escape refusal. |
| `collection_reload_retargeted_startup_alias_compares_accepted_canonical_roots` | Unix startup alias retargeting compares against the previously accepted canonical root and immediately replaces the indexed inventory. |
| `collection_reload_overflow_cancels_pending_proposals_after_invalid_or_deleted_settings` | A failed proposal cannot later commit after malformed/deleted settings whose native event was lost; overflow cancels stale retry state. |
| `collection_reload_observed_settings_edits_cancel_retries_before_debounce` | A newer malformed/deleted settings event cancels the previous retry immediately, even while the new event is waiting in a longer debounce window. |
| `collection_reload_missing_or_directory_settings_never_apply_defaults` | A missing settings file or directory replacement produces an error through the real configuration handler; a newer event cancels the queued disable proposal and preserves settings, durable chunk ID and source. |
| `collection_reload_recovers_a_committed_generation_before_a_reverted_proposal` | An external durable publication with a temporarily missing manifest keeps recovery pending; restoring it followed by invalid source bytes preserves the old live query generation across failed reconciliation and a superseding no-op proposal; a later idle retry enforces the latest policy and honors another writer's reserved chunk IDs. |
| `collection_reload_recovery_checks_workspace_before_exposing_foreign_sources` | Recovered foreign provenance is rejected before it reaches the live reader or cleanup transaction. |

The retained fixture below is defined in `src/documents/store.rs`. Its setup now uses a distinct unscoped writer and workspace-bound reader of the same local index. This preserves the original query-boundary oracles while allowing the scoped source-discovery guard to reject unauthorized writes normally.

| Revised retained fixture | Evidence |
| --- | --- |
| `documents::store::workspace_tests::hardening_workspace_document_snapshots_reject_foreign_hits_after_binding` | The original unscoped writer publishes foreign data and an unscoped query confirms it exists; after explicitly reloading the separate scoped reader, its old pinned query exposes no foreign path or private text, its fresh query rejects the foreign row, and binding a reopened store rejects the foreign provenance. |

The two Unix symlink fixtures are conditionally compiled on Unix platforms. The fixtures do not rely on native notification delivery or real retry sleeps: they inject settings/source events and advance the dispatch clock directly. Native watch registration is still exercised against temporary local directories.

Run the focused regressions and existing document watcher coverage with:

```bash
cargo test --lib collection_reload -- --test-threads=1
cargo test --lib watcher::unified::document_ -- --test-threads=1
cargo test --lib documents::store -- --test-threads=1
git diff --check
```

No production credentials, downloaded models, or paid provider calls are needed for these tests. Fixture descriptions are evidence targets; the accompanying change review records the commands actually run and their results.
