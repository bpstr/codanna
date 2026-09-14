# Independent local workspaces

**Status:** Under review in PR #34. Automatic setup, registry administration,
explicit selection, and local read-only MCP routing are implemented on the branch.
Shared HTTP routing, repository-qualified graph semantics, and workspace-bound
recall are not complete. See the [design](design/multi-workspace.md) and
[MCP guide](workspace-mcp.md).

## Normal usage

A workspace is an independently indexed coding project. Each workspace uses its
own graph/index and configuration. It can be a single repository, a monorepo, or
multiple repositories intentionally indexed together. There is no built-in project
list, prescribed repository layout, or required business/product hierarchy.

Run this in each intended workspace:

```bash
cd /path/to/your/project
codanna index
```

The path above is a placeholder. A fresh bare `index` discovers the boundary,
creates missing settings/source defaults, and registers the workspace. No manual
`workspace add`, ID selection, or per-repository `add-dir` list is required for a
standard layout. Existing source lists and ignore files are preserved.

For an intentionally combined multi-repository workspace, run the first index
from its containing root. Later queries from covered member directories select
that established workspace. This grouping is optional; unrelated checkouts keep
separate indexes and are not combined based on their names or location.

## One reusable MCP entry

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

This opts the local stdio connection into read access to registered workspaces.
Supported client roots select the workspace, or tools can supply an ID/alias or
registered project path. HOME launch can handshake and enumerate workspaces
without opening any index; it does not guess a default project.

Ordinary `args: ["serve"]` remains bound to the launch directory or explicit
configuration/selector and retains its supported watching behavior. It does not
silently gain registry-wide access. Each stdio process has its own worker pool;
the router is not a shared machine-wide daemon. HTTP/HTTPS servers still require
explicit setup and retain existing network controls.

First indexing remains explicit. An unindexed ordinary server returns guidance;
the router can connect without any index and reports guidance when a knowledge
query needs one. Connecting does not trigger a rebuild or paid inference batch.
Semantic queries retain the selected workspace's configured provider behavior.

## Discovery and preservation

Discovery uses configured-source ownership, the nearest Git/worktree checkout,
recognized manifests, or an explicitly opened repository container. Starting in
an unknown child never scans siblings to infer which projects belong together.

An established parent may own configured descendants even when they retain older
standalone settings. Discovery does not overwrite/import child settings or index
files, and old standalone registrations remain. Explicitly selecting a child
configuration retains its standalone behavior.

HOME/filesystem roots are not automatic choices. Malformed applicable configs
fail instead of selecting another workspace. A model-cache-only `.codanna`
directory is not a project. Symlink-equivalent roots deduplicate; separate clones
and worktrees keep separate local storage. Aliases derive from folder names and
are disambiguated; existing aliases are preserved.

A fresh workspace uses `.` and initial cache/dependency exclusions only when no
settings/ignore rules already exist. An empty source list can receive a root
default only with absent/empty index storage. Populated indexes are not silently
broadened or cleared.

## Inspect and override

```bash
codanna workspace discover --json
codanna workspace list --json
```

`discover` explains the root, selection reason, configuration presence, and bounded
repository inventory without writes, model loading, or index creation. It reports
truncation, skips caches/dependencies and directory symlinks, and is not an index
plan or persisted repository-identity map.

Use an ID returned by `list` for optional management or explicit selection:

```bash
codanna workspace show <workspace-id> --json
codanna workspace doctor <workspace-id> --json
codanna --workspace <workspace-id> index
codanna --workspace <workspace-id> retrieve search "authentication"
codanna workspace rename <workspace-id> <new-alias>
codanna workspace move <workspace-id> /path/to/moved/project
codanna workspace remove <workspace-id>
```

Angle-bracket values are placeholders to replace, not shell commands to paste
literally. `doctor` checks configuration and directory presence, not completeness.
Manual `workspace add /path/to/project --name <alias>` is available for initialized
projects but is not needed for normal automatic setup.

`--workspace` and `--config` are mutually exclusive and override discovery.
Selected-command relative paths resolve from the selected root. Set
`CODANNA_AUTO_SETUP=0` to opt out of implicit automatic startup; the explicitly
requested `workspace serve` remains a separate mode.

Automatic startup applies to bare non-dry indexing, ordinary local serving,
`retrieve`, `mcp`, and `dump`. Help, utilities, explicit-path indexing, and dry
runs retain their separate paths. This feature does not redesign legacy dry runs.

The existing v1 registry/IDs are preserved. Registry updates use process locks,
atomic replacement, and stale-snapshot protection. Older binaries do not take
these new locks. Rename/validated relocation preserve identity; relocation requires
the original directory to have moved already. Removal unregisters only and leaves
source/configuration/index files intact. It blocks new router calls but does not
revoke in-flight requests or terminate independently launched servers.

## Remaining limitations

Current automatic/selected workspaces require local index storage and source
roots beneath the workspace. External membership, persisted repository IDs,
generation-qualified references, and repository-partitioned graph resolution
remain unfinished. Discovering multiple repositories is not proof that all
cross-repository graph edges are correct.

The router advertises read-only tools, not resource subscriptions, scoped watcher
notifications, or custom mutations. Shared HTTP routing and cross-process writer/
watch ownership remain separate completion gates.

Selected launches pin cwd/config and remove inherited `CI_*` overlays and recall
bindings. **Automatic and router-worker launches disable conversation recall
until proper workspace-specific binding exists.** Explicit `--config` retains
legacy recall behavior. Other provider environment behavior is unchanged.

Partial selected rebuilds are rejected except for validated initial full-root
indexing of fresh empty storage. No installed binary is changed by this PR.

## Verification

```bash
cargo test --lib workspace_auto_ --all-features
cargo test --test workspace_auto --all-features
cargo test --test workspace_cli --all-features
cargo test --test workspace_mcp --all-features
```

Fixtures use synthetic temporary workspaces, cleared credentials, disabled
embeddings, and isolated homes. They cover unrelated checkouts, optional member
roots, renamed aliases, real local indexing/MCP calls, preservation, ambiguous
roots, and failures. Consult the PR for actual tested commits/results; a test's
presence is not evidence of complete platform or release qualification.
