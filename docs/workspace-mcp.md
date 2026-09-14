# Local multi-workspace MCP

**Status:** The local read-only router is under review in PR #34. This is not a
shared HTTP daemon or a completed repository-provenance implementation. See
[workspace setup](workspaces.md) and the [architecture](design/multi-workspace.md).

## Reusable setup

Index each intended coding-agent workspace with `codanna index` in its root.
Configuration/source defaults and registration are automatic for standard fresh
layouts. Existing source lists and ignore files remain authoritative. A workspace
can be a repository, monorepo, or intentionally combined repository container;
there is no fixed inventory, required product layout, or recognized project name.

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

This grants the local stdio connection read access to registered workspaces.
Each workspace keeps a separate graph/index and runtime context. Ordinary
`codanna serve` remains project-bound. The new mode does not open a network port,
replace existing network access controls, or share workers between independently
launched client processes.

## Scope selection

Tools accept either `workspace` (a registry ID/alias) or `project_path` (an absolute
local path), never both. These fields are removed before strict backend argument
validation. There are no built-in selectors; obtain IDs from `list_workspaces`.

```text
list_workspaces()
search_context(query="authentication")
get_workspace(workspace="<id-returned-by-list_workspaces>")
search_context(project_path="/path/to/your/project", query="authentication")
```

The ID and path above are placeholders. When no selector is supplied, supported
client roots select the workspace. Several roots already belonging to the same
workspace are unambiguous; roots spanning unrelated workspaces require selection.
Unknown roots are not discarded to select a different known graph. Without roots
support, the launch directory is used. HOME launch can handshake/list workspaces
and use explicit selectors without creating state or guessing a default.

Roots are requested during a tool call, not initialization. Legacy clients use
bounded roots RPC; newer negotiated clients use MRTR input requests. Continuation
handles are random, short-lived, single-use, and bound to the original tool and
arguments. Root changes affect subsequent requests. A per-call override does not
change the next call's default, and there is no process-global current workspace.

Only local `file://` roots are accepted. Paths/roots cannot register workspaces,
create settings, start indexing, follow remote hosts, or authorize arbitrary
indexes. A roots-capable client that cannot answer receives selection guidance
rather than fallback into the launch directory's unrelated graph.

## Ownership and references

Every routed result has a workspace text header and structured ownership. The
workspace ID/name come from the selected registration, never a hard-coded label.
The `result` field contains original backend structured content when available;
readable backend content is retained.

Keep the returned workspace ID on symbol-ID follow-ups. Local IDs are not globally
unique. Generation-qualified references remain unfinished; look up IDs again
after a full rebuild rather than assuming an old ID still refers to the same symbol.

`get_workspace` reports configuration/directory diagnostics and whether a worker
can be observed as loaded. It does not establish index/embedding completeness or
repository isolation. Its nonblocking loaded check may report false during a
concurrent operation; it is not authoritative process accounting.

## Workers and limits

Handshake, tool enumeration, and workspace enumeration open no index/model. A
knowledge query lazily starts the running Codanna executable with that workspace's
validated config/cwd. Code and documents use its stores. Inherited recall bindings
are disabled until workspace-specific recall is implemented.

The pool has four cached slots and sixteen admitted workspace queries. Different
workspaces can run independently; a worker serializes its own queries. Active and
queued calls pin slots. Unpinned least-recently-used entries can be evicted at
capacity, and entries idle for five minutes are pruned on later pool activity.
There is no idle timer or strict machine-wide process/memory guarantee; transport
teardown during eviction is asynchronous.

Queue and query budgets are thirty seconds each, with ninety seconds for cold
startup. Cancellation/timeouts close the affected serialized reader. Failures
have a short retry backoff; calls are not automatically replayed. Connection
shutdown closes its worker transports.

Each ordinary call revalidates registration, settings, and index metadata.
Configuration/code-manifest changes invalidate the cached reader. Unregistration
blocks new calls but does not revoke in-flight work or stop independent servers.
This is not a transactional snapshot against simultaneous external writers.

A broken workspace returns a scoped error without disabling other workspaces.
Indexing remains explicit. MCP connection never triggers a force rebuild, first
indexing, or paid inference batch. Semantic queries still use the selected
workspace's configured provider; read-only does not mean inference-free.

## Unsupported surfaces and remaining work

The router advertises tools only. Resources, subscriptions, file-change events,
custom reindex requests, and other mutations are not forwarded to a default
worker. Use the existing bound-server mode for supported watching and local CLI
commands for indexing.

Remaining gates include repository-qualified persisted identities and graph
resolution for multi-repository workspaces, generation-qualified references,
workspace-bound recall, shared authenticated HTTP, cross-process writer/watch
coordination, scoped subscriptions, external membership, automatic first indexing
after handshake, platform qualification, and measured performance budgets.

The router reuses actual generated tool schemas, not a second hard-coded list.
Separate workspace selection does not by itself prove all unfinished boundaries.

## Verification

```bash
cargo test --test workspace_mcp --all-features
cargo test --lib workspace_mcp_ --all-features
```

Real protocol/worker fixtures use synthetic temporary sources and homes, cleared
credentials, disabled semantic search, and deadlines. Coverage includes concurrent
workspace queries, optional member roots, arbitrary renamed aliases, HOME startup
without writes, overrides, legacy/MRTR roots, ambiguity, continuation tampering/
replay, and failure/unregistration isolation. No fixture indexes user projects or
spends paid inference. Actual run results and pending qualification belong in the PR.
