# Calendar graph contract results

## Pre-change baseline

[Focused CI run 35664366650](https://github.com/bpstr/codanna/actions/runs/35664366650)
executed the six public MCP regression tests on branch head
`91953acc05566d0df1f9744a25a1646c8873aefd`, with GitHub's merge checkout
`57a8a11c5e48fb2448ca8bb841642ce1605e14ae`. Runtime source was unchanged from
`ddb5ae61a72938d82cceaf42123dc7a88bfe3417`.

Result: **3 passed, 3 failed, 0 ignored**. The isolated/external calls,
active-definition empty callers/impact, and positive-call/depth tests failed at
the new structured graph evidence assertion. Missing/ambiguous lookup, budget
failure, and frozen-oracle controls passed. An early assertion failure does not
prove that every subsequent assertion in the same test was reached.

This baseline proves that the new response contract was absent; it is not a
measurement of parser recall, lexical ranking, or semantic quality. The original
absolute empty-response wording was independently confirmed by reading the
pinned runtime source. No raw historical Assign response has been invented.

| Input | SHA-256 |
| --- | --- |
| `src/mcp/tools/symbols.rs` | `c872de4d2e887e4d7f3b2b5c2f81da41717206bc0c208275e6b30e8e9d15f1bd` |
| `tests/calendar_graph_evidence.rs` | `6d63f800ddc56348b616d63ad24aaa93d1b9c529623bd29bcd01eefbb2f27e2f` |
| `cases.json` | `ff70add6eed4ab621f22fad74182e1370c5b0592df69981752f1cf3af35114d4` |

Source fixture hashes match `cases.json`. Toolchain: Rust 1.98.1,
`x86_64-unknown-linux-gnu`; semantic indexing disabled. The log records a
pre-run test executable digest, but the baseline runner invoked Cargo again
for execution and Cargo rebuilt the target. Therefore that digest is not
asserted to identify the exact executed bytes. Index generation was not exposed
by these tool responses and remains unknown.

Earlier formatting and test-construction compilation failures were harness
errors, not successful reproductions of a Codanna graph defect.

## Runtime change

Successful `get_calls`, `find_callers`, and `analyze_impact` responses retain
text and add `structuredContent.graph`, schema version 1. It identifies the
operation, current target ID/path/line, result count, depth, and successful query
status. Its scope is explicitly `resolved_indexed_relationships`; source
coverage and freshness are unknown, and generation is null rather than guessed.

An empty successful query is not a statement that source code has no calls or
that a change has no effect. Positive impact wording also describes indexed
dependents rather than guaranteed effects. Missing and ambiguous lookups,
validation errors, and exhausted graph budgets retain their separate paths.

Post-change execution results are pending; no passing result is claimed in
this checkpoint. Full T01 corpus coverage, JSX resolution, ranking ablation,
semantic eligibility, routing, and lifecycle verification remain separate tasks.
