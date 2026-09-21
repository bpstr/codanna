# JSX ownership and namespace impact

A focused implementation of T03 from [the calendar investigation, PR #43](https://github.com/bpstr/codanna/pull/43).
Changes and revision-specific verification are tracked in [PR #46](https://github.com/bpstr/codanna/pull/46).
This repairs graph construction, independently of the graph-response wording in
PR #44 and request-limit diagnostics in PR #45. Ranking and semantic indexing are
not changed.

## Correctness contract

```tsx
export const Page = () => <Calendar />;
export function Shell() {
  const Panel = () => <Calendar compact />;
  return <Panel />;
}
```

`Page` and `Panel` own their respective Calendar Uses edges. `Shell` owns its
reference to `Panel`, rather than inheriting the JSX inside the nested arrow.
Class methods such as `render` also provide a named parser owner. Configured
function wrappers reuse the existing wrapper-name detection; anonymous callbacks
retain the prior enclosing-owner behavior.

Member-expression JSX such as `<ui.Calendar />` and `<motion.div />` represents a
value even when its namespace or property starts lowercase. Intrinsic `<div />`
and `<span />` do not become component edges. Closing tags do not duplicate Uses.

Qualified Uses with an imported root resolve the actual export slot through the
existing export/barrel machinery. A local namesake cannot stand in for a missing,
ambiguous, or external export. Parameter, destructuring, catch, and directly
rebound block-local namespace evidence is treated conservatively; this is not a
complete JavaScript object-member/type inference engine. Type-only exports can
remain type Uses; this does not create runtime Calls.

The parse stage retains the usage start line and column in relationship metadata.
The parser fixture checks the complete JSX range; the stored metadata retains
only its start position. These parser/storage positions are zero-based, unlike
human-readable MCP locations. JSX rendering does not invent an immediate Calls
edge.

The collector reconnects a relationship's source name using both line and column
containment. It selects the innermost enclosing range, including when declarations
start on the same line or at the same position. Declaration-order permutations
must not exchange owners. The previous last-declared fallback remains for parsers
whose name-only ranges cannot enclose a use site.

## Regression fixtures

| Fixture target | Boundaries exercised |
| --- | --- |
| `tests/jsx_impact_regressions.rs` | Nine tests: exact parser owners/targets/ranges; arrows and class methods; namespace versus intrinsic syntax; aliases/barrels and same-name decoys; source metadata; external/shadowed namespace controls; public MCP impact depth; save/reopen, import-only edit, deletion and recreation. |
| `tests/jsx_owner_columns.rs` | Six tests: disjoint and nested same-line owners, coincident starts, shared boundary-line columns, name-only fallback compatibility, and real TSX arrows with the same name on one source line. The latter asserts outgoing stored Uses and their source positions. |

Only hand-authored source files enter disposable indexes. Assertions remain
outside the indexed root. Semantic indexing is disabled. No provider, model,
private application source, conversation import, or production index is used.
Some missing-member/external controls are deliberately unresolved syntax; this
is a code-intelligence regression corpus, not a claim that every negative fixture
passes a TypeScript compiler.

```bash
cargo test --locked --test jsx_impact_regressions --test jsx_owner_columns
cargo test --locked --lib indexing::pipeline::stages::collect::tests
cargo test --locked --test web_export_regressions --test callback_references --test index_lifecycle_regressions
./contributing/scripts/quick-check.sh
./contributing/scripts/full-test.sh
```

## Recorded boundary repair

[Run 35667134005](https://github.com/bpstr/codanna/actions/runs/35667134005)
first executed the corrected nine-test harness at
`9b550b3167ba2cd86da25b49e5e935604fa9fe9f`: **2 passed, 7 failed**.
After runtime commit `08f44a7bff9a8cecc4ea13ba6e269b879e0ead13`,
the same test file produced **9 passed, 0 failed**. Its unchanged SHA-256 is
`8ce5ef17139da6ef6847f9b17457f410905eabe86cf9da601dacedc8425f1a70`.
The neighboring export, callback-reference, and index-lifecycle targets produced
23, 5, and 6 passing tests respectively. Their test counts do not qualify the
entire repository or model relevance.

## Recorded source-column repair

[Run 35667592843](https://github.com/bpstr/codanna/actions/runs/35667592843)
executed the six additional tests at `1f31c7adb80f39bc5400554edb678457c4e03d7e`:
**1 passed, 5 failed**. The name-only fallback control passed; the disjoint,
nested, coincident-start, boundary-column, and real same-line TSX witnesses failed.
Collector commit `f6e6458eac631e2ce2e4590561ec72da1968fae5` then produced:

| Executed target | Passed | Failed |
| --- | ---: | ---: |
| JSX boundary/impact regressions | 9 | 0 |
| Source-column regressions | 6 | 0 |
| Existing collector unit tests | 11 | 0 |

The unchanged column-test SHA-256 is
`a30dbe2df896b8027f773eb24382806473ad3038011b5caa44ac259075ab7dec`;
the candidate collector source SHA-256 is
`1bc9b71abfcf91b941fcb99419d85a7606f4692c8cec4c96341dfda396ea415a`.
The `jsx-column-evidence` artifact retains baseline/candidate logs and tested
source. These results are actual GitHub CI executions on Linux with Rust stable;
no local Rust execution or exact binary hash is claimed. Earlier fixture
compilation and formatting failures are harness failures, not behavioral baseline
evidence. Full-suite completion is tracked separately in the PR and is not
inferred from these focused passes.

The temporary branch-only patch-application workflow is removed after publishing
the source commits. The retained `JSX impact contracts` workflow is read-only
and runs both regression targets. Existing indexes do not change retroactively:
reindex affected source with the updated binary through the normal workspace
workflow. No query silently rebuilds an index and no storage schema is migrated.

## Remaining T03 coverage and next opportunities

The executed lifecycle fixture uses direct facade mutations, not real watcher
events. Watch parity, all MCP transports, a conflicting peer workspace, and the
exact historical private Assign index are not qualified here. Full task closure
requires those separate witnesses and the repository gates.

Further focused cases should distinguish function-expression/wrapper owners,
loop-header namespace rebinding, nested namespace and project-alias JSX, and
nearest-scope target resolution for duplicated nested component names. Source
owner reconnection and target binding are different stages: the same-line fixture
repairs/tests the former, not every possible same-name callee lookup. Add exact
positive and forbidden-target assertions before widening resolution heuristics.

References: [TypeScript JSX](https://www.typescriptlang.org/docs/handbook/jsx.html)
and [React element types](https://react.dev/reference/react/createElement).
