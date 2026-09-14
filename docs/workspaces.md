# Independent coding-agent workspaces

Implementation is under review in PR #34. Workspaces are arbitrary local coding
projects with separate indexes and request context; there are no built-in project
names, customer layouts, or required repository inventories.

## Normal setup

Configure the MCP entry once:

```json
{"mcpServers":{"codanna":{"command":"codanna","args":["workspace","serve"]}}}
```

Open a project in the coding agent and use its Codanna tools. The local client
roots, or the MCP process's launch directory when roots are unsupported, select
the project. First knowledge use creates missing local settings and registration
and starts a bounded code-only index. No per-project registration, source list,
workspace ID, MCP path, or initial CLI indexing command is normally required.

While initial indexing is running, a tool may return `status: indexing` with
`ready: false`; retry the same scoped query shortly. An empty project remains
selectable and can initialize when its first files arrive. Setup errors never
substitute results from another workspace. See [MCP behavior and limits](workspace-mcp.md).

One configuration does not mean a single machine-wide process: separate stdio
clients have separate router pools. Their indexes remain independent, and new
bootstrap writers for the same root coordinate through an OS-backed lock.
Shared HTTP service ownership and general CLI/watcher coordination remain
separate features.

## Root selection

An independently opened checkout or nearer configuration is not captured by an
ancestor merely because that ancestor indexes `.`. A plain non-Git local client
root is also valid; an ancestor's bare Codanna configuration cannot widen that
session. Symlink-equivalent roots canonicalize to the same location. Git worktrees
retain independent checkout/index locations.

A monorepo or deliberately combined source root remains supported when that root
is actually opened. Opening a child checkout does not implicitly opt into the
combined parent. Multiple unrelated roots are ambiguous and require a deliberate
selection, rather than guessing a shared product from names or directory ancestry.

A client starting the process in HOME must supply its actual project roots.
A long-running server cannot learn changes in another process's cwd by inspecting
its own cwd; clients must publish root changes or relaunch in their new project.
Invalid/unknown client context never falls back to the last queried workspace.

## Inspection and optional overrides

```bash
codanna workspace discover --json
codanna workspace list --json
codanna workspace show <workspace-id-or-alias> --json
codanna workspace doctor <workspace-id-or-alias> --json
```

Discovery is explicitly read-only and includes a bounded diagnostic repository
inventory. Ordinary query routing does not run this inventory. Doctor describes
configuration and index-directory presence, not proof of a complete healthy index.

Manual commands remain useful for recovery and unusual layouts:

```bash
codanna index
codanna workspace add <initialized-project-path> --name <alias>
codanna --workspace <workspace-id-or-alias> config
codanna --workspace <workspace-id-or-alias> index
codanna workspace rename <workspace-id-or-alias> <new-alias>
codanna workspace move <workspace-id-or-alias> <new-path>
codanna workspace remove <workspace-id-or-alias>
```

Explicit `--workspace` and `--config` cannot be combined. Relative source arguments
for an explicitly selected command are interpreted inside that workspace.
Registration and rename do not rebuild; unregister does not delete local data or
stop independent server processes. Relocation requires the original directory to
have been moved already and preserves identity rather than adopting a live copy.

The registry retains the existing v1 `projects.json` format and IDs, with locked
transactions, atomic replacement, and stale-snapshot protection. Older binaries
do not participate in the new lock protocol.

## Compatibility and remaining limits

Ordinary `codanna serve` retains its existing explicitly indexed, workspace-bound
mode and supported watch behavior. `CODANNA_AUTO_SETUP=0` opts out of implicit CLI
discovery; the deliberately invoked `workspace serve` is a separate mode.
Network serving still requires explicit configuration and existing access controls.

Automatic first indexing is local/code-only: new settings disable semantic search;
existing settings are preserved, but the initial indexing job does not perform
embedding calls. Existing embeddings and configured semantic providers are used
only by semantic operations. Large or incomplete indexes receive explicit recovery
guidance rather than a silent force rebuild. See the MCP guide for budgets.

The router validates configuration, local index paths, and loaded code provenance.
It does not provide repository-partitioned relationships inside a deliberately
combined workspace, generation-qualified IDs, external member adoption, or a
machine-wide daemon. Inherited recall bindings remain disabled pending explicit
workspace recall ownership. Scoped subscriptions and mutations are not exposed.

All examples and regression fixtures are synthetic. See the [architecture](design/multi-workspace.md)
and [delivery checklist](design/multi-workspace-implementation.md) for the remaining
release gates; commit-specific CI results in the PR determine what is qualified.
