# Isolated workspaces for coding-agent projects

**Status:** Architecture and acceptance contract for PR #34. Automatic local
setup, registry administration, and local read-only MCP routing are implemented
on the review branch. The entire contract is not yet implemented or qualified.
See the [workspace guide](../workspaces.md), [MCP guide](../workspace-mcp.md), and
[delivery checklist](multi-workspace-implementation.md) for the distinction.

## 1. Purpose

Codanna must serve multiple independent coding-agent projects without mixing
their code graphs, search results, configuration, documents, or conversation
context. The feature is generic: any Codex, Claude Code, IDE, or other MCP client
can work with any supported local codebase. No customer's projects, repository
names, directory layout, or business/product structure are part of the contract.

A **workspace** is an independently indexed source and knowledge boundary.
Normally it is the project or checkout the developer opens. It can contain one
repository, a monorepo, or several repositories intentionally indexed together.
A parent workspace is optional, not a prerequisite for using separate projects.

```text
MCP connection
  -> workspace ID -> independent graph/index and runtime
  -> workspace ID -> independent graph/index and runtime
  -> ...
```

Aliases and locations are discovered or supplied at runtime. All example labels
in documentation and tests are synthetic placeholders, not built-in workspaces.
Never infer shared ownership from matching names, remotes, or sibling locations.

## 2. Required experience

In each intended workspace, `codanna index` discovers the source boundary and
performs initial configuration/registration automatically. Standard layouts must
not require manual IDs, a repository inventory, or an MCP entry per project.
Existing source lists, exclusions, aliases, and local data remain authoritative.

One reusable local MCP configuration uses `codanna workspace serve`. Supported
client roots select the workspace on an unscoped request. Explicit workspace IDs,
aliases, and registered project paths are overrides, not normal setup requirements.
A client without roots can use its launch directory. An ambiguous or unknown scope
must produce guidance rather than query a different graph.

Initial indexing is currently explicit. Future automatic first indexing must run
only after protocol initialization, with authorized scope, visible progress,
resource/inference budgets, cancellation, and no silent destructive rebuild.

## 3. Invariants

1. Each workspace has an independent index/graph and mutation boundary.
2. A request resolves its workspace once and keeps it immutable until completion.
3. No process-global mutable current workspace exists.
4. Every routed result identifies its owning workspace; exact references retain it.
5. Matching symbol, file, or collection names never join independent graphs.
6. Discovery is not indexing, and client roots are not filesystem authorization.
7. One corrupt index, unavailable directory, or worker failure must not disable
   unrelated workspaces.
8. Configuration, code, documents, recall, resolver caches, and watch state use the
   same workspace context when those capabilities are supported.
9. Existing single-workspace commands remain supported.
10. No real user projects or personal filesystem paths belong in shipped examples,
    instructions, fixtures, defaults, or acceptance requirements.

## 4. Registry and identity

Evolve the existing `~/.codanna/projects.json` registry, not a competing database.
The current implementation retains its v1 schema and existing IDs. The registry
holds identity, aliases, canonical locations, and routing metadata. Workspace
settings remain in the workspace configuration; do not duplicate authoritative
source/provider/index settings in the registry.

Keep stable workspace identity distinct from a mutable alias and checkout path.
Canonicalize symlink-equivalent roots. A move preserves identity only through a
validated relocation; a second live clone or Git worktree must not silently share
mutable index storage. Equal basenames are valid at different locations and need
unambiguous generated aliases or ID selection.

Registry updates require an OS-backed lock around the complete read-modify-write
transaction, atomic replacement, appropriate synchronization, and stale-snapshot
protection. Corrupt input and unsupported versions must be preserved, not replaced
with an empty registry. Migration must be recoverable and document old-binary
compatibility. Older binaries do not participate in new locking protocols.

Registration alone must not rebuild an index. Unregistration removes routing
metadata, not source/configuration/index files. The current router rejects new
calls after unregistration; in-flight work and independent servers are not revoked.

## 5. Discovery and source boundaries

Resolve from an explicit start path without changing the router's process cwd.
Use valid workspace configuration and configured-source ownership, then the
nearest checkout or recognized project manifest. A developer can establish a
multi-root workspace by indexing its intended containing directory once.

An established parent may own configured descendants, including child repositories
with standalone settings. Discovery must not overwrite or import those child
settings/indexes. An unconfigured checkout must not scan its siblings and invent
a parent workspace. Generic HOME/filesystem roots are not automatic choices.

A model-cache-only `.codanna` directory is not a workspace. A malformed applicable
configuration must return an error rather than silently select another boundary.
Discovery is bounded, skips dependency/cache directories and directory symlinks,
and reports truncation. Diagnostic repository inventory is not an index plan or
proof of persisted repository ownership.

Fresh source defaults may be created only for absent/empty storage. Never widen
an existing populated index merely because its configured source list is empty.
Explicit external source roots require supported, validated membership; path
traversal or a copied configuration cannot redirect one workspace to another index.

## 6. Optional repositories within a workspace

The primary feature is independent workspace graphs. A workspace can also contain
multiple repositories without becoming a special product-specific concept.
Repository membership should be derived under its established source boundary,
respecting exclusions. Manual membership is an override for unusual layouts.

Preserve repository provenance for code, documents, and recall as this capability
is completed. Identical relative paths in separate members must remain distinct.
Resolver candidates must not connect members based solely on matching names.
Cross-repository edges within a workspace require verifiable dependency evidence;
unresolved relationships remain unresolved. Cross-workspace graph traversal is
not implied by either membership or shared MCP serving.

Do not maintain independently editable repository and `indexed_paths` lists that
silently drift. Preview any migration and specify which representation is
authoritative. Attaching a source is not permission to merge existing child indexes.
Removing membership must make detached data non-queryable through that scope;
cleanup needs explicit, safe publication semantics.

Repository IDs and generation-qualified references are future persisted/API work.
If row semantics change, use existing format/emission compatibility gates and
require an explicit migration or rebuild. Do not mix old and new row semantics
or promise rebuild-free storage changes.

## 7. Runtime architecture

Keep the existing single-workspace engine behind a router. Start with lazy,
isolated workers, one per active workspace, with fixed configuration and cwd.
Do not mutate the parent's environment or cwd to service a request. Resolver and
recall code with process-global assumptions must remain isolated until explicit
context injection replaces those assumptions.

The local implementation exposes `codanna workspace serve`. Each stdio connection
has its own worker pool; sharing a registry does not create a machine-wide daemon.
Ordinary `codanna serve` stays workspace-bound and retains its supported behavior.

A future shared HTTP service must reuse existing authentication, authorization,
host/origin, session-ownership, and TLS controls. Do not expose registry-wide
filesystem access through a new unauthenticated endpoint. Enumerate only visible
workspaces and avoid private host paths in remote responses/errors.

Bound worker/admission counts, cold loads, query queues, indexing, and memory.
Coalesce concurrent loads and pin active/queued requests before eviction. Use
per-workspace synchronization rather than serializing independent graphs behind
one giant lock. Cancel or close timed-out workers, back off failures, and never
blindly replay mutations after an uncertain disconnect.

Shared model files, model runtime memory, and workspace vectors are different
resources. Keep the existing download cache; do not introduce a shared embedding
service or new inference spending implicitly.

## 8. MCP scope and result contracts

Resolve explicit workspace or `project_path` first, within the authorized scope.
In the local router, supported client roots take precedence over the launch
directory; the launch directory is the fallback only without roots support.
An explicit override affects its request only. Multiple roots are unambiguous
only when all map to the same registered workspace.

Check client capabilities and negotiated protocol before requesting roots. The
implementation supports legacy roots RPC and modern MRTR roots input. Bind
continuations to the original request with random, expiring, single-use handles.
Unknown, remote, malformed, changed, or excessive roots must not cause fallback
to an unrelated graph. Root changes affect future requests, not in-flight scope.

Tools accept `workspace` or `project_path`, mutually exclusive. Reuse generated
backend schemas and preserve strict argument validation. `list_workspaces` and
`get_workspace` must not load every index or model. Returned text and structured
content identify the owning workspace. Agents must retain that ID on local
symbol-ID follow-ups and look up IDs again after a full rebuild.

Eventually exact references should include workspace, repository where applicable,
local ID, and index generation. Mismatched references are errors, not permission
to fall back to a name search. Namespace cache keys, resources, subscriptions,
continuations, telemetry, and job receipts, not only successful tool responses.

The current local router advertises read-only tools only. Until custom mutations,
resources, and subscriptions can be scoped correctly, reject them instead of
forwarding them to a default worker. Shared read-only access must not silently
expand a pre-existing connection's scope.

## 9. Documents and conversation recall

`search_context` must bind code, documents, and recall through one context.
Workspace-specific recall must never come from an unrelated inherited global
selector. Current selected/automatic/router launches disable inherited recall
until a reliable binding exists; explicit `--config` retains legacy behavior.

Map imported conversation checkout paths to their owning workspace without
relabeling unrelated history. Shared physical recall storage is acceptable only
with mandatory namespace filters and provenance. Workspace-wide conversations
or documents do not need a fabricated repository owner. Historical content is
evidence, not active instructions or current policy.

Collection aliases are local to their workspace and, when needed, repository.
A narrowed request must not silently search another workspace/member as fallback.
Keep source-specific language settings and ignore rules explicit; do not layer
nested standalone configurations without a supported migration policy.

## 10. Writers, watching, and rebuilds

A complete implementation needs one writer/watcher authority per storage set
across processes, including CLI writers and independently launched MCP clients.
An in-memory router lock or registry lock alone does not provide that guarantee.
Use storage-safe OS coordination; forward to the owner or return a scoped busy
error. Never delete a live writer's files to take ownership.

Distinguish persistent watch leases from query-only idle workers. Eviction must
not silently stop a promised continuous watch. Reindex jobs need status, bounded
execution, cancellation semantics, and unambiguous completion/failure receipts.

Validate the whole selected source set before destructive work. Missing roots,
partial traversal, or authorization errors must not erase valid data and report
success. Keep prior generations recoverable until safe publication. Partial
rebuilds must not accidentally clear untouched members or other workspaces.

## 11. Health, diagnostics, and future search

Report configured, loading, available, busy, missing-root, rebuild-required, and
failed states honestly. Metadata/directory presence is not proof of a complete
index or successful embeddings. Include workspace identity in logs and recovery
commands, and measure warm routing separately from cold/model startup.

Normal queries target one workspace. Optional federated search is later work:
authorize every target, bound fan-out and inference, retain provenance, and report
partial failures. Independent rankings are not necessarily score-comparable;
start with grouped results or an evaluated deterministic fusion policy. Searching
multiple graphs must never implicitly merge their topology.

## 12. Acceptance and delivery

Use synthetic temporary workspaces only. Test unrelated single-repository graphs,
a workspace with multiple members, duplicate directory basenames, moved paths,
clones/worktrees, and nested folders. Vary arbitrary aliases and locations so no
particular name or repository structure is a prerequisite.

Deliberately collide symbol names, relative paths, collection aliases, and local
IDs. Verify exact ownership, scoped queries/graph traversal, stale-reference
rejection where supported, and consistent code/doc/recall filtering. Run concurrent
clients against different graphs and verify per-request overrides do not alter
each other's defaults. Rebuilding/removing one workspace must preserve the other.

Cover configuration and registry corruption, missing roots, symlink escapes,
writer contention, worker failures, cancellation, admission/eviction, root changes,
unknown roots, protocol variants, and shutdown. Use deterministic local data,
cleared credentials, and mocked/disabled inference; never index a user's real
projects or spend paid inference during automated validation.

The [delivery checklist](multi-workspace-implementation.md) tracks remaining
qualification. Keep the PR draft until its intended merge scope is reviewed.
Do not treat a routing test as proof of complete repository graph semantics,
shared-daemon behavior, storage locking, recall isolation, or all-platform support.
