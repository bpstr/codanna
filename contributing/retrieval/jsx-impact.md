# JSX ownership and import identity

Next T03 batch from [PR #43](https://github.com/bpstr/codanna/pull/43).
The fixture-only checkpoint intentionally precedes the runtime repair.

`tests/jsx_impact_regressions.rs` checks exact parser owners, targets and ranges;
resolved/persisted Uses edges; public MCP impact depth; namespace export identity
in the presence of local and reference decoys; shadowed/external receivers; and
reopen/import-edit/delete/recreate behavior. Test source and expectations remain
outside the disposable index. No embeddings, models, providers, or private
application source are used.

Baseline and candidate results have not been observed at this checkpoint.
This is not a claim to reproduce the exact private Assign index failure.

References: [TypeScript JSX](https://www.typescriptlang.org/docs/handbook/jsx.html)
and [React element types](https://react.dev/reference/react/createElement).
Intrinsic tags and component values need distinct handling; JSX rendering is a
Uses relationship, not an invented direct Calls relationship.
