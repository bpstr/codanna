# Workspace-relative symbol search scope

This T07 slice adds an explicit, opt-in path/subtree scope to lexical symbol
discovery. It does not hide archive/reference code by default and does not add
an Assign-specific ranking rule.

## Contract

A scope is relative to the configured **workspace**, not the source directory
passed to the indexer. For a workspace containing `src/active/calendar`, use
`src/active/calendar`, even when only `src` was indexed. Examples:

- `src/active/calendar` for a subtree;
- `src/reference/calendar/Calendar.ts` for an exact file;
- `.` for the whole workspace.

Client backslashes are normalized to `/`; redundant separators and `.` components
are normalized. Absolute paths, drive-qualified paths, empty strings and any
`..` component are rejected. Component boundaries matter: `calendar` does not
include `calendar-old`, and `Calendar.ts` does not include `Calendar.ts.backup.ts`.
An unknown scope returns no matches. It is never retried against another indexed
root or broadened to an unscoped search.

Current indexed `file_info` rows supply matching `file_id` terms to a mandatory
Tantivy filter **before** TopDocs candidate collection. Out-of-scope symbols
therefore cannot consume the ranking budget and then be hidden afterward.
No new Tantivy field or index migration is required.

## Surfaces

- MCP `search_symbols.path_prefix`;
- MCP `search_context.code_path_prefix` (code section only);
- CLI `codanna retrieve search "calendar settings" --path-prefix src/active/calendar`;
- CLI retrieve aliases `path_prefix:...` and `path:...`;
- direct MCP CLI `codanna mcp search_symbols --args '{"query":"calendar settings","path_prefix":"src/active/calendar"}' --json`.

The MCP CLI argument catalog recognizes both scoped fields. Its JSON result
collector must use the same scoped facade as text dispatch; accepting a field
but dropping it during JSON collection is a correctness failure.

Invalid code scopes return tool errors, not successful unavailable/empty sections.
The direct `search_context` handler stops before document retrieval and recall.
MCP CLI JSON emits an error envelope with a failing process exit; text errors
also exit unsuccessfully. Type validation prevents a numeric/array/object scope
from being silently interpreted as an omitted filter. Valid missing scopes retain
the existing empty-result convention rather than inventing an index failure.

## Index lifecycle

Stored paths remain workspace-relative for identity and cleanup. Filesystem reads
resolve those keys against the configured workspace, independently of process cwd.
This applies to incremental modification checks, rename hashing and READ workers;
it does not authorize new roots or relax existing source-byte/regular-file limits.
No test changes global cwd to make the implementation appear correct.

## Verification boundaries

`tests/symbol_path_scope.rs` covers pre-top-k filtering, broad reference discovery,
exact-file and sibling boundaries, normalized aliases, invalid scopes, independent
language filters, same-name definitions, delete/recreate/reopen, direct MCP parity,
and invalid context scopes before other sources.

`tests/symbol_scope_cli.rs` exercises actual CLI processes for text/JSON parity,
retrieve output, malformed scope types, failing envelopes/exits and missing scopes.
Its HOME/config/source/index are disposable and semantic/document indexing is off.
See `symbol-path-scope-results.md` for executed revisions and outstanding checks.

This is retrieval scope, not an authorization boundary. The current file-scope
implementation scans indexed file registrations and does not yet claim a
large-workspace latency bound or atomic cross-reader snapshot across concurrent
index replacement. Those require separate measurements/fixtures before T07 is
marked complete. Workspace/network source policy still decides what may be indexed.
