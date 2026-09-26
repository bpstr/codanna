# Fixture follow-ups after the first implementation batches

This review distinguishes executed contracts from source findings and proposed
experiments. Graph response work is in [PR #44](https://github.com/bpstr/codanna/pull/44);
request-validation work is in [PR #45](https://github.com/bpstr/codanna/pull/45).
Neither PR claims to repair JSX resolution or retrieval ranking.

## What the first fixtures prove

The graph corpus separates an isolated function, an external browser call,
a resolved local call chain, and an unrelated same-named reference implementation.
Symbols are selected by path, exact name, and kind, then resolved to current IDs.
The oracle is outside the indexed root. It tests the public MCP methods rather
than only a formatter, including missing/ambiguous targets and budget errors.

The context fixtures distinguish a correctly rejected caller argument from a
retrieval failure. `conversation_limit: 0` remains invalid; the repair is naming
that field. Browser-scroll argument-shape errors remain caller-side evidence,
not a Codanna parser or MCP failure. No real transcript becomes fixture content.

## Source-confirmed gaps requiring runtime reproductions

Reviewed source: `ddb5ae61a72938d82cceaf42123dc7a88bfe3417`.
These are source-level findings, not newly executed Rust regression results.

### Arrow-function ownership in JSX

`TypeScriptParser::extract_jsx_uses_recursive` recognizes an `arrow_function`
but asks that node for a `name`; it does not recover the binding from the parent
variable declarator. Its fallback inherits the previous function context. A
fixture must cover both a top-level arrow component and a nested, separately
named arrow component, not merely another function declaration:

```tsx
export const CalendarPage = () => <Calendar />;

export function Shell() {
  const LocalPanel = () => <Calendar />;
  return <LocalPanel />;
}
```

Assert exact owner identity and range at the parser stage, then the resolved
`Uses` target and reverse impact at the pipeline/MCP stages. Do not substitute
a global name match or silently attribute every nested component to `Shell`.

### Lowercase namespace/member components

The JSX extractor admits names only when the first character is uppercase.
That predicate is too coarse for member expressions: `<ui.Calendar />` and
`<motion.div />` need different treatment from intrinsic `<div />`. Motion's
[official component documentation](https://motion.dev/docs/react-motion-component)
provides a real-world example of the latter member-expression syntax.

Use a local namespace import fixture, with both `ui.Calendar` and `UI.Calendar`,
a lowercase intrinsic, and a same-named unrelated `Calendar` in another file.
Require member/import-aware resolution; do not broaden the uppercase predicate
into accepting all lowercase intrinsic tags. No Motion dependency is necessary.

### Weak JSX oracles

`tests/parsers/typescript/test_jsx_uses.rs` checks that uses exist and that a
`Button` appears, but the main multi-owner case does not assert the exact owner,
source line, edge count, or absence of unrelated edges. A wrong owner can satisfy
those checks. Strengthen set equality before expanding the feature matrix.
Existing path-alias and lifecycle suites should be reused, not described as absent.

### Edge hydration can hide missing symbol documents

`IndexFacade::graph_neighbors` obtains relationship endpoints, loads symbols,
and uses `filter_map` to omit endpoints without a hydrated symbol document.
Therefore zero returned neighbors is not necessarily zero stored edges. Add a
fixture that deliberately creates a dangling endpoint and distinguishes raw
edges, hydrated neighbors, and storage errors. A future response may need an
unhydrated-edge count or partial-evidence status; do not claim that this is
already implemented by the new unknown-coverage metadata.

## Additional adversarial fixture matrix

Every positive case needs a nearby negative control. Expectations below are
proposed acceptance criteria, not assertions that the current implementation
already passes them.

| Priority | Fixture | Required oracle or invariant |
| --- | --- | --- |
| P0 | Direct, assigned-arrow, nested-arrow and class-render JSX owners | Exact owner, `Uses` target, source range; JSX must not invent an immediate `Calls` edge. |
| P0 | Named/default imports, renamed aliases, barrels, namespace members | Resolve the imported definition; forbid the same-named archive/peer definition. |
| P0 | Local shadowing of an imported component or callback | Nearest valid binding wins; an unresolved binding must not fall back to an arbitrary global namesake. |
| P0 | Persist, reopen, edit import, delete target, then reindex | Same logical query yields current-generation evidence; stale IDs and old edges must not leak. Test watch parity separately. |
| P1 | `import type` and `export type` beside equivalent value imports | Preserve type evidence without turning erased declarations into runtime calls. Mark deliberately invalid TS separately. |
| P1 | Namespace method calls with external receiver plus local namesake | External/unresolved state stays explicit; no fabricated local call merely because method names match. |
| P1 | Go value/pointer receivers, embedding and interface dispatch | Distinguish concrete method resolution from possible interface targets; avoid name-only cross-type binding. |
| P1 | Dangling edge, unreadable index, node/edge budget boundaries | Empty, unavailable, partial/hydration loss, and exhausted-budget outcomes remain distinct. |
| P1 | Active, archived, generated, test and peer files competing at top-k | Measure expected and forbidden hits before changing boosts; use frozen tuning/holdout queries. |
| P1 | Source order, file order, extra unrelated decoys and repeated runs | Relevant identities/edges remain stable under irrelevant changes; do not compare historical numeric IDs. |
| P2 | Duplicate/ineligible/stale vectors in the embedding population | Report eligible unique symbols and generation explicitly; raw vector/document counts are not recall quality. |
| P2 | Disabled recall, unavailable executable, malformed output, timeout and empty success | Separate outcomes with mocked local processes; no discovery of private transcript roots. |

## Language references

[TypeScript JSX](https://www.typescriptlang.org/docs/handbook/jsx.html) explains
intrinsic versus value-based elements and in-scope lookup. Its simple-name
capitalization convention is not a sufficient parser rule for member expressions.
[TypeScript type-only imports](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-8.html)
are erased from runtime output. The [Go specification](https://go.dev/ref/spec#Method_sets)
defines method sets and promoted methods; fixtures should reflect those rules
rather than infer dispatch from matching method names.

## Suggested next batch

Freeze the assigned-arrow and lowercase-namespace JSX inputs first. Diagnose the
boundary in order: syntax extraction, owner attribution, import resolution,
persistence, reverse graph, public MCP. Fix only the first failing layer, retain
negative controls, and then add reopen/incremental parity. Ranking experiments
should follow their independent T01/T05 baseline rather than piggyback on a
parser or response-wording repair.
