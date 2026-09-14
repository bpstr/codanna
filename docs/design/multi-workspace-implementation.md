# Multi-workspace implementation checklist

**Canonical contract:** [Isolated workspaces](multi-workspace.md).
**Implemented usage:** [Workspace guide](../workspaces.md) and
[local MCP router guide](../workspace-mcp.md).

## Scope

Build a generic solution for independent coding-agent workspaces, each with a
separate graph/index. Workspaces come from runtime discovery and configuration,
not a predefined project list. A workspace can contain one repository, a monorepo,
or multiple intentionally grouped repositories; none of those layouts is mandatory.

Normal usage must not require manual registrations, IDs, member inventories, or
an MCP definition per workspace. Do not infer membership from matching names or
merge unrelated graphs just because their directories share a parent.

Documentation, tests, instructions, and defaults must use synthetic placeholders
or dynamically generated fixtures, never real user project names or private paths.

## Implemented increments under review

The branch contains registry administration, explicit CLI selection, automatic
local setup/discovery, and an opt-in local read-only MCP router. The router has
workspace/path selectors, client-root resolution, result ownership, and a bounded
lazy reader pool per stdio connection. These are not claims of a shared daemon,
complete repository provenance, or completion of every acceptance gate below.

Selected and automatic launches still disable inherited conversation recall.
Initial indexing is explicit. Network routing, index writer/watch coordination,
and scoped resource/subscription forwarding remain separate work. Consult the
PR for exact commits and observed validation, not historical aggregate test counts.

## S1. Discovery and registry qualification

- [ ] Qualify arbitrary workspace names and paths, nearest checkouts/manifests,
  configured-source ownership, and optional multi-root boundaries.
- [ ] Cover malformed applicable configs, cache-only state directories, HOME/root
  rejection, symlinks, external-source rejection, and discovery budgets.
- [ ] Verify automatic initial settings/source/alias selection preserves existing
  configuration, exclusions, and populated indexes.
- [ ] Qualify idempotent add/list/show/rename/move/remove/doctor and metadata-only
  diagnostics; management must not rebuild indexes or delete source data.
- [ ] Verify full-transaction registry locks, atomic replacement, stale snapshots,
  concurrent processes, corrupt/versioned input, and recovery semantics.
- [ ] Qualify copies/worktrees and relocation identity without shared mutable data.
- [ ] Preserve explicit `--config`, `--workspace`, and legacy CLI behavior while
  rejecting contradictory selection before provider or index loading.

**Exit:** independently discovered workspaces remain distinct regardless of their
names. Registration is metadata-only. Manual setup is an override, not a prerequisite.

## S2. Optional member provenance and graph isolation

- [ ] Discover repository membership under an established workspace boundary,
  preserving exclusions and permitting explicit unusual-layout overrides.
- [ ] Keep one membership authority; preview any `indexed_paths` migration.
- [ ] Preserve member identity in file keys, symbols, resolver state, documents,
  recall, and exact references; workspace-wide content remains valid.
- [ ] Partition resolution so equal names/aliases do not invent cross-repository
  edges. Permit only relationships supported by actual dependency evidence.
- [ ] Introduce versioned row/API changes and generation-qualified references
  with explicit compatibility/rebuild handling, never mixed row semantics.
- [ ] Preserve member language/ignore behavior and existing standalone indexes.
- [ ] Make removed members non-queryable through their former scope, with explicit
  safe data cleanup and no ambiguous shared writable ownership.

**Exit:** both single-repository and multi-repository workspaces are qualified.
Use synthetic fixtures with colliding paths/symbols; no named product topology is
an acceptance requirement. Workspace routing alone does not qualify member graphs.

## S3. Runtime and cross-process ownership

- [ ] Qualify immutable workspace context for settings, stores, resolver, and recall.
- [ ] Verify lazy loads, load coalescing, active/queued pinning, bounded admission,
  eviction, independent execution, cancellation, shutdown, and failure backoff.
- [ ] Establish one index writer/watch authority across CLI and MCP processes;
  registry locking alone is insufficient. Forward or reject conflicts safely.
- [ ] Distinguish persistent watches from query-only leases and bound resource use.
- [ ] Preflight rebuild scope, preserve prior data on failure, and provide accurate
  job status/receipts. Do not replay mutations after uncertain disconnects.
- [ ] Remove cwd/environment assumptions before introducing in-process runtimes.

**Exit:** failing or rebuilding one workspace does not mutate or block unrelated
graphs. Document the difference between per-connection pools and shared workers.

## S4. MCP scope and transport qualification

- [ ] Qualify centralized scope and authorization before runtime access for every
  supported tool, exact reference, error, cache key, and continuation.
- [ ] Verify workspace enumeration/details do not load all indexes or models.
- [ ] Preserve strict backend argument validation and generated schema alignment.
- [ ] Verify concurrent workspace calls and per-request overrides; no global switch.
- [ ] Qualify workspace-bound documents and recall; disable unsupported recall
  rather than use an unrelated namespace.
- [ ] Add scoped resources, notifications, subscriptions, and custom mutation
  handling. Until supported, keep these surfaces unadvertised and rejected.
- [ ] Implement shared HTTP serving with existing auth, host/origin, TLS, session
  ownership, and workspace visibility constraints; no new unauthenticated access.
- [ ] Preserve existing bound stdio mode and document its distinction from the router.

**Exit:** each exposed transport operates only on the intended graph/context.
A local read-only router does not imply shared authenticated HTTP support.

## S5. Automatic client experience

- [ ] Qualify capability-checked legacy/modern root negotiation, deadlines,
  continuation integrity, unknown/ambiguous roots, and changing roots.
- [ ] Map multiple roots within an established workspace to that workspace while
  keeping unrelated workspaces ambiguous; names must not influence ownership.
- [ ] Test clients without roots, HOME launch, explicit path overrides, reconnects,
  and real supported clients with recorded versions/capabilities.
- [ ] Keep root/path hints within authorized registrations, not arbitrary filesystem
  indexing permissions. Never silently fall back after invalid supported roots.
- [ ] Add first-use indexing after handshake only with authorization, progress,
  cancellation, resource/inference limits, and no implicit force rebuild.
- [ ] Keep recovery commands actionable and metadata diagnostics honest.

**Exit:** developers can use multiple independent Codex/Claude/IDE workspaces with
one reusable MCP setup. Manual registration/member lists remain optional overrides.

## S6. Release qualification

- [ ] Execute synthetic isolation matrices across code, docs, recall, graph calls,
  resources, watches, rebuilds, removal, and colliding identities.
- [ ] Run registry/writer/process tests on supported platforms, including worktrees,
  moved roots, duplicate basenames, corruption, and recovery.
- [ ] Measure metadata-only startup with many registered but unloaded workspaces,
  cold-load queues, peak memory, and warm routing versus the direct-server baseline.
- [ ] Run focused tests and repository quick/full gates with no real credentials
  or paid inference; record exact commit/run evidence for completed gates.
- [ ] Document migrations, rollback/old-binary limits, remaining unsupported cases,
  watch semantics, remote visibility, and resource/inference behavior.
- [ ] Keep examples synthetic and describe only implemented behavior as available.
- [ ] Obtain review before merge; do not deploy or replace installed binaries implicitly.

**Exit:** the declared release scope is reproducible and every claimed boundary has
test evidence. Leave incomplete gates unchecked, even when some pieces exist.

## Deferred work

Cross-workspace federated search/groups, inferred cross-workspace dependencies,
shared embedding runtimes, an implicit daemon, and in-process multi-index runtimes
are not prerequisites for basic independent graphs. Revisit them only after the
current experience is correct and measured.
