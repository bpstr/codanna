# Independent coding-agent workspaces

Implementation is under review in PR #34. Workspaces are arbitrary local coding
projects with separate indexes and request context. There are no built-in project
names, customer layouts, or required repository inventories.

## Normal setup

Configure the MCP entry once:

```json
{"mcpServers":{"codanna":{"command":"codanna","args":["workspace","serve"]}}}
```

Open a project in the coding agent and use its Codanna tools. Supported client
roots, otherwise the MCP process's launch directory, select the project. First
knowledge use creates missing local settings and registration and starts a bounded
code-only index. No per-project registration, source list, workspace ID, MCP path,
or initial CLI indexing command is normally required.

Initial indexing may return `status: indexing` with `ready: false`; retry the same
query. An empty project can initialize when its first supported source arrives.
Setup errors never substitute another workspace's results. See the
[MCP behavior and limits](workspace-mcp.md).

Code-only workspaces then watch edits, additions, and removals automatically.
Reconnecting catches up offline changes. One reader owns the code-writer lease;
other connections follow its commits and can take over after it exits. A single
MCP configuration does not imply a machine-wide shared process: separate stdio
clients still have separate bounded reader pools.

## Root selection

An independently opened checkout or nearer configuration is not captured by an
ancestor merely because that ancestor indexes `.`. A plain non-Git client root
is also valid; an ancestor's bare Codanna configuration cannot widen that session.
Symlink-equivalent roots canonicalize to the same location. Separate Git worktrees
retain independent checkout/index locations.

A monorepo or deliberately combined root remains supported when that root is
actually opened. Opening a child checkout does not implicitly select the combined
parent. Multiple unrelated roots require a deliberate selection rather than
inferring shared ownership from names or ancestry.

A process launched in HOME must receive actual project roots from its client.
A running server cannot inspect another process's later cwd changes; the client
must publish root changes or relaunch in the new project. Missing context never
falls back to the last queried workspace.

## Optional inspection and administration

```bash
codanna workspace discover --json
codanna workspace list --json
codanna workspace show <workspace-id-or-alias> --json
codanna workspace doctor <workspace-id-or-alias> --json
codanna --workspace <workspace-id-or-alias> index
codanna workspace rename <workspace-id-or-alias> <new-alias>
codanna workspace move <workspace-id-or-alias> <new-path>
codanna workspace remove <workspace-id-or-alias>
```

Discovery is read-only and includes a bounded diagnostic inventory; ordinary
query routing does not run that inventory. Doctor reports metadata, not proof of
a completed healthy index. `get_index_info` reports the active reader's freshness.

Explicit `--workspace` and `--config` cannot be combined. Relative source arguments
for explicit workspace commands resolve inside that workspace. Registration and
rename do not rebuild; unregister does not delete local data or stop independent
processes. Relocation requires the original location to have moved already.
The existing v1 registry and IDs remain supported with locked atomic updates.

## Conversation recall and documents

Install the companion `codanna-recall` binary and import only a transcript you
explicitly select, from the intended project directory:

```bash
codanna-recall import --provider codex --file /path/to/selected-session.jsonl
codanna-recall search "previous decision"
```

The `claude` provider uses the same workflow. The importer and local MCP derive
the same namespace from the canonical project directory, without manual labels.
Equal basenames remain distinct; inherited legacy workspace labels cannot redirect
the router's recall. No private conversation directories are scanned automatically.
Moving a project does not silently relabel its old conversations. Explicit legacy
`--workspace` recall remains a separate opt-in mode.

Document state, storage, configured roots, and materialized hits must fit the same
workspace as code. Configured stores load lazily and refresh after published
metadata changes, including stores created after a first query. This is not
automatic document ingestion. A foreign or corrupt auxiliary store is refused
without disabling independent code-only symbol lookup.

## Compatibility and limits

Ordinary `codanna serve` retains its project-bound explicit configuration and
watch mode. `CODANNA_AUTO_SETUP=0` disables implicit CLI discovery; deliberately
invoking `workspace serve` is a separate mode. Network serving still requires
explicit configuration and existing authentication controls.

New automatic settings disable semantic search. Existing settings are preserved,
but bootstrap never invokes embedding providers. Existing semantic indexes or
providers use `semantic-manual` freshness mode instead of silently re-embedding;
use the established explicit semantic indexing/watch mode for that case. Explicitly
disabling file watching is also respected. Root ignore/configuration changes
trigger revalidation/reload rather than a scope fallback.

Updated code writers across CLI indexing, MCP reindex, bootstrap, and file watching
share the same OS-backed lease. A competing explicit writer fails before replacing
data; it does not steal the watcher's lease. Older binaries and direct external
storage writers do not participate in this advisory protocol.

Generation-qualified IDs, repository-partitioned graphs within intentionally
combined roots, external member adoption, a shared multi-workspace daemon, and
scoped subscription/mutation tools are not implemented by this local router.

See the [architecture](design/multi-workspace.md) and
[delivery checklist](design/multi-workspace-implementation.md). Commit-specific
CI evidence in the PR determines qualification; examples alone are not test results.
