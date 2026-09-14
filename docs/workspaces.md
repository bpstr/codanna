# Workspace registration and explicit selection

**Status: first implementation slice, under review in PR #34.** This adds CLI
registration and explicit per-process selection. It is not the completed shared
multi-workspace MCP service described in [the design](design/multi-workspace.md).

## Model and setup

Assign is one configured root with its related source directories; the unrelated
Codanna repository is another root. Register the parent Assign configuration,
not every child directory. Git submodules are not required.

Initialize each root with the existing `codanna init` command if necessary.
Do not force-regenerate an existing configuration. Then register them:

```bash
codanna workspace add /projects/assign --name assign
codanna workspace add /projects/codanna --name codanna
codanna workspace list --json
codanna workspace show assign --json
codanna workspace doctor assign --json
```

Registration changes the existing `~/.codanna/projects.json` registry only; it
neither indexes sources nor rewrites project settings. A sibling `projects.lock`
serializes writers in this version. The v1 on-disk registry and existing IDs are
retained. Older Codanna processes do not participate in the new locking protocol;
do not concurrently modify the registry with an older binary.

Discovery follows the nearest valid `.codanna/settings.toml` from the supplied
path. An unconfigured `assign/assign-web/src` therefore resolves to Assign. A
child that already has its own `.codanna/settings.toml` still forms a separate
legacy boundary: automatic adoption into a parent product is not implemented.
A model-cache-only `.codanna` directory is not a configuration. A malformed
nearest configuration is an error, not permission to select an ancestor.

## Select from any directory

```bash
codanna --workspace assign config
codanna --workspace assign add-dir assign-core
codanna --workspace assign add-dir assign-web
codanna --workspace assign index
codanna --workspace codanna retrieve search "IndexFacade"
```

The global long option accepts an exact alias or the ID returned by `list`.
Relative command paths are interpreted from the **selected workspace root**.
Do not combine `--workspace` with `--config` or with `workspace` management
commands, which take their own positional selector.

The selected command is launched with a fixed cwd and absolute configuration
path, before the existing application loads providers, indexes, or models.
Unix uses `exec` to preserve PID, signals, stdio, and the child exit status;
the non-Unix implementation waits for the child and needs platform qualification.
There is no global mutable current workspace and no resident router in this slice.

Normal existing commands without `--workspace` retain their previous behavior.
Legacy `indexed_paths` still supplies source directories. The proposed
`[workspace]` and `[[workspace.repositories]]` configuration, member-management
commands, and repository-qualified storage are **not implemented yet**. Do not
use the proposed configuration as though it were supported.

## Separate MCP entries today

Each entry can now explicitly select its configuration independent of client cwd:

```json
{
  "mcpServers": {
    "codanna-assign": {
      "command": "codanna",
      "args": ["--workspace", "assign", "serve"]
    },
    "codanna-codanna": {
      "command": "codanna",
      "args": ["--workspace", "codanna", "serve"]
    }
  }
}
```

These remain two independently launched single-workspace servers. One connection
with per-tool workspace selection, shared workers, and MCP roots negotiation
remains unimplemented. HTTP processes require different bind ports when running
simultaneously; there is not yet a shared HTTP workspace endpoint.

## Administration and safety limits

```bash
codanna workspace rename assign assign-product
# After moving the folder yourself, with the original location no longer present:
codanna workspace move assign-product /development/assign
codanna workspace remove assign-product
```

Renaming and explicit relocation preserve the registry ID. A second live copy or
worktree is not silently treated as the original. Removal unregisters only: it
does not delete sources or indexes, stop independently running servers, or revoke
already running processes. Symlink-equivalent roots register idempotently.
Duplicate legacy aliases fail selection; use exact IDs to rename/remove them.

`doctor` reports configuration validity and whether the index directory exists.
It does **not** verify index schema, readability, embeddings, or indexing
completion. `configured` must not be interpreted as `index ready`.

For this first selection slice:

- The configuration and index must be local to the workspace. Copied
  `workspace_root` settings, external/shared index paths, parent traversal, and
  symlink index escapes are rejected before the selected command starts.
- Configured code roots must exist beneath that workspace. External repository
  membership and repository-aware path ownership are deferred.
- `--workspace ... index PATH` is rejected, including force mode. Configure source
  roots with `add-dir`, then run bare `index`. This avoids selecting a subset
  while the existing rebuild path can clear the complete selected index.
- Inherited `CI_*` configuration overlays are removed from the child to prevent
  overriding the validated target. Put intended settings in that workspace's
  configuration instead. Other environment settings retain existing behavior.
- Inherited `CODANNA_RECALL_WORKSPACE` and `CODANNA_RECALL_INDEX` are removed.
  Recall is consequently disabled in explicitly selected launches until a real
  workspace-specific recall binding is implemented. Ordinary launches keep the
  existing recall behavior.

These checks are not the completed repository/knowledge authorization model.
This slice does not add repository-qualified IDs, partition graph resolution,
validate every document source, or coordinate index writers/watchers across
servers. Never treat a merged multi-root graph as proven repository isolation.

## Implementation verification

Focused tests added with this slice:

```bash
cargo test --lib workspace_ --all-features
cargo test --test workspace_cli --all-features
```

The process-level suite uses isolated HOME directories and disabled semantic
search, including eight concurrent registry writers. No test indexes code,
loads an embedding model, or receives provider credentials. Process-level
fixtures currently run on Unix; equivalent Windows process witnesses are pending.
Run the repository quick/full gates before merge. The PR records which checks
actually ran; the existence of these commands is not evidence that they passed.
