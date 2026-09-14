# Local product workspaces

**Status: implementation under review in PR #34.** This branch implements
workspace registration, explicit process selection, automatic local discovery,
and an opt-in local read-only multi-workspace MCP router. Shared HTTP serving
and repository-qualified graph architecture remain incomplete.
See [the feature design](design/multi-workspace.md) for the larger target and
[the router guide](workspace-mcp.md) for protocol and worker boundaries.

## Normal usage

Index each independent product once from its root:

```bash
cd /projects/assign
codanna index

cd /projects/codanna
codanna index
```

Assign is one workspace containing its related repository directories. Codanna
is a different workspace. A fresh bare `index` creates the missing local
configuration, selects the product root as its source, and registers it.
Individual `workspace add` and `add-dir` commands are not required for this layout.
Existing nonempty source lists and existing ignore files are preserved.

After indexing Assign, ordinary queries from its member repositories select
the configured Assign parent:

```bash
cd /projects/assign/assign-web
codanna retrieve search "TaskStatus"
codanna serve
```

For one connection that can query several products, use this reusable MCP entry:

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

The explicit `workspace serve` command opts into local registry-wide read access.
It accepts per-tool workspace IDs/aliases or project paths and uses supported
client roots for automatic selection. A launch from HOME can handshake and list
workspaces without loading models; client roots or explicit tool selectors then
choose the product. Assign's member roots resolve to its parent workspace.

For strictly project-bound operation, keep `args: ["serve"]`. That mode remains
bound through the launch directory or an explicit configuration/selector; it does
not silently gain cross-workspace access. The new router does not replace this
mode or its existing supported watching behavior.

Separate stdio launches still own independent processes. The router shares lazy
workers across requests on its own connection, not implicitly across all client
processes. HTTP/HTTPS servers continue to use explicit selection and their existing
network policies; independent servers need distinct bind ports. Shared HTTP
workspace routing is not implemented yet.

First indexing remains explicit. Connecting does not launch a rebuild. An
unindexed ordinary `serve` reports setup guidance; `workspace serve` can connect
without any index and returns guidance when a selected knowledge query needs one.
Automatic indexing after connection remains future work.

## Discovery boundaries

A configured parent owns a child when its existing indexed source roots cover
that child. This can select Assign even when assign-web already has a standalone
configuration. Neither the child's configuration nor its existing index is
modified or imported by discovery. Old standalone registry entries are retained.

Without a Codanna configuration, bare indexing discovers the nearest Git checkout
or a recognized project manifest. Starting from a directory that contains member
repositories can establish that explicitly opened directory as the product root.
Run the first index from Assign, not from a generic directory holding unrelated
products. Starting in an unconfigured child does not scan its siblings or infer
product membership from repository names. The product boundary is established
once, not configured separately for every child.

Home and filesystem roots are not automatic project choices. Malformed nearest
configurations are errors; a directory containing only Codanna's model cache is
not a project configuration. Symlink-equivalent paths deduplicate. Worktree
checkouts retain separate local index locations. Aliases are generated from
folder names and disambiguated when names collide; existing aliases are preserved.

Fresh configurations use `.` as the source root and get minimal cache/dependency
exclusions when no ignore file exists. An initialized configuration with no
sources can receive the root default only while its index is absent or an empty
initialization skeleton. A populated index with an empty source list is rejected
instead of being silently broadened or cleared.

## Read-only inspection

```bash
codanna workspace discover
codanna workspace discover /projects/assign/assign-web --json
codanna workspace list --json
codanna workspace show assign --json
codanna workspace doctor assign --json
```

`discover` reports the selected root, reason, configuration presence, and observed
repository locations. Its inventory is bounded and reports truncation. It skips
dependency/cache directories and directory symlinks. This is an explanatory
inventory, not an indexing plan or persisted repository ownership map. It does
not register projects, initialize settings, load models, or open indexes.

`doctor` checks configuration and index-directory presence, not index completeness.
The MCP router also provides `list_workspaces` and `get_workspace` without opening
every registered index. Their presence is not a graph-isolation or readiness claim.

## Explicit controls

```bash
codanna workspace add /projects/assign --name assign
codanna --workspace assign config
codanna --workspace assign index
codanna --workspace codanna retrieve search "IndexFacade"
codanna workspace rename assign assign-product
codanna workspace move assign-product /development/assign
codanna workspace remove assign-product
```

Explicit `--workspace` and `--config` take priority over discovery and cannot be
combined. Relative paths for selected commands are interpreted from the selected
root. An explicitly selected child configuration retains its standalone behavior.
Set `CODANNA_AUTO_SETUP=0` to retain legacy implicit startup. The explicitly
requested `workspace serve` command is a separate mode, not implicit startup.

The automatic path applies to bare nondry indexing, local stdio serving, and
ordinary `retrieve`, `mcp`, and `dump` commands. It does not bootstrap help,
completions, utility commands, explicit-path indexing, or dry runs. This does not
change the legacy dry-run implementation.

Registration uses the existing v1 `projects.json` and IDs. Registry writes use
process-level locking and atomic replacement. Older binaries do not participate
in the new lock protocol. Rename and explicit relocation preserve identity;
relocation requires the original directory to have been moved already. Removal
unregisters only: it does not delete files or stop independent servers. The router
revalidates registrations for new calls; it does not revoke already running calls.

## Remaining limitations

The new selector still requires local configuration, local index storage, and
code roots beneath the product. External repository membership, repository IDs,
and repository-partitioned graph resolution are not implemented. Repository
discovery alone is not proof of graph isolation inside a multi-repository workspace.

Local per-request routing and a bounded lazy reader pool are implemented in
`workspace serve`. Network routing, shared workers across separate client
processes, subscriptions, and custom mutation routing remain separate work.
The new mode advertises tools only and rejects unscoped custom reindex requests.

Selected launches use a fixed working directory and exact configuration. They
remove inherited `CI_*` configuration overrides and inherited recall workspace
and index bindings. **Automatic and router-worker launches use that same policy:
conversation recall is disabled until workspace-specific bindings are implemented.**
Explicit `--config` keeps the existing configuration and recall behavior. Other
provider environment settings retain existing behavior.

Partial selected rebuilds are rejected except for the validated initial full-root
index of a fresh empty configuration. Index-writer and watcher coordination across
separate server processes remains a later implementation stage.

## Tests

```bash
cargo test --lib workspace_auto_ --all-features
cargo test --test workspace_auto --all-features
cargo test --test workspace_cli --all-features
cargo test --test workspace_mcp --all-features
```

The automatic-workspace integration suite indexes tiny deterministic fixtures with
semantic search disabled and tests separate Assign and Codanna queries, nested
selection, preserved child configuration, read-only diagnostics, and initial setup.
The MCP suite adds real protocol/worker tests for client roots, scoped concurrent
queries, HOME startup, and failure isolation. Subprocess fixtures use temporary
homes and cleared environments. Hardening-prefixed names include them in the
existing Hardening workflow. Unit tests exercise root selection, inventory budgets,
symlinks, aliases, configuration preservation, and scope validation. Process
witnesses currently run on Unix. The PR records observed validation results and
pending platform qualification; these commands are not evidence of a passing run.
