# Deep Review Record

- **Repository:** `https://github.com/bpstr/codanna.git`
- **Reviewed commit:** `1b59d9b8a2f9d20204faea66671712dff66e6c75`
- **Reviewed branch:** `main`
- **Commit subject:** `Merge pull request #22 from bpstr/hardening/vector-generation-replacement`
- **Review request:** `rust full .`
- **Review date:** `2026-09-12`
- **Method:** AI-assisted, repository-wide static review using the Deep Code Review runner.
- **Fixture note:** This is preserved historical review evidence for the exact commit above. It must not be interpreted as current behavior, provider-quality evidence, real-usage evidence, settlement evidence, or launch qualification.

The generated, confidence-scored report follows verbatim.

---

# Final Code Review Triage

Confidence threshold: 80. Of 55 scored findings, 38 survive; 17 below-threshold findings were omitted because repository evidence did not strongly contradict their confidence assessments. `[NEW]` and `[PRE-EXISTING]` preserve the supplied change/history classification, including the review's path-scope caveat.

## P0 — Merge blockers

### 1. [NEW] Serve-lock reclamation can delete another live server's lock
- **Evidence:** `src/cli/commands/serve.rs:49-96` · **Confidence:** 96
- **Rationale:** Two reclaimers can both decide an old owner is dead; one can then unlink the other's newly acquired lock. The ownership-blind `Drop` has the same flaw, allowing concurrent servers on one Tantivy index with corruption risk.
- **Fix:** Hold an OS file lock for the server lifetime, or atomically verify an ownership token before removal. Add a deterministic two-contender regression.

### 2. [NEW] Vector replacement writers share one staging filename
- **Evidence:** `src/vector/storage.rs:205-248` · **Confidence:** 95
- **Rationale:** Separate instances can open and truncate the same `.<file>.replace.tmp` inode. One writer can rename it while another continues mutating the published file, corrupting the vector generation.
- **Fix:** Use a unique staging path per invocation, fully sync it, and atomically promote only that artifact. Test independent concurrent storage instances with prepared vectors.

### 3. [NEW] Chunked embedding publishes only the final chunk
- **Evidence:** `src/vector/engine.rs:71-124`; `src/indexing/pipeline/stages/embed.rs:15-88` · **Confidence:** 96
- **Rationale:** `EmbedStage` calls replacement-semantic `index_vectors` per 256-symbol chunk. At 257+ symbols, later calls discard prior chunks while counters still report them, causing silent index data loss.
- **Fix:** Assemble and atomically publish one complete generation, or introduce a distinct append API. Add a deterministic 257-symbol mocked regression.

### 4. [PRE-EXISTING] Network MCP authentication is forgeable or absent
- **Evidence:** `src/mcp/http_server.rs:243-455`; `src/mcp/https_server.rs:204-285` · **Confidence:** 98
- **Rationale:** HTTP accepts a fixed bearer token obtainable through a fixed unauthenticated code; HTTPS treats TLS as client authorization. Any reachable caller can query shared indexes and invoke operations.
- **Fix:** Implement identity-bound authentication on both transports, enforce scopes/workspace ACLs, and reject non-loopback binding unless secure authentication is configured.

### 5. [PRE-EXISTING] Project-registry load failures can erase all registrations
- **Evidence:** `src/init.rs:330-364` · **Confidence:** 95
- **Rationale:** Both registration paths convert every load failure into an empty registry and save it, overwriting malformed, unreadable, or incompatible data.
- **Fix:** Treat only `NotFound` as empty; propagate other errors, preserve the source file, and save atomically only after a successful load.

### 6. [PRE-EXISTING] OAuth parameters permit script injection and open redirects
- **Evidence:** `src/mcp/http_server.rs:322-401`; `src/mcp/https_server.rs:397-498` · **Confidence:** 98
- **Rationale:** Decoded `redirect_uri` and `state` values enter JavaScript and inline handlers without contextual escaping, and redirect targets are not checked against registered URIs.
- **Fix:** Require exact registered redirects, build URLs with a URL API, eliminate executable interpolation, and return a 303 instead of generated script-bearing HTML.

### 7. [PRE-EXISTING] Force-reindex can expose arbitrary filesystem content
- **Evidence:** `src/mcp/server.rs:272-355`; `src/indexing/facade.rs:1111-1237` · **Confidence:** 97
- **Rationale:** Requests accept any existing path. Canonicalization occurs without configured-root containment, so callers can index and later search host data outside the workspace.
- **Fix:** Enforce canonical containment, reject escapes/unsafe symlinks, authorize mutation separately, and bound traversal.

### 8. [PRE-EXISTING] Request diagnostics disclose credentials and OAuth payloads
- **Evidence:** `src/mcp/http_server.rs:261-338`; `src/mcp/https_server.rs:247-412` · **Confidence:** 98
- **Rationale:** Handlers print headers, cookies, token forms, OAuth state, registration JSON, and MCP request data verbatim, placing reusable credentials and sensitive payloads in logs.
- **Fix:** Log only allowlisted metadata and structurally redact credential, cookie, token, authorization, state, and body fields as `[REDACTED]`.

### 9. [PRE-EXISTING] Plugin extraction follows symlinks outside an untrusted clone
- **Evidence:** `src/plugins/resolver.rs:38-79`; `src/plugins/mod.rs:809-830` · **Confidence:** 98
- **Rationale:** Repository-controlled file and directory symlinks are dereferenced, allowing a clone to copy readable host files into an installed/indexed plugin tree.
- **Fix:** Reject symlinks, or canonicalize every entry and prove it remains below the clone root. Test absolute, parent-escaping, and directory links.

### 10. [PRE-EXISTING] Visualization renderers execute repository-controlled markup
- **Evidence:** `agents/plugins/claude/codanna-toolset/skills/{graph,x-ray}` renderer files · **Confidence:** 99
- **Rationale:** Repository strings are interpolated into inline scripts; `</script>` can break out and execute when the generated page auto-opens.
- **Fix:** Use non-executable JSON or context-safe encoding, escape markup, add CSP/injection fixtures, and default to `--no-open` until fixed.

### 11. [PRE-EXISTING] Executable-path overrides are interpolated into shell commands
- **Evidence:** `agents/plugins/claude/codanna-toolset/skills/{graph,x-ray}` dump scripts · **Confidence:** 99
- **Rationale:** `--binary PATH` enters `execSync` command strings, so shell metacharacters execute arbitrary commands.
- **Fix:** Use `execFileSync`/`spawnSync` with argv, validate the executable, and never derive this override from repository content.

## P1 — Should fix

### 12. [NEW] Async paths block while holding shared locks
- **Evidence:** `src/mcp/tools/search.rs:64-237,845-864`; `src/watcher/unified.rs:405-770`; `src/semantic/simple.rs:293-358` · **Confidence:** 94
- **Rationale:** Filesystem walks, embeddings, vector scans, document sync, and Tantivy writes run synchronously under facade/store locks, stalling Tokio workers and unrelated requests.
- **Fix:** Release guards before CPU/network/filesystem work and move blocking operations to bounded workers or an indexing actor. Use deterministic mocked concurrency tests.

### 13. [NEW] Single-file hot reload rebuilds the repository-wide symbol cache
- **Evidence:** `src/indexing/pipeline/incremental.rs:293-306`; `src/indexing/pipeline/types.rs:872-897`; `src/watcher/unified.rs:674-676` · **Confidence:** 98
- **Rationale:** Every edit rebuilds all symbol lookup maps under the facade write lock, with memory estimated at roughly 500 bytes per symbol.
- **Fix:** Maintain an incrementally updated cache; at minimum size from symbols and skip construction when no resolution work is pending.

### 14. [NEW] Single-file semantic persistence rewrites the full vector corpus
- **Evidence:** `src/indexing/pipeline/incremental.rs:259-306`; `src/semantic/simple.rs:537-585` · **Confidence:** 97
- **Rationale:** Every file change clones all embeddings and rewrites vector/language artifacts, even when the edit produced no embeddings.
- **Fix:** Persist bounded upsert/tombstone deltas, track dirty state, compact periodically, and avoid cloning borrowed data.

### 15. [NEW] Graph operations issue nested searches and per-node hydration
- **Evidence:** `src/indexing/facade.rs:481-862`; `src/storage/tantivy/query.rs:37-869`; `src/mcp/tools/{search,symbols}.rs` · **Confidence:** 94
- **Rationale:** Relationship queries hydrate every neighbor before truncation; impact traversal repeats four searches per node and then point-fetches symbols, creating N+1 scaling.
- **Fix:** Add limited relationship pages, bulk ID hydration, and frontier-batched traversal.

### 16. [NEW] HTTPS sessions overwrite one shared notification peer
- **Evidence:** `src/mcp/https_server.rs:175-210`; `src/mcp/server.rs:54-60,254-267`; `src/mcp/notifications.rs:79-153` · **Confidence:** 99
- **Rationale:** Cloned servers share one `Arc<Mutex<Option<Peer>>>`; each initialization overwrites it, so only the newest session receives notifications.
- **Fix:** Construct a server/listener per session, sharing only immutable state, or use rmcp's session-scoped subscription path.

### 17. [NEW] HTTP notification listeners survive disconnected sessions
- **Evidence:** `src/mcp/http_server.rs:179-218`; `src/mcp/notifications.rs:66-164` · **Confidence:** 99
- **Rationale:** Per-connection tasks are tied only to process cancellation, retaining disconnected peers/stores and fanning events to stale listeners.
- **Fix:** Tie each listener and `JoinHandle` to session closure and terminate it on send failure.

### 18. [NEW] Behavior lookup can panic or substitute Rust semantics
- **Evidence:** `src/parsing/factory.rs:345-358` · **Confidence:** 88
- **Rationale:** Unknown language IDs can receive `RustBehavior`; a poisoned registry mutex is unconditionally unwrapped. Both violate language-sensitive parsing.
- **Fix:** Return typed errors for poison and unknown IDs; never substitute another language's behavior.

### 19. [NEW] The crate exposes two incompatible storage error types
- **Evidence:** `src/error.rs:219-253`; `src/storage/error.rs:5-47`; `src/lib.rs:33-42` · **Confidence:** 98
- **Rationale:** The root re-exports `error::StorageError`, while public storage APIs return distinct `storage::StorageError`, breaking intuitive matching/propagation for consumers.
- **Fix:** Re-export the active type and remove or explicitly rename the legacy type under a compatibility plan.

### 20. [NEW] Legacy parser construction rejects implemented JavaScript/TypeScript
- **Evidence:** `src/parsing/factory.rs:40-385`; `src/parsing/language.rs:28-188`; `src/parsing/registry.rs:296-395` · **Confidence:** 94
- **Rationale:** Public `create_parser` rejects JavaScript and TypeScript while registry-backed paths construct them, producing inconsistent supported APIs.
- **Fix:** Delegate legacy construction to the registry, deprecate duplicates, and add a compiled-language completeness test.

### 21. [NEW] Compact symbols truncate 32-bit file IDs to 16 bits
- **Evidence:** `src/symbol/mod.rs:81-93,194-208,295-338` · **Confidence:** 96
- **Rationale:** Infallible `as u16` conversions alias IDs above 65,535 and can convert 65,536 to invalid zero, corrupting file identity at scale.
- **Fix:** Retain `u32`, or use checked conversion into an explicit compact-ID type. Privatize fields and test boundaries.

### 22. [NEW] Invalid chunking settings reach production indexing
- **Evidence:** `src/documents/config.rs:72-190`; `src/cli/commands/documents.rs:347-355`; `src/documents/chunker.rs:72-89` · **Confidence:** 99
- **Rationale:** Public/Serde fields bypass an unused validator. Zero maximum or overlap at least the window can force one sliding-window iteration per character.
- **Fix:** Convert external settings into a validated runtime type at the boundary and accept only that type in chunkers.

### 23. [NEW] TypeScript parser tests race through process-wide CWD
- **Evidence:** `tests/parsers/typescript/test_alias_resolution.rs:135-218`; `tests/parsers/typescript/test_pipeline_resolution.rs:275-443` · **Confidence:** 98
- **Rationale:** Tests in one process change CWD under different/no locks and restore manually after panic-prone operations, enabling contamination and cascading failures.
- **Fix:** Pass explicit roots; meanwhile use one global lock, RAII restoration, and unique temp directories.

### 24. [NEW] CLI shell regressions neither run in CI nor fail reliably
- **Evidence:** `tests/cli/*.sh`; `tests/cli_tests.rs:1-47`; `.github/workflows/full-test.yml:53-62` · **Confidence:** 98
- **Rationale:** Cargo/CI do not invoke four scripts, and failed checks are followed by successful commands so exit status often remains zero.
- **Fix:** Migrate to Rust integration tests, or aggregate status/fail fast and invoke explicitly in CI.

### 25. [PRE-EXISTING] Search limits can panic Tantivy or request enormous allocation
- **Evidence:** `src/mcp/requests.rs:52-69`; `src/mcp/tools/search.rs:722-753`; `src/storage/tantivy/query.rs:48-169` · **Confidence:** 99
- **Rationale:** Unbounded `u32` limits reach Tantivy 0.26.1 `TopDocs`; zero asserts and near-maximum values attempt doubled-capacity allocation.
- **Fix:** Enforce small server-owned maxima for all search/graph limits and reject invalid parameters before Tantivy.

### 26. [PRE-EXISTING] Deleting one collection drops every collection's file state
- **Evidence:** `src/documents/store.rs:744-782` · **Confidence:** 99
- **Rationale:** The predicate removes every nonempty file state regardless of collection, causing surviving chunks to be re-added and watcher tracking lost.
- **Fix:** Retain states belonging to other collections and add a two-collection regression.

### 27. [PRE-EXISTING] MCP config executes a mutable Context7 package on first use
- **Evidence:** `.mcp_stdio.json:3-8` · **Confidence:** 93
- **Rationale:** `npx -y @upstash/context7-mcp` downloads and executes an unversioned registry tag outside the lockfile.
- **Fix:** Install an audited exact version under a committed lockfile and use `npx --no-install`.

### 28. [PRE-EXISTING] Relationship persistence failures count as resolved
- **Evidence:** `src/indexing/pipeline/stages/write.rs:74-118`; `src/indexing/pipeline/phase2.rs:106-180` · **Confidence:** 96
- **Rationale:** Phase 2 ignores write/commit failures while incrementing counters, reporting failed durable writes as successful resolution.
- **Fix:** Aggregate durable-write stats, propagate commit errors, and count only stored relationships.

### 29. [PRE-EXISTING] Profile removal and team sync return false success
- **Evidence:** `src/profiles/mod.rs:443-559` · **Confidence:** 99
- **Rationale:** Removal deletes the lock entry despite file-deletion errors; sync logs install failures then unconditionally returns success.
- **Fix:** Preserve partial state, return collected failures with a nonzero outcome, and provide retry details.

### 30. [PRE-EXISTING] MCP notification delivery failures are discarded
- **Evidence:** `src/mcp/notifications.rs:97-148`; `src/mcp/server.rs:121-142` · **Confidence:** 96
- **Rationale:** Send results are ignored and success logged unconditionally, leaving subscribed clients stale without an actionable signal.
- **Fix:** Check every send, record nonsensitive session/event context, and stop or reconnect closed listeners.

### 31. [PRE-EXISTING] Traversal errors silently produce incomplete indexes/watches
- **Evidence:** `src/indexing/walker.rs:58-61,91-95` · **Confidence:** 97
- **Rationale:** `filter_map(Result::ok)` discards permission/I/O errors, so unreadable subtrees disappear while indexing appears complete.
- **Fix:** Preserve path-specific errors and fail or explicitly report partial success.

### 32. [PRE-EXISTING] Corrupt index metadata is treated as absent and overwritten
- **Evidence:** `src/storage/persistence.rs:60-68,159-200` · **Confidence:** 86
- **Rationale:** Load failures become `None` and later fresh metadata, risking loss of indexed paths/stamps despite some CLI mitigation.
- **Fix:** Treat only `NotFound` as optional; preserve and surface corrupt/unreadable metadata and require explicit recovery.

## P2 — Worth noting

### 33. [NEW] Narrow layouts replace interactive mail with a screenshot
- **Evidence:** `examples/typescript/react/src/app/(app)/examples/mail/page.tsx:16-39`; `examples/typescript/react/src/app/demo/page.tsx:48-68` · **Confidence:** 94
- **Rationale:** Below `md`, controls/content disappear. This is concrete for keyboard, screen-reader, zoom, and narrow-viewport users, but confined to an example app.
- **Fix:** Preserve an interactive sequential layout and verify at 320px/400% zoom with keyboard and screen reader.

### 34. [NEW] Theme color controls lack accessible names and selected state
- **Evidence:** `examples/typescript/react/src/components/theming/ThemeCustomizer.tsx:309-362` · **Confidence:** 98
- **Rationale:** Empty color-only buttons expose neither unique names nor radio/pressed state; the defect is example-only.
- **Fix:** Add names, radio/pressed semantics, and a non-color selection indicator.

### 35. [NEW] Public documentation materially misstates current behavior
- **Evidence:** `src/vector/embedding.rs:7-145`; `src/indexing/facade.rs:1-100`; `src/parsing/{factory.rs,language.rs,language_behavior.rs,java}`; `src/indexing/pipeline/types.rs:404-461` · **Confidence:** 84
- **Rationale:** Several public comments/examples are stale or unusable. The original bundled claim overstates some TODO/UTF-8 comments, so change only verified defects.
- **Fix:** Update architecture/status prose, compile examples, describe DashMap guards accurately, and document/enforce substring boundaries.

### 36. [NEW] Several tests have weak or artificial oracles
- **Evidence:** `tests/exploration/abi15_grammar_audit`; `tests/integration/test_parse_command.rs`; `tests/parsers/typescript/test_alias_resolution.rs`; `tests/cli/test_serve_http_sessionless.rs` · **Confidence:** 93
- **Rationale:** Tests permit fallback/vacuous success, duplicate production logic, tolerate either alias outcome, or release a port before bind.
- **Fix:** Fail audits on missing inputs/errors, require nonempty parse output, exercise production alias resolution, and eliminate free-port TOCTOU.

### 37. [PRE-EXISTING] Plugin runtime prerequisites are incomplete
- **Evidence:** `agents/plugins/claude/codanna-toolset/.claude-plugin/plugin.json`; related skill/README files · **Confidence:** 88
- **Rationale:** The plugin depends on system Node and version-sensitive `require(esm)`, but setup material lacks consistent tested minima/recovery steps.
- **Fix:** Declare Node/Codanna minima and add non-installing preflight checks with recovery guidance.

### 38. [PRE-EXISTING] Language and MCP additions require synchronized catalogs
- **Evidence:** `src/parsing/{language.rs,factory.rs,registry.rs}`; `src/mcp/service.rs`; `src/cli/commands/mcp.rs`; `src/config/defaults.rs` · **Confidence:** 89
- **Rationale:** Features are duplicated across registries, dispatch, schemas, defaults, and help, creating omission risk without an immediate failure.
- **Fix:** Centralize typed descriptors, derive metadata/dispatch, finish registry migration, and add completeness checks.

## Review coverage and gaps

- All requested specialists completed, and shared stack profiling succeeded.
- Codanna discovery was unavailable under the run's no-approval constraints; reviewers used read-only source/Git inspection.
- Tests and builds were not run, so validation is static. Regression checks must use deterministic fixtures and mocked provider transports under the paid-inference policy.
- Prior GitHub review comments were unavailable because the saved token was invalid and the API was unreachable.
