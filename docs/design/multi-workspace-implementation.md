# Multi-workspace implementation checklist

**Status:** Design-first branch. Application implementation has not started.

**Canonical contract:** [Product-level multi-workspace support](multi-workspace.md).

The required hierarchy is **Assign workspace -> member repositories**, alongside
an independent **Codanna workspace -> Codanna repository**. Do not turn Assign
members into separate user-facing workspaces and then require a workspace group.

## Review scope of this branch

This initial PR adds the corrected feature brief, this delivery checklist, and
a roadmap entry. It does not add CLI commands, change index formats, implement
MCP routing, or claim that any runtime acceptance test has passed.

Review the ownership model, membership authority, migration policy, transport
boundaries, and rollout order before changing compatibility-sensitive code.
Future commits should state which slice they implement and which checks actually
ran. Keep the PR draft until the intended merge scope is explicit; nothing in
this checklist authorizes an automatic merge or production deployment.

## S1. Deterministic discovery and registry lifecycle

Primary areas: `src/config/mod.rs`, `src/config/paths.rs`, `src/init.rs`,
`src/cli/args.rs`, `src/cli/commands/`, `src/main.rs`.

- [ ] Add typed workspace identity and scoped aliases using the existing project
  registry infrastructure; keep one authoritative registry at `projects.json`.
- [ ] Resolve from an explicit start path without changing process cwd. Require
  a project configuration, not merely a `.codanna` directory. Return a scoped
  error for a malformed nearest configuration rather than selecting an ancestor.
- [ ] Define and test stable-ID move behavior and copied-marker/clone conflicts.
- [ ] Implement idempotent registration, list/show, rename, move, unregister, and
  metadata-only diagnostics with structured output.
- [ ] Lock the complete registry read-modify-write transaction, persist safely,
  preserve corrupt/unknown-version input, and migrate recoverably from v1.
- [ ] Add explicit CLI workspace selection before provider/model/index setup.
  Reject contradictory `--workspace` and `--config` choices.
- [ ] Ensure read/list/doctor paths have no indexing, model, or configuration-write
  side effects. Unregister must leave source files and local index data intact.
- [ ] Keep existing local single-workspace commands compatible.

**Regression gate:** real filesystem fixtures for symlinks, nested folders,
missing roots, global-cache-only directories, malformed configs, aliases,
moves, copies, and multiple concurrent registry-writing processes. No tests
mutate the developer's actual home, cwd, registry, or provider environment.

**Exit criterion:** Assign and Codanna can be selected independently by CLI;
registration alone changes no index contents. Product membership comes next.

## S2. Multi-repository ownership inside a workspace

Primary areas: configuration, indexing path identity, storage metadata, parsing
and project resolution, documents, and the repository's recall integration.

- [ ] Add explicit member-repository configuration to workspace settings, with
  canonical paths, repository IDs, aliases, and validated optional external roots.
- [ ] Implement repository add/list/remove against that configuration, not a
  second registry-side source list. Preview legacy `indexed_paths` migration.
- [ ] Map a member checkout or nested source folder back to its product workspace;
  disallow ambiguous overlap and duplicate canonical membership across workspaces.
- [ ] Preserve repository ownership in file keys, symbol references, resolver
  lookups, document scopes, and recall mappings. Support workspace-wide docs and
  conversations without inventing a repository owner.
- [ ] Decide and implement the minimal versioned storage change needed for that
  provenance. Reuse existing compatibility gates; never mix old/new row semantics.
- [ ] Partition resolver candidates by repository. Do not infer cross-repository
  edges from matching names, package aliases, or semantic similarity.
- [ ] Preserve member-specific language config and ignore behavior explicitly.
  Never silently import or overwrite a nested standalone `.codanna` index.
- [ ] Define member-removal state so detached sources cannot remain silently
  searchable. Purging indexed data must be explicit and scoped.
- [ ] Give public references a workspace, repository, and index-generation
  namespace without replacing every internal local symbol ID.

**Regression gate:** Assign contains `assign-core` and `assign-web`; Codanna is
a different workspace. All contain identical filenames and symbol names. Assert
exact repository ownership, default search across Assign members, exclusion of
Codanna, and no false cross-repository graph edges. Include collection aliases,
recall text, stale references after rebuild, external roots, and member removal.

**Exit criterion:** one product workspace safely contains multiple repositories.
A router alone must not be used to bypass this gate.

## S3. Workspace runtime and writer ownership

Primary areas: `src/cli/commands/serve.rs`, `src/project_resolver/`,
`src/documents/`, `src/watcher/`, `src/mcp/tools/context.rs`, persistence.

- [ ] Introduce an immutable resolved workspace context containing exact config,
  roots, storage paths, repository mappings, and recall binding.
- [ ] Implement a small lazy worker manager around the existing single-workspace
  engine. Use one worker per workspace, not one per repository.
- [ ] Set worker cwd and environment only at process creation. Strip unrelated
  recall/config overrides; never mutate the router's cwd or global environment.
- [ ] Establish writer/watch ownership across OS processes, including CLI writers
  and separately spawned stdio clients. Refuse or forward conflicting writes.
- [ ] Coalesce concurrent loads, pin active runtimes, bound cold starts and memory,
  and isolate per-workspace failures with restart backoff.
- [ ] Distinguish persistent watch leases from query-only idle runtimes. Do not
  silently stop an explicitly requested watch during eviction.
- [ ] Give rebuilds durable status/receipts. Preflight all selected roots before
  changing storage; keep previous data recoverable on failure.
- [ ] Test graceful shutdown and child cleanup. Do not replay uncertain mutations
  automatically when a worker disconnects.

**Regression gate:** duplicate loads, multiple-process writer attempts, one
workspace failing while another serves queries, query during rebuild, missing
rebuild roots, cancellation, eviction, worker crash loops, and shutdown. Verify
code, docs, and recall use the same context even with conflicting inherited env.

**Exit criterion:** workspace-local lifecycle and failures are enforced rather
than assumed. No MCP scope expansion is enabled yet.

## S4. Workspace-aware MCP routing

Primary areas: `src/mcp/server.rs`, `src/mcp/requests.rs`, `src/mcp/tools/`,
`src/mcp/http_server.rs`, transport/auth integration, resources/notifications.

- [ ] Add centralized scope resolution and authorization before opening a runtime.
- [ ] Add workspace enumeration/details without loading every registered index.
- [ ] Add consistent workspace selection and appropriate repository filters to
  search, context, documents, symbols, graph tools, and index information.
- [ ] Route existing custom requests, resource reads, and notifications too;
  leaving those on the old default facade is not acceptable.
- [ ] Return structured provenance plus compatible readable text. Validate
  qualified references and scope all cache keys and continuation tokens.
- [ ] Enforce session-local defaults; reject absent/ambiguous scope instead of
  using a mutable global current workspace or the last-used workspace.
- [ ] Preserve single-workspace stdio and legacy unambiguous requests.
- [ ] Implement a shared HTTP service using existing authentication and host/origin
  controls. Explicitly document that per-client stdio routers do not share a
  cross-process worker pool without a separate service bridge.
- [ ] Make workspace allowlists authoritative for discovery and operations. Do
  not disclose unauthorized workspace metadata or absolute paths in errors.

**Regression gate:** concurrent sessions targeting Assign and Codanna; exact
request/response schema tests for every affected tool; unknown keys; conflicting
selectors; stale generation IDs; authorization failures; notification/resource
isolation; and network transport regression tests.

**Exit criterion:** one shared HTTP MCP service can safely serve both workspaces,
including product-wide Assign queries, without a new unauthenticated endpoint.

## S5. Client roots and day-to-day usability

- [ ] Negotiate roots only for clients that support them; bound discovery latency.
- [ ] Resolve `assign-web` and `assign-core` roots to the same Assign workspace.
- [ ] Keep roots spanning Assign and Codanna ambiguous until explicitly scoped.
- [ ] Handle symlinks, unregistered roots, roots changes, reconnects, and clients
  without roots support. Root updates affect future requests, not active ones.
- [ ] Treat roots and optional `project_path` solely as routing hints within
  authorized registrations; never index or register arbitrary client paths.
- [ ] Add workspace-labelled errors with exact recovery commands and metadata-only
  doctor output. Show unknown/incomplete indexing honestly.
- [ ] Update user-facing CLI help, MCP instructions, and README only for behavior
  that is implemented and verified.

**Regression gate:** protocol fixtures for all root scenarios and at least one
real client smoke test without model/inference spending. Record client versions
and observed capabilities; do not assume all clients implement roots.

**Exit criterion:** a client opened in an Assign member repository gets Assign
context automatically when supported, with explicit selection as a reliable fallback.

## S6. Qualification and release

- [ ] Execute the full three-repository/two-workspace isolation matrix in the
  feature brief for code, documents, recall, graphs, and resource notifications.
- [ ] Verify a force rebuild/removal in Assign leaves Codanna's persisted data
  unchanged. Verify repository-scoped operations preserve other Assign members.
- [ ] Exercise registry and writer locks in separate processes on supported OSes.
- [ ] Register many unloaded fixtures and measure metadata-only startup, loaded
  runtime count, peak memory, cold-load queues, and steady-state routing overhead.
- [ ] Compare warm queries with the direct single-workspace baseline. Report model
  startup separately; do not present unmeasured latency targets as results.
- [ ] Run focused tests, then repository quick/full gates with deterministic data,
  provider credentials removed, and inference mocked or disabled.
- [ ] Add narrow CI regression coverage using existing repository workflows where
  possible; do not duplicate expensive build matrices without justification.
- [ ] Document migration/rollback limits, old-binary behavior, watch semantics,
  remote permissions, remaining limitations, and explicit recovery commands.
- [ ] Record evidence for completed checkboxes and obtain review before merge.

**Exit criterion:** the feature's end-to-end scenario is reproducible and all
claimed isolation boundaries have executable evidence.

## Deferred work

Cross-workspace federated search, workspace groups, automatic cross-product graph
relationships, shared embedding runtimes, an implicit always-running daemon, and
an in-process multi-workspace engine are deliberately outside S1-S6. Revisit them
only after the core product-workspace experience is reliable and measured.

## Validation of this design-only PR

For the initial documentation change, validate changed-file whitespace, relative
links, and TOML/JSON example syntax. Every new command and config field must remain
labelled as proposed. Check that the root roadmap does not advertise a released
feature. Rust compilation, runtime tests, and performance claims are not applicable
to this docs-only diff and must not be reported as passed.
