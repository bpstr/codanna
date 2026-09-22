# Workspace-relative symbol search scope

This T07 slice adds an explicit, opt-in path/subtree scope to lexical symbol
discovery. It does not hide archive/reference code by default and does not add
an Assign-specific ranking rule.

## Contract

A path scope is a portable workspace-relative path. Examples:

- `active/calendar`
- `reference/calendar`
- `.` for the whole workspace

Client backslashes are normalized to `/`. Absolute paths, drive-qualified
paths, empty strings and any `..` component are rejected.

The scope is resolved against current indexed `file_info` rows. Matching
`file_id` terms are added as a mandatory Tantivy filter **before** TopDocs
candidate collection. This prevents out-of-scope generic symbols from consuming
the ranking budget and then being hidden afterward.

No new Tantivy field or index migration is required.

## Surfaces

- MCP `search_symbols.path_prefix`
- MCP `search_context.code_path_prefix` (code section only)
- CLI `codanna retrieve search ... --path-prefix active/calendar`
- CLI key/value aliases `path_prefix:...` and `path:...`

Existing unscoped APIs delegate with no path prefix and keep broad discovery.

## Invariants

The regression corpus requires:

- limit-one scoped discovery cannot be crowded out by 96 archived generic
  `settings` symbols;
- unscoped search still discovers archive/reference code;
- a missing subtree returns no result rather than falling back globally;
- `.` matches unscoped ranking;
- escaping/absolute paths fail clearly;
- exact same-name `Calendar` definitions remain independently discoverable;
- MCP symbol search and the code section of `search_context` use the same
  pre-ranking scope.

This is a retrieval scope, not a security boundary. Workspace/network source
policy remains responsible for deciding what may be indexed in the first place.
