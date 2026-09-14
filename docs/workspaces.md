# Local product workspaces

**Status: implementation under review in PR #34.** This branch implements
workspace registration, explicit process selection, and automatic local discovery.
The shared MCP router and repository-qualified graph architecture remain incomplete.
See [the feature design](design/multi-workspace.md) for the larger target.

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

The same MCP entry can be reused in clients that launch commands from their
current project directory:

```json
{
  "mcpServers": {
    "codanna": {
      "command": "codanna",
      "args": ["serve"]
    }
  }
}
```

This reuses one configuration, not one server process. Separate stdio clients
still start independent servers. Clients that launch from HOME need an explicit
working directory or workspace selector until MCP roots negotiation is implemented.
HTTP/HTTPS servers continue to use explicit selection and their existing network
access policies; independent servers need distinct bind ports.

First indexing remains explicit. Connecting an unindexed `serve` process reports
that `codanna index` is required rather than starting a long indexing job before
the MCP handshake. Automatic indexing after connection remains future work.

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
Set `CODANNA_AUTO_SETUP=0` to retain legacy implicit startup.

The automatic path applies to bare nondry indexing, local stdio serving, and
ordinary `retrieve`, `mcp`, and `dump` commands. It does not bootstrap help,
completions, utility commands, explicit-path indexing, or dry runs. This does not
change the legacy dry-run implementation.

Registration uses the existing v1 `projects.json` and IDs. Registry writes use
process-level locking and atomic replacement. Older binaries do not participate
in the new lock protocol. Rename and explicit relocation preserve identity;
relocation requires the original directory to have been moved already. Removal
unregisters only: it does not delete files or stop independent servers.

## Remaining limitations

The new selector still requires local configuration, local index storage, and
code roots beneath the product. External repository membership, repository IDs,
repository-partitioned graph resolution, shared worker management, and per-request
workspace routing are not implemented. Repository discovery alone is not proof
of graph isolation inside a multi-repository workspace.

Selected launches use a fixed working directory and exact configuration. They
remove inherited `CI_*` configuration overrides and inherited recall workspace
and index bindings. **Automatic launches use that same policy: conversation
recall is disabled until workspace-specific bindings are implemented.** Explicit
`--config` keeps the existing configuration and recall behavior. Other provider
environment settings retain existing behavior.

Partial selected rebuilds are rejected except for the validated initial full-root
index of a fresh empty configuration. Index-writer and watcher coordination across
separate server processes remains a later implementation stage.

## Tests

```bash
cargo test --lib workspace_auto_ --all-features
cargo test --test workspace_auto --all-features
cargo test --test workspace_cli --all-features
```

The automatic-workspace integration suite indexes tiny deterministic fixtures with
semantic search disabled and tests separate Assign and Codanna queries, nested
selection, preserved child configuration, read-only diagnostics, and initial setup.
The subprocess fixtures use temporary home directories and a cleared environment.
Their hardening-prefixed names include them in the existing Hardening workflow.
Unit tests exercise root selection, inventory budgets, symlinks, aliases, and
configuration preservation. Process witnesses currently run on Unix. The PR
records observed validation results and pending platform qualification.
