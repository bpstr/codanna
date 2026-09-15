# Isolated workspaces for coding-agent projects

**Status:** Architecture and acceptance contract for PR #34. Local automatic
setup, code-only freshness, scoped imported recall, and independent routing are
implemented on the review branch. Qualification is tracked separately in the
[delivery checklist](multi-workspace-implementation.md). See the
[workspace guide](../workspaces.md) and [MCP guide](../workspace-mcp.md).

## Purpose

One project-agnostic MCP definition must serve arbitrary independently opened
coding projects. Client roots or launch cwd select the project. First use prepares
its private settings, registration, and code index without manual project names,
IDs, repository inventories, or initial indexing commands. Setup status/errors
never substitute another project's graph.

A workspace is an independent graph/index and knowledge boundary. It may be one
checkout, a monorepo, or an intentionally opened combined root. No customer's
names, business hierarchy, or filesystem topology is part of this feature.
Aliases and locations are runtime data; shipped examples must be synthetic.

## Invariants

1. Resolve each request once and keep its workspace immutable until completion.
2. Never mutate process-global cwd/environment to route concurrent requests.
3. Independent indexes, equal names, and equal local IDs do not merge identity.
4. Parent coverage, sibling locations, or equal remotes do not establish ownership.
5. Unavailable, ambiguous, and initializing scopes never fall back to other graphs.
6. Bootstrap is local, bounded, code-only, and non-destructive; no implicit paid
   inference or forced replacement of existing data.
7. Code, documents, recall, resolver state, and watching use one resolved scope.
8. Preserve explicit settings, source exclusions, existing registry IDs, and data.
9. Successful routing is not proof of complete indexing or measured performance.

## Discovery and identity

Client roots are routing hints within the local session's authority, not arbitrary
remote filesystem access. Exact opened configurations are validated. Source
subdirectories may resolve to the nearest applicable checkout/manifest. A bare
ancestor configuration cannot widen an independently opened plain project.
Intentional combined indexing remains possible by opening/selecting that root;
it does not automatically adopt independent children.

Only diagnostic/indexing operations inventory descendants. Query resolution uses
bounded ancestor checks. A cache-only `.codanna` directory is not a workspace.
Symlink-equivalent roots canonicalize; separate worktrees/clones retain independent
storage. HOME, filesystem roots, and broad system folders are not implicit projects.

Reuse the existing v1 `projects.json` registry and IDs. Aliases are mutable, not
primary identity. Lock the complete read-modify-write transaction, replace
atomically, reject stale snapshots, and preserve corrupt/unsupported state.
Registration is not rebuilding; unregister never deletes source/index data.
Validated relocation preserves registry identity, not automatic adoption of a copy.

## Bootstrap and continuous freshness

Handshake/tool enumeration stay lightweight. First knowledge use prepares a
validated local root and schedules private staged indexing. Validate all roots
before writes. Unsupported/empty roots remain retryable without launching empty
indexers. Use the existing language registry and source exclusions, count bounded
entries/files/bytes, and propagate incomplete discovery.

Disable embeddings for bootstrap, preserve project settings, check file-count
overflow and the original configuration before publication, and never clear
nonempty live data on refusal. Preserve cancellation and ownership until actual
writes finish. These safeguards are not a filesystem-wide transaction.

Code-only workspaces continue with native watching and bounded offline catch-up.
Install watches between catch-up passes; ordinary queries do not repeat discovery.
One OS-backed lease elects the code writer; other clients follow committed changes
and may take over after exit. Root settings/ignore changes require revalidation.
Explicitly disabled watching and existing semantic data/provider settings retain
explicit maintenance modes rather than silently spending inference.

The lease resides outside replaceable index contents. Updated CLI index, MCP
reindex, bootstrap, and watcher paths use it and retain it through blocking work.
A conflicting explicit writer fails before destructive mutation. Coordination is
advisory among updated participants, not enforcement against old binaries or direct
third-party storage access.

## Runtime and Rust rules

Reuse isolated readers with fixed cwd/configuration and explicit lifecycle states.
Each stdio frontend has its own bounded pool, not a machine-wide shared daemon.
Coalesce initialization; cancelling one waiter must not restart shared work.
Use short state locks, bounded concurrent RPCs, typed errors, and request-specific
cancellation. Invalid parameters are not evidence that a worker crashed.

Admission includes discovery and diagnostics. Blocking closures own permits until
actual completion, even when an await times out. Check cancellation during bounded
traversal without pretending kernel filesystem calls can be forcibly interrupted.

Cache metadata snapshots and coalesce refresh instead of hashing full metadata or
inventorying sources per query. Scope/registry validation still performs I/O.
Measure routing, cold loading, and model initialization independently.

Lite facade loading keeps lexical/statistics queries model-free. Load semantic
and document facilities on demand with shared initialization and retained errors.
Document revision checks invalidate both stale stores and cached absence without
walking source files. Auxiliary failures must not poison unrelated code lookup.
Physical child permits last until observed exit. Eviction pins active work;
shutdown cancels owned jobs, closes transports, and awaits writes/child cleanup.

## MCP and knowledge contracts

An explicit workspace ID/alias or registered project path overrides one request,
not the session default. Capability-checked client roots precede launch cwd.
A HOME-launched process without roots must return a scope error; it cannot infer
another process's changing cwd. Cache roots only with promised change notifications.
Use generation-bound, expiring, single-use continuations for negotiated root input.
Root changes affect future requests, never in-flight ownership.

Reuse backend schemas and strict validation. Enumerating workspaces must not load
all indexes/models. Reflect bootstrap/cache writes honestly in tool annotations.
Every routed result carries workspace provenance. Keep that ID on symbol-ID
follow-ups; raw IDs are not globally unique and must be refreshed after rebuilding.

Imported recall uses a domain-separated hash of the canonical project directory.
The importer derives the same namespace without manual labels. Ignore inherited
legacy labels in the local router, validate reply scope before rendering, and bound
subprocess output/time. Import selects one transcript explicitly; never scan private
history as a side effect of discovering a project. Equal basenames remain distinct;
moves do not silently relabel conversations. Legacy explicit namespaces are opt-in.

Validate document storage, configured roots, persisted sources, and each returned
hit against the same workspace. Refuse foreign or corrupt state before model use.
Do not create replacement empty stores on failed reads. Published metadata must
invalidate lazy caches. This does not imply automatic document ingestion.

## Existing network session lifecycle

Project-bound HTTP/HTTPS retains authentication, ACLs, session ownership, origin,
host, TLS, and replay checks. Keep session-map guards short and cancellation out of
the ordinary traffic queue. DELETE/expiry drops full or unpolled SSE receivers,
releases permits, and closes session transports without depending on client polls.
Stream capacity is per-session. This repair does not introduce registry-wide
network access or a multi-workspace HTTP endpoint.

## Remaining extensions and qualification

Generation-qualified exact references, optional repository-partitioned graphs
within deliberately combined roots, explicit external source adoption, scoped
subscriptions/resources/mutations, shared authenticated multi-workspace serving,
and intentional federated search remain separate extensions. None should redefine
the basic independent-project onboarding requirement.

Start acceptance tests with two plain fresh directories, identical symbols,
different content, and no prior configuration or indexing. Test automatic roots
and root changes on one connection, concurrent independent sessions, broad parents,
empty projects, errors, aliases, clones, and symlinks. Also test native edit/create/
delete updates, offline catch-up, follower commits and handoff, writer contention,
recall isolation, document boundary/absence-cache refresh, concurrent cancellation,
physical cleanup, and session deletion with saturated/unpolled streams.

Use synthetic data, cleared credentials, disabled/mock inference, and deadlines.
Run focused tests, full repository gates, and platform witnesses; report exact
commits/results rather than inferring success from test source. Performance claims
require measurements. Keep unfinished qualification visible and the PR draft.
This contract never authorizes an implicit merge, deployment, or installation.
