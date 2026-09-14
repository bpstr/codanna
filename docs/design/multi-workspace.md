# Product-level multi-workspace support

**Status:** Proposed; design review only. No workspace commands, router, or
storage changes are implemented by this document.

**Source baseline:** `f776840bf9a2a6241c0b62975ea2bf4e44e93e0d`.

**Implementation tracking:** [Delivery checklist](multi-workspace-implementation.md).

## 1. Decision and motivating example

A **workspace is an independent product or knowledge boundary containing one
or more repositories**. A repository is a source and provenance boundary inside
that workspace. It is not automatically a workspace of its own.

```text
Codanna installation
|-- workspace: assign
|   |-- repository: assign-core
|   |-- repository: assign-web
|   |-- repository: assign-mobile
|   |-- repository: openapi-spec
|   |-- repository: typescript-sdk
|   `-- repository: php-sdk
|-- workspace: codanna
|   `-- repository: codanna
`-- workspace: customer-product
    |-- repository: backend
    `-- repository: frontend
```

A query for `task status` in Assign searches its configured code, documents, and
conversation recall across the relevant member repositories. It does not search
the unrelated Codanna workspace. Repository filtering narrows that workspace;
it never changes its ownership or authorization boundary.

This decision supersedes the earlier one-workspace-per-repository proposal.
Workspace groups are **not required** to represent Assign and are deferred.
There is no requirement to move folders, use Git submodules, or create a parent
Git repository. Member repositories can be sibling directories or explicitly
configured external roots.

## 2. Goals and exclusions

Deliver independent workspaces through one installation and an optional shared
MCP service. Preserve existing single-workspace commands, permit product-wide
search within a workspace, and make workspace selection explicit and safe under
concurrent use. Each workspace owns its configuration, index lifecycle,
documents, recall binding, resolver context, health, and watch ownership.

Do not merge unrelated workspaces into one index, infer graph edges from equal
names, index arbitrary client paths, or introduce a process-global current
workspace. Do not require new parser implementations, a database replacement,
a vector service, or an in-process multi-index rewrite for the first release.
Cross-workspace discovery, workspace groups, and explicit cross-product graph
relationships are later features, not prerequisites.

## 3. Current implementation and integration points

At the source baseline:

| Area | Current behavior | Required change |
| --- | --- | --- |
| [Settings](../../src/config/mod.rs) | One workspace root and index path; discovery starts from cwd | Explicit, validated workspace context and safe discovery |
| [Indexed paths](../../src/config/paths.rs) | Several directories feed one configured index | Explicit repository membership and provenance, not automatic workspaces |
| [Initialization](../../src/init.rs) | Existing project IDs and `projects.json` registry | Evolve this registry rather than add a competing source of truth |
| [CLI](../../src/cli/args.rs) and [startup](../../src/main.rs) | Load one configuration and facade | Resolve workspace before loading models, providers, or storage |
| [MCP server](../../src/mcp/server.rs) | One facade and optional document store | Route to a pinned workspace runtime |
| [Requests](../../src/mcp/requests.rs) | No workspace selector; unknown fields rejected | Add validated scope consistently to applicable tools |
| [Context](../../src/mcp/tools/context.rs) | Recall workspace selected through process environment | Bind recall to the same resolved workspace as code and docs |
| [Resolver helpers](../../src/project_resolver/helpers.rs) | Some cache lookup paths are cwd-relative | Isolate workers first; inject explicit context before sharing a process |
| [HTTP](../../src/mcp/http_server.rs) and [serve](../../src/cli/commands/serve.rs) | Shared server state is still single-workspace | One service manager with independent workspace runtimes |

The current global data directory and project-local directory both use the name
`.codanna`. Discovery must not recognize a model-cache-only directory as a
workspace. A malformed nearest project configuration must produce an error,
not silently fall through into an unrelated ancestor workspace.

Adding multiple `indexed_paths` is not sufficient: it provides neither request
routing between independent products nor reliable repository ownership within
a product. A current `--force` operation clears the selected index, so workspace
selection must be resolved and validated before entering that path.

## 4. Ownership and identity

Use distinct types for `WorkspaceId`, `RepositoryId`, and index generation.
Aliases are human-readable, unique within their scope, and mutable. They are
not primary keys. Names such as `backend` may occur in different workspaces.

Persist machine-local workspace identity independently of its current absolute
path. A local identity marker, if used, belongs in ignored Codanna state, not
tracked configuration. Preserve a known ID on an explicit move. A copied marker
at a second live checkout must not silently claim the original workspace;
require explicit adoption or allocate a new identity.

Separate three concepts:

- Workspace identity: product and knowledge ownership.
- Repository identity: configured member source within that workspace.
- Checkout/index generation: the actual indexed files and revision state.

Git worktrees and separate clones must not share mutable index files merely
because their remote URL or repository name is equal. A checkout can have one
authoritative workspace owner in the first release. Reject attaching the same
canonical checkout to two workspaces; do not implement writable shared membership.

Externally, a symbol reference carries at least workspace ID, repository ID,
local symbol ID, and index generation. Generation prevents a reference obtained
before a full rebuild from silently resolving to another symbol afterward.
Document chunks, recall hits, resource URIs, and notification payloads need
similarly unambiguous provenance.

## 5. Configuration and registry

Keep `~/.codanna/projects.json` as the registry location for the initial
migration. Evolve its schema into workspace routing metadata; do not maintain
both an authoritative projects registry and an authoritative workspaces registry.

The registry maps stable workspace IDs and aliases to canonical roots and exact
configuration paths. It must not duplicate repository lists, indexing settings,
provider secrets, or another authoritative index-path setting.

Repository membership belongs to the workspace configuration. Proposed syntax
for the existing `.codanna/settings.toml` follows. **These fields are not yet
implemented and this fragment is not an installation instruction.**

```toml
[workspace]
name = "assign"

[[workspace.repositories]]
name = "assign-core"
path = "assign-core"

[[workspace.repositories]]
name = "assign-web"
path = "assign-web"

[[workspace.repositories]]
name = "assign-mobile"
path = "assign-mobile"

[[workspace.repositories]]
name = "openapi-spec"
path = "openapi-spec"

[[workspace.repositories]]
name = "typescript-sdk"
path = "typescript-sdk"

[[workspace.repositories]]
name = "php-sdk"
path = "php-sdk"
```

Resolve relative paths against the owning workspace root, never the router cwd.
A member outside that root must be an explicit allowlisted source, not an
accidental path traversal. Render its results as repository-relative paths.
Reject duplicate aliases, symlink-equivalent roots, and overlapping source
ownership unless there is an explicit, tested partition rule.

The workspace repository list becomes the membership authority when enabled.
Legacy `indexed_paths` remains supported for existing workspaces; do not keep
two independently editable lists that silently drift. An explicit migration
must show how existing roots map to repositories before changing configuration.
Do not enumerate an entire parent directory and assume all descendants belong
to Assign.

Nested repository `.codanna` directories may contain existing standalone
indexes. Attaching that repository is not permission to overwrite, import, or
merge those indexes. The first implementation indexes the explicitly selected
sources into workspace-owned storage after user-requested indexing. Reusing
existing child index artifacts is a separate, compatibility-checked operation.
Nested configurations are not silently layered over workspace settings.

## 6. Registry durability, migration, and lifecycle

Hold an OS-backed registry lock across reload, validation, mutation, temporary
write, synchronization, and atomic replacement. Atomic replacement alone does
not prevent concurrent lost updates. Use cross-platform locking and persistence
helpers already available where suitable; define crash recovery and parent
metadata synchronization where supported. Read-only discovery must not create
a registry, lock file, index, or model cache unnecessarily.

Migrate the existing registry under that same lock. Preserve usable IDs and
paths, retain recoverable original data, reject unknown versions, and refuse
corrupt input without replacing it with an empty registry. Older binaries must
not be expected to understand a migrated schema. Report that compatibility
boundary before a write.

Registration is metadata-only and idempotent for the same canonical root.
Registering from `assign-web/src` resolves existing Assign membership instead
of creating a new workspace for `src`. Distinct legacy repository registrations
are not automatically combined because their names or parent directory match.
Combining them into Assign is an explicit attach/adopt operation.

Unregistering removes routing metadata and stops the corresponding runtime;
it leaves sources and project-local indexes untouched. Moving a workspace is
explicit, preserves identity, invalidates stale absolute-path caches, and
validates the new location. Missing roots remain visible with actionable health
status, not silently forgotten.

Registration alone does not require an index rebuild. Adding repository
provenance to persisted rows may require a versioned migration or rebuild;
use Codanna's existing compatibility gates and state that requirement honestly.
Do not promise both a changed storage schema and universally rebuild-free use.

## 7. Proposed CLI contract

All commands in this section are **proposed**, not currently available.

```text
codanna workspace add /projects/assign --name assign
codanna workspace add /projects/codanna --name codanna
codanna workspace list --json
codanna workspace show assign --json
codanna workspace doctor assign --json
codanna workspace rename assign assign-product
codanna workspace move assign-product /development/assign
codanna workspace remove assign-product

codanna workspace repo add assign /projects/assign/assign-web --name assign-web
codanna workspace repo list assign --json
codanna workspace repo remove assign assign-web

codanna --workspace assign index
codanna --workspace codanna retrieve search "IndexFacade"
```

Provide the equivalent configuration-file workflow for repositories; the CLI
edits the same authoritative configuration rather than a hidden parallel list.
Member removal removes routing membership first; stale indexed rows must not
remain queryable as though still attached. Purging those rows is explicit and
workspace-scoped, with a documented state transition.

Preserve `codanna index` and `codanna serve` in existing projects. Explicit
`--workspace` and `--config` must not contradict: reject conflicting selectors
before touching providers or storage. Without explicit selection, local CLI
usage can discover its valid workspace from cwd. Router mode must fail closed
when selection is absent or ambiguous, never fall back to a global mixed index.

A `project_path` convenience selector, if added to MCP, resolves only an already
registered workspace or member repository. It cannot open arbitrary files or
create a registration. Name the primary parameter `workspace` consistently.

## 8. Runtime and transport architecture

```text
Shared HTTP MCP service
  session scope + authorized workspace set
                    |
             WorkspaceRouter
                    |
       +------------+------------+
       |                         |
  Assign worker             Codanna worker
  one workspace context     one workspace context
  member repositories       member repository
  code/docs/recall           code/docs/recall
  resolver/watch state      resolver/watch state
```

Start with one lazy worker per active workspace, not one workspace or worker per
repository. Reuse the existing engine and MCP request implementation behind a
small internal adapter. Each worker gets an absolute configuration path, a
fixed cwd, workspace-specific recall binding, and a sanitized environment.
The parent must never change its cwd or process environment per request.

One HTTP service can genuinely share workers across several clients. A stdio
server is normally started separately by each client; it cannot share a worker
pool across processes just by using the same registry. Preserve efficient
single-workspace stdio. Proposed multi-workspace stdio may use a local router
per connection, or a future explicit bridge to the shared service. Do not
advertise a shared daemon for a plain per-client stdio command.

Proposed shared entry point: `codanna serve --http --workspaces`. The exact
flag and transport compatibility require tests before release. Reuse existing
network authentication and host/origin restrictions; do not bypass them with
a new unauthenticated routing endpoint.

A later in-process implementation must inject `WorkspaceContext` through code,
docs, recall, resolver caches, watchers, and persistence first. It is not a
prerequisite for the first working router.

## 9. Request scope and session behavior

Resolve a request once into an immutable workspace context before loading any
knowledge store. Use this order within the connection's authorized scope:

1. Explicit workspace ID or alias, or an unambiguous registered path selector.
2. A server-configured/session-pinned default workspace.
3. Exactly one workspace resolved from supported client roots.
4. An actionable selection-required error.

A repository selector without a workspace resolves only inside an already
selected workspace; a generic alias such as `backend` is never global. Snapshot
the chosen workspace, member set, and relevant index generation for the request.
Do not switch them mid-query when client roots or registry state changes.

Use client roots only after checking capability support. Roots are discovery
hints, not authorization. Match canonical member roots back to their product
workspace, so a client in `assign-web` selects Assign rather than an invented
`assign-web` workspace. Multiple roots all belonging to Assign remain
unambiguous. Roots spanning Assign and Codanna require explicit selection.
Timeouts, unsupported capabilities, and unknown roots leave explicit selection
available. Root-change notifications affect future requests only.

There is no process-global `current_workspace` and no global switch tool.

## 10. MCP tools and output contract

Add `list_workspaces` and workspace details without loading models or indexes
merely to enumerate registrations. Return only workspaces visible to that
connection. Unknown and unauthorized selectors must not disclose hidden names
or filesystem paths.

Add consistent workspace scope to symbol search, semantic search, documents,
`search_context`, graph tools, index information, resource reads, and existing
custom reindex/statistics requests. Repository filtering is optional for search
and validation of exact references. Keep existing strict unknown-field checks.

Proposed request:

```json
{
  "workspace": "assign",
  "query": "task status",
  "repository": "assign-web"
}
```

Omitting `repository` searches all Assign members. Omitting `workspace` is safe
only when the rules above resolve one workspace. Do not overload an array of
repositories to mean an array of unrelated workspaces.

Proposed result identity:

```json
{
  "workspace": {"id": "ws_assign_example", "name": "assign"},
  "repository": {"id": "repo_web_example", "name": "assign-web"},
  "index_generation": "generation-example",
  "symbol_id": 1742,
  "path": "src/features/tasks/TaskCard.tsx",
  "line": 42
}
```

The IDs above are illustrative, not the final serialization grammar. Preserve
local symbol IDs internally; namespace references at the public boundary. A
qualified reference that conflicts with the selected workspace or generation
is an error, never a fallback name search. Legacy unqualified IDs remain
supported only against an unambiguous single workspace/index context, without
claiming protection against rebuild reuse.

Prefer structured content plus compatible readable text. Scope error payloads,
continuation tokens, cache keys, telemetry, and notifications as carefully as
successful results. Use repository-relative paths by default for remote output;
absolute path exposure is an explicit local capability.

## 11. Index storage and graph boundaries

Retain an independent workspace-owned index boundary. The initial strategy can
extend the existing multi-root index with repository IDs and repository-scoped
resolver state. A workspace may therefore index all Assign repositories while
Codanna uses another index. Internal per-repository shards are a future storage
choice, not a reason to expose six Assign workspaces to the user.

Before allowing product-wide results, prove that identical relative paths,
package names, module aliases, and symbol names in two repositories remain
distinct. Source identity must include repository ownership; never strip all
member roots and store several `src/config.ts` files under the same key.

Default graph resolution is repository-local. A cross-repository edge inside
Assign requires explicit, verifiable dependency evidence supported by the
resolver, such as a real workspace-package dependency. Record evidence type and
provenance; unresolved relationships remain unresolved. Equal names or embedding
similarity are not dependency evidence. Unrelated workspace graphs never join.

Do not claim that the router alone provides this protection: repository-aware
storage and resolution tests are a release prerequisite for multi-repository
workspaces, even when process isolation already protects different workspaces.

## 12. Code, documents, recall, and source configuration

`search_context` chooses one context and passes it to all three sources. Recall
must not select a different workspace through inherited global environment.
Workers may use a scoped environment as a transitional adapter; the public
contract is an explicit `RecallBinding` on the workspace context.

Map Codex/Claude conversation checkout paths to registered member repositories
and their owning workspace during explicit recall configuration/import. Moving
folders needs an explicit mapping update; it must not relabel unrelated history.
A physical shared recall database is acceptable only with mandatory namespace
filters and provenance. Workspace-wide conversations need not pretend to belong
to a specific repository.

Document collection identity is workspace-qualified and, for repository-owned
collections, repository-qualified. Workspace-owned documents are a separate
valid scope. When several member repositories contain a collection named
`docs`, require a repository or return explicitly labelled collections; do not
arbitrarily choose one. Repository-filtered context search does not include
other repositories' docs or recall as a hidden fallback.

Each member retains the appropriate language-project configuration, ignore
rules, and source-root mapping. Existing standalone Codanna configurations are
not automatically imported; provide an explicit preview/migration for desired
settings. Historical conversation text is evidence, never active instructions.

## 13. Concurrency, watching, rebuilds, and resources

Guarantee one writer/watcher authority per workspace storage set across all
processes, including manual CLI indexing and separate stdio clients. An
in-memory router lock is insufficient. Use compatible OS-backed storage locks;
new indexing requests either reach the owner or fail with a scoped busy error.
Never delete a live writer's files to acquire ownership.

Load each workspace with single-flight initialization. Requests pin runtimes
until completion; idle eviction cannot interrupt an in-flight request or index
job. Queries for Codanna must not wait on one giant lock while Assign loads.
Bound worker count, concurrent cold loads, query fan-out, and indexing jobs.
Watchers are explicit persistent resources: idle eviction must not silently
stop a promised continuous watch. Distinguish active-watch and query-only leases.

A normal query never triggers indexing or a force rebuild. Validate the entire
selected rebuild source set before destructive operations. Missing members,
authorization failures, or partial traversal must not silently produce an
apparently complete rebuilt workspace. Use the shared index-planning work where
available; preserve the previous generation until safe publication. Reject or
explicitly scope repository-only force rebuilds so they cannot erase untouched
members by accident.

Separate shared model downloads from runtime memory and workspace embeddings.
Reuse the existing model cache. Bounded worker/model memory is required;
introducing a shared embedding service is optional and must not be silently
enabled. Automated validation must not use paid inference or real credentials.

## 14. Failures and observability

Report registered, loading, ready, missing-root, busy, rebuild-required, and
failed states per workspace. Startup reads registry metadata, not every index.
A bad index or worker crash affects only that workspace. Use restart backoff;
never retry mutations blindly after an uncertain worker disconnect.

Include workspace identity in logs and job receipts. Track cold-load latency,
worker count, active watchers, queue depth, cancellations, and memory usage.
Diagnostic/list/status paths must avoid model loading and writes. Counts and
stored metadata do not prove that indexing completed successfully.

Errors should identify the visible affected workspace and the exact recovery
command. Reindex jobs have durable IDs, cancellation behavior, and status;
MCP transport disconnect is not automatically proof that the job stopped.

## 15. Search boundaries and future federation

Workspace search already covers its configured repositories; this is the
normal Assign experience, not optional cross-workspace federation. Preserve
repository provenance and use an explicit repository filter for narrower work.

Later multi-workspace search must be opt-in, authorized before dispatch, bounded,
and labelled with partial failures. Membership in a registry does not by itself
authorize global search. Global enumeration must not wake every workspace or
send queries to paid embedding providers without explicit configuration.

Do not compare raw vector/BM25 scores from unrelated index configurations as
though calibrated. Prefer grouped results first; any rank-based fusion has a
documented deterministic policy and evaluation fixtures. Cross-workspace graph
traversal and workspace groups stay deferred.

## 16. Verification and release acceptance

Use deterministic local fixtures and mocked or disabled inference. Keep provider
credentials and `.secrets` out of all tests, probes, benchmarks, and CI.

The core acceptance fixture contains Assign with at least two repositories plus
a separate Codanna workspace. All intentionally contain the same symbol names,
relative paths, collection aliases, local IDs, and similar conversation text.

Required behaviors:

- Workspace enumeration returns Assign and Codanna, not each Assign member as
  a separate workspace. Member enumeration returns the configured repositories.
- An Assign search includes its members by default and excludes Codanna.
  Repository filters narrow code, docs, and recall consistently.
- Graph/name resolution cannot cross repositories on name equality alone, and
  graph traversal never crosses unrelated workspaces.
- Concurrent clients keep independent defaults. Shared-root selection,
  ambiguous roots, unsupported roots, reconnects, and root changes are tested.
- Symlinks, nested folders, model-cache-only `.codanna` directories, malformed
  nearest configs, aliases, moved checkouts, and copied markers are covered.
- Registry writes do not lose concurrent registrations; corrupt/unknown-version
  registries are preserved. Separate processes cannot acquire duplicate writers.
- Rebuilding Assign leaves Codanna unchanged. Failed discovery leaves the prior
  index recoverable. Removing membership cannot leave hidden queryable sources.
- Qualified references reject mismatched workspace/repository/generation.
  Resources, notifications, and recall follow the same scope as tool results.
- One failed worker does not break healthy workspaces. Cancellation, eviction,
  load coalescing, backoff, and graceful shutdown have executable coverage.
- Registration does not rebuild an existing index. Required storage migrations
  are explicit and never create mixed old/new repository semantics.

Benchmark warm routing against the direct single-workspace baseline and report
cold loads separately. Register many unloaded fixtures to measure metadata-only
startup. Set numerical performance budgets from those measurements, not from
unverified sub-millisecond promises.

The end-to-end completion scenario is Codex in Assign, another client in Codanna,
and an explicit product-wide Assign query sharing one HTTP service without
context contamination, duplicated writers, or misleading graph relationships.

## 17. Delivery rule

Follow the linked implementation checklist in small reviewable slices. Do not
mark the feature complete after adding only a registry, one new MCP parameter,
or a router around storage that still loses repository ownership. Preserve
single-workspace behavior and test every newly introduced boundary before
advertising multi-workspace support in the README.
