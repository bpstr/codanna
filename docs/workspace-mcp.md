# Local multi-workspace MCP

**Status:** Implementation under review in PR #34. This is the local, read-only
router increment, not completion of the shared HTTP or repository-graph design.
See the PR for exact tested commits and observed CI results.

## One setup for all products

Index the intended product roots with the normal automatic setup:

```bash
cd /projects/assign
codanna index

cd /projects/codanna
codanna index
```

Assign remains one workspace containing its related subrepositories. Codanna is
another workspace. There is no need to manually register every product or add
all Assign repositories individually for a fresh parent-root setup. Existing
nonempty source lists and ignore rules remain authoritative. Opening an unknown
child cannot reveal which of its siblings form a product: establish the intended
parent boundary by indexing that parent once.

Use one reusable MCP definition for cross-workspace access:

```json
{
  "mcpServers": {
    "codanna": {
      "command": "codanna",
      "args": ["workspace", "serve"]
    }
  }
}
```

`workspace serve` explicitly grants this **local stdio connection** access to the
user's registered workspaces. Ordinary `codanna serve` remains project-bound;
existing MCP entries are not silently widened to all projects. This new command
does not listen on HTTP, expose a network port, or bypass the existing network
authentication/host/origin policies.

This is one connection routing to several lazily loaded workers, not an implicit
machine-wide daemon shared by all independently launched clients. Each separate
stdio launch owns its own bounded worker pool.

## Automatic selection and explicit overrides

Tools accept either `workspace` (registry ID or alias) or `project_path` (absolute
local path), never both. These fields are removed before the remaining arguments
are passed to the existing tool implementation.

```text
list_workspaces()

search_context(query="task status")
search_context(workspace="assign", query="task status")
find_symbol(workspace="codanna", name="IndexFacade")
search_context(project_path="/projects/assign/assign-web", query="task status")
get_workspace(workspace="assign")
```

Without an explicit selector, a client that advertises roots supplies its current
project roots. Assign-core and assign-web roots both resolve to Assign. Roots
spanning Assign and Codanna are ambiguous and require an explicit selector.
Unknown roots are not silently discarded in favor of a different known product.
A client without roots support falls back to its launch directory; a launch from
HOME can still handshake, list workspaces, and use explicit tool selectors.

Roots are requested while handling a tool call, not during initialization.
Legacy clients use bounded `roots/list` requests. Protocol 2026-07-28 clients use
MRTR input requests with random, short-lived, single-use continuation handles
bound to the original tool and arguments. Root changes affect subsequent calls;
there is no mutable process-global current workspace. An explicit override on
one request does not change the next request's default.

Only local `file://` roots are accepted. Roots do not register new workspaces,
create settings, index source, follow remote hosts, or authorize arbitrary index
paths. The router resolves them through the existing local registry. Clients
that advertise roots but cannot answer them get actionable selection guidance,
not a silent fallback to an unrelated launch directory.

## Results and exact references

Every routed result includes a workspace text header and structured ownership:

```json
{
  "workspace": {"id": "example-stable-id", "name": "assign"},
  "result": null
}
```

The `result` value contains the backend's original structured content when
available; existing readable tool content is retained. Symbol-ID follow-up calls
must keep the returned workspace ID. Local symbol IDs are not globally unique,
and generation-qualified symbol references are not implemented by this increment.
Do not reuse an old symbol ID after a full rebuild without looking it up again.

`get_workspace` reports configuration/directory diagnostics and whether an idle
worker is loaded. It does not assert index completeness, compatible repository
semantics, or successful semantic embedding. During a concurrent operation the
nonblocking loaded diagnostic may report false; it is not process accounting.

## Loading and failure behavior

Initialization, tool enumeration, and workspace enumeration do not open indexes
or embedding models. An ordinary indexed-knowledge tool starts a workspace reader
on demand, using the running Codanna executable, that workspace's validated
configuration and cwd, and the existing launch environment policy. Code/documents
therefore use that worker's stores; inherited recall selectors are removed.

The pool holds at most four cached workspace slots and admits at most sixteen
workspace queries. Calls for different workspaces can proceed independently;
queries within one worker are serialized. Active or queued callers pin their
slot so eviction cannot start a duplicate worker for the same workspace.
Unpinned least-recently-used workers can be evicted at capacity. Five-minute idle
entries are pruned on subsequent pool activity, not by an always-running timer.
Transport teardown is asynchronous during eviction; this is a cache bound, not
a strict machine-wide process or memory guarantee.

Queue waiting and tool execution have thirty-second budgets; cold worker startup
has a separate ninety-second budget. Cancellation/timeouts close the affected
serialized reader rather than leaving that query running indefinitely. Errors
are not automatically replayed. Failed readers have a short retry backoff.
Graceful connection shutdown closes its worker transports.

Each ordinary call rechecks registration and validates the selected settings and
index metadata before dispatch. Configuration and code-index manifest changes
invalidate the cached reader. Unregistering a workspace blocks new routed calls;
it does not retroactively revoke an already executing query. Independent servers
started elsewhere are not managed by this router. This is not a transactional
snapshot across simultaneous external configuration/index writers.

A corrupt/missing workspace index returns a scoped error without disabling other
workspaces. Indexing is still an explicit `codanna index` operation. Connecting
to MCP never triggers a force rebuild, a paid inference batch, or first indexing.
Semantic queries retain the selected workspace's configured embedding behavior;
read-only does not mean every semantic query is inference-free.

## Deliberately unsupported in this increment

The router advertises tools only: resource subscriptions, file-change
notifications, custom reindex requests, and other mutation endpoints are not
forwarded through an unscoped default worker. Use the existing explicit local
commands for indexing and existing bound-server mode for supported watching.

The following are still separate completion gates:

- Repository-qualified persisted identities and repository-partitioned graph
  resolution inside a product. Finding both Assign repositories does not prove
  that all graph edges between them are valid.
- Workspace-specific conversation recall. Selected worker launches still disable
  inherited recall until a reliable binding is available.
- Shared authenticated HTTP routing, cross-process writer/watcher coordination,
  worker sharing across separate client processes, and qualified subscriptions.
- External member-root adoption, automatic first indexing after handshake,
  complete platform qualification, and measured performance budgets.

The router reuses the actual generated symbol/search/context tool schemas rather
than maintaining a separate hard-coded tool list. It does not change the existing
single-workspace storage format or claim that the remaining architecture is done.

## Verification

```bash
cargo test --test workspace_mcp --all-features
cargo test --lib workspace_mcp_ --all-features
```

The Unix subprocess witnesses create tiny temporary Assign/Codanna sources,
index with semantic search disabled, clear inherited provider credentials, and
exercise real MCP handshakes/tool calls and worker subprocesses. They cover
HOME startup without writes, two products on one connection, scoped overrides,
legacy/MRTR roots, ambiguous/unknown roots, continuation tampering/replay, and
failure/unregistration isolation. No fixture indexes a real user project or
spends paid inference credits. Consult PR #34 for observed results; the existence
of a test or this guide is not a claim that every CI gate has passed.
