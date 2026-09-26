# Loop-local JSX and callback binding evidence

Implementation: [PR #48](https://github.com/bpstr/codanna/pull/48), stacked on
[JSX ownership PR #46](https://github.com/bpstr/codanna/pull/46).
This is a focused continuation of T03 in [PR #43](https://github.com/bpstr/codanna/pull/43).
It changes reference extraction, not ranking or semantic embedding input.

## Reproduced failure

```tsx
import * as ui from './provider';

export function Loop(items) {
  for (const ui of items) {
    consume(<ui.Calendar />);
  }
  return null;
}

export function Real() {
  return <ui.Calendar />;
}
```

Before the fix, `Loop` incorrectly acquired a persisted Uses edge to the imported
provider's Calendar. The local loop value does not establish that export identity.
`Real` legitimately uses the imported provider and must retain its edge. Adding
another file that exports Calendar must not change either result.

The same gap affected a bare `<Calendar />` whose name was rebound by a loop and
bare callback arguments inside JS/TS loops. These are false-positive dependency
findings, not merely missing output fields. The fixtures distinguish suppression
inside the lexical loop from correct imported/global references outside it.

## Repair

The shared unresolved-binding guard now inspects declarations in loop headers.
Tree-sitter represents `for-in`, `for-of`, and `for-await-of` using a
`for_in_statement` node with separate `kind`, `left`, and `right` fields. Classic
`for` declarations use the `initializer` field. The guard considers only binding
patterns, not the iterable or initialization expressions. It walks ancestors;
a sibling loop does not shadow an unrelated reference.

Binding-pattern traversal treats object-property keys and default expressions
as values rather than additional declarations. Thus `{namespace: ui}` binds ui,
whereas `{ui: renamed}` and `{value = ui}` do not. This is shared by JSX Uses
and callback References extraction; it does not convert either into Calls.

No global name fallback is introduced for dynamic loop-local values. A future
flow-sensitive resolver could establish their real target; this narrow repair
instead avoids claiming the imported target without that evidence.

## Executed evidence

[Baseline run 35669276324](https://github.com/bpstr/codanna/actions/runs/35669276324)
executed the five new test groups at branch revision
`73cfeb3c2be1f5e82b6195d1997d2a3e728db3ed`, merge checkout
`65e0ca1d9b502bf62e1451ce768fbf629bbe4ad2`: **1 passed, 4 failed**.
The persisted-graph test observed the incorrect provider edge. The parser tests
observed extra inside-loop references. Parameterized tests stop at their first
failure, so later baseline variants are not claimed to have been reached.
The baseline formatting check also failed; that is separate from these executed
behavioral failures. The repository formatter then formatted the test source.

[Candidate run 35669650945](https://github.com/bpstr/codanna/actions/runs/35669650945)
executed runtime repair `459d3e623655ddb29548a48839284229d797839c`, merge checkout
`644046a1f8a0b9b3e84a145ae920f653d33e9b25`: **5 passed, 0 failed**.
Nine existing JSX-impact tests, six source-column tests, and five callback
reference tests also passed: **25 passing tests across the four targets**.
Formatting passed. The test artifact records the source hashes below; the PR
records subsequent lint and workflow completion separately.

| Recorded input | SHA-256 |
| --- | --- |
| Baseline references source | `e40afa35b36d3582e3038f7d4cc342783a0d38c113a653e279e40aa7f0cd6afc` |
| Candidate references source | `4eedea3004934eddef83e33aa2ba1371041ed107e0acc4b609acb7d53db37ac4` |
| Baseline test source before formatting | `4c9482ce7ee10463a3c0d099436abe4299fc25f505b571aa42ec6e490313673b` |
| Candidate formatted test source | `60d1fb6823f392bf9a283af6b7b616a3dac6310a75d570426cf4a6d57e27bfe1` |

The expected cases were not relaxed between executions. The source inputs and
assertions are the same; the test-file byte change is formatting. These are
actual GitHub CI tests on Rust 1.98.1 / Linux x86-64, not claimed local Rust runs.
Only synthetic files enter disposable indexes, with semantic indexing disabled.
No model, paid provider, private transcript, or production index is involved.

## Fixture coverage and boundaries

The five groups cover let/const loop bindings, object/array destructuring,
await-of, bare and namespace JSX, both JS and TS callback extraction, property
key/default/initializer controls, exact outside-loop source positions, and
persisted Uses after save/reopen and loop-header edits. Restoring the original
shadowed header must remove the edge again. A same-named decoy provider stays
unconnected and no immediate Calendar Calls edge is allowed.

The for-in component-access case is deliberately unresolved negative syntax;
this corpus does not claim every negative case is accepted by a TypeScript type
checker. Function-scoped var hoisting beyond the loop, arbitrary assignments,
flow-sensitive local values, actual watcher events, peer-workspace conflicts,
and all transports remain separate coverage. Do not infer those guarantees from
these loop-local tests or close all of T03.

References: [TypeScript lexical declarations](https://www.typescriptlang.org/docs/handbook/variable-declarations)
and [Tree-sitter JavaScript grammar](https://github.com/tree-sitter/tree-sitter-javascript/blob/v0.25.0/grammar.js).
