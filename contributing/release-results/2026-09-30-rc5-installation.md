# September 30 RC5 installation

**Installed:** `codanna 1.0.0-rc5 (be166174)` at
`/Users/bpstr/.local/bin/codanna`, together with matching `codanna-index-plan`,
`codanna-knowledge`, `codanna-manifests` and `codanna-recall` executables.

The annotated tag `v1.0.0-rc5` was pushed to the fork and resolves to the exact
qualified commit `be166174d0b59d375ad05f5b3e3ccbdf7d146d74`. Tagging and replacement followed the user's
approval after the [qualification results](2026-09-30-rc5.md): local artifact
checks and both Linux test lanes had passed before promotion.

All five executables were staged beside their resolved installed paths, hashed,
flushed, given the original executable permissions and atomically renamed into
place. The main binary was published last. All installed hashes match
`candidate-final-manifest.json`; the main SHA-256 is
`27ab9a2b1dac38f48b71ddda261871a962d804ee52923b74b9c40112d74a7dc0`.

## Installed-path verification

Version and all five executable help commands passed. The local manual's scratch
smoke passed using the installed path: fresh graph indexing, exact symbol/call
lookup, separate-process reopen, unchanged refresh, metadata and actual stdio MCP
startup. Assertions found one source file, two symbols and one call relationship.
Embeddings were disabled; no paid inference or model download occurred.
The active Assign index, its v2 policy and settings were not changed.

Evidence is retained at
`/Users/bpstr/Github/codanna-release-evidence/2026-09-30-rc5/`, including
`installation.json`, `manual-smoke-installed.log`, the scratch path recorded in
that log, installed help outputs and the original qualification/failure logs.

## Rollback and existing servers

The verified RC4 executable set remains in
`/Users/bpstr/Github/codanna-release-evidence/2026-09-30-rc5/rollback/`.
`preflight.json` records original installed paths, permissions and hashes. Restore
those saved executables with sibling-file atomic replacement if rollback is
needed; do not mix auxiliary versions or overwrite indexes as part of rollback.

There are 11 surviving pre-installation MCP servers (PIDs
2555, 7577, 9200, 12268, 15060, 22891, 43969, 58600, 58755, 89757, 96188). They span Assign, Codanna and other workspaces and
still hold the previous executable. No unrelated client was killed or restarted.
Their owning clients must restart these servers to load RC5; replacing the file
alone does not upgrade them. `servers-before-install.json` records their commands.
A fresh installed-path scratch MCP server successfully loaded RC5.

## Publication and disk

Tag-triggered release packaging and repeat CI were running when installation was
recorded. Their generated cross-platform artifacts are separate from this tested
macOS executable. No stable/public release was manually published, and no success
claim is made for those still-running jobs.

No large build was run during installation. About
35.02 GiB remained free after verification. Installation staging
files were consumed by atomic renames; rollback and qualification evidence remain
intact. The earlier qualification receipt records its bounded build cleanup.
