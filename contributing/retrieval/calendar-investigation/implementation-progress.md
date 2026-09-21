# Calendar investigation implementation progress

Snapshot: 2026-09-22 (Europe/Budapest). This is a checkpoint of two small
implementation batches, **not completion of all fourteen task groups**.
All writes are confined to `bpstr/codanna`; upstream has not been modified.

## Implemented batches

| Batch | Pull request / branch | Verified head | Scope |
| --- | --- | --- | --- |
| Graph evidence | [#44](https://github.com/bpstr/codanna/pull/44), `fix/calendar-graph-evidence` | `4795f45ff7aad053b34e4d5eb1a60c0e5e5b0ba7` | T01 graph-contract fixture subset; T02 successful graph response wording and additive evidence metadata. |
| Context limits | [#45](https://github.com/bpstr/codanna/pull/45), `fix/context-limit-diagnostics` | `ae44c713c1dc5c7f877237c3652d88e74cf4fb74` | T09 field-aware argument diagnostics shared by deserialization and direct handler validation. |

Both PRs are independent draft PRs against main revision
`ddb5ae61a72938d82cceaf42123dc7a88bfe3417`. Neither has been merged.

### Graph responses

`get_calls`, `find_callers`, and `analyze_impact` retain readable text and add
`structuredContent.graph`. Empty results now mean no **resolved indexed**
relationships were found, not that source code has no calls or a change has no
effect. Positive impact wording likewise describes indexed dependents.

The evidence includes operation, current target identity/path/line, result
count, requested depth, and completed query status. Source coverage and
freshness are explicitly unknown; index generation is null rather than guessed.
Missing/ambiguous lookups and budget errors retain separate response paths.
No parser, import resolver, ranking, or storage algorithm was changed.

The synthetic corpus separates isolated, external-browser-call, local call-chain,
and unrelated same-named reference symbols. An oracle outside the indexed root
selects symbols by path/name/kind and resolves IDs from the newly created index.
The focused runner verifies fixture hashes, compiles once, records the executable
hash, and executes those exact bytes without invoking Cargo a second time.

### Context request diagnostics

The original `conversation_limit: 0` request remains invalid. Its diagnostic now
names `conversation_limit`, states the accepted integer range, and includes the
received out-of-range value. The same treatment applies to `code_limit` and
`document_limit`, including wrong JSON types and overflow.

All three limits retain inclusive bounds 1 through 10 and omitted default 5.
Zero is not a section-disable switch. Unknown keys remain invalid. Direct Rust
calls use the same validator and reject bad limits before retrieval or recall.
Retrieval behavior and output sections are unchanged.

## Execution evidence

| Check | Graph batch | Context batch |
| --- | --- | --- |
| Focused regression tests | **6 passed, 0 failed, 0 ignored** | **5 passed, 0 failed, 0 ignored** |
| Quick Check | Passed | Passed |
| Repository auto-fix check | Passed | Passed |
| Hardening workflow | Passed | Still running at this snapshot |
| Review security regressions | Passed | Still running at this snapshot |
| Full Test Suite | Still running; no overall pass claimed | Still running; no overall pass claimed |

For the graph full-suite job, formatting, all-target/all-feature Clippy, and
no-default-features compilation had passed. Default-feature tests were running;
all-feature tests, CLI checks, and documentation gates had not all completed.
Workflow results are tied to the heads above, not to future commits or merges.

### Graph baseline and post-change run

[Baseline run 35664366650](https://github.com/bpstr/codanna/actions/runs/35664366650)
compiled and ran the new tests against unchanged runtime source. **Three passed
and three failed**. Failures occurred at the new graph metadata assertion;
subsequent assertions in those failing tests are not claimed as reached. The
old absolute wording was independently confirmed by reading the pinned source.
This is not a parser-recall or ranking-quality baseline.

[Post-change run 35664941685](https://github.com/bpstr/codanna/actions/runs/35664941685)
ran all six tests successfully. The actual GitHub merge checkout was
`9f95c09f6ab695916e85c5740184a69b5242f76b`, and the executed test binary SHA-256 was
`b2085352233d376c3f07fc8f821b3c8a91bf9e5efaf47eb839d7d0a69a90007a`.
The `calendar-graph-evidence` artifact contains its log and tested-source archive.

The baseline runner's earlier pre-run binary hash is not asserted to identify
executed bytes: a second Cargo invocation rebuilt the target. The post-change
runner fixes that provenance gap. Earlier formatter and fixture-construction
compilation failures were harness errors, not reproduced Codanna behavior.

### Context run

[Context run 35665415312](https://github.com/bpstr/codanna/actions/runs/35665415312)
ran all five tests successfully on merge checkout
`d6b7f34668d6cd8c25d4bd0f69af5f9567ef8fba`.
The `context-request-contracts` artifact records source hashes and the complete
compiler/test log. This runner does not claim an executed-binary digest.

Both focused runs used Rust 1.98.1 on x86_64-unknown-linux-gnu. Local Rust tooling
was unavailable; these are actual GitHub CI results, not claimed local runs.
The new fixtures disable semantic indexing and use no provider, private
transcript, or production index. Initial `not_run` fixture manifests and pending
notes in earlier commits are historical checkpoints; this document and the PR
bodies record subsequent execution without rewriting the frozen baseline.

## Small commit checkpoints

| Commit | Delivered change |
| --- | --- |
| `b307280` | Freeze graph corpus and independent oracle. |
| `b546153` | Add public MCP graph contract tests. |
| `91953ac` | Correct fixture construction and retain compiler diagnostics. |
| `e156aa7` | Implement truthful graph wording and evidence metadata. |
| `4795f45` | Verify fixture hashes and execute the recorded test binary. |
| `18737f0` | Add context argument/schema/direct-handler tests. |
| `8247018` | Implement shared field-aware context validation. |
| `ae44c71` | Run context contracts independently in CI. |
| `4d527ca` | Record source-level fixture gaps and adversarial follow-ups. |

Repository-generated formatting commits are preserved. No branch was force-pushed.

## Findings that must not be conflated

An empty indexed graph is incomplete evidence about source behavior, which is
why T02's response contract changed. In contrast, `conversation_limit: 0` was a
caller argument error under the existing valid bounds; T09 improves its
explanation rather than weakening validation. The accessibility-tree errors in
the intake came from browser-scroll argument shapes, not Codanna responses.
No new Codanna parser error is inferred from those browser failures.

## Remaining work

T01's complete lexical/semantic/routing/lifecycle acceptance corpus remains open.
T02 still needs broader evidence-state fixtures, including dangling endpoints
and hydration loss; the current unknown-coverage metadata does not implement
those diagnostics. T03/T04 parser and import/member resolution, T05/T06 measured
retrieval/ranking experiments, T07 workspace scope, T08 semantic eligibility,
T10 routing, and T11 lifecycle reconciliation remain open. The rest of T09
includes structured/text result parity and unavailable-versus-empty recall.
T12/T14 product/caller follow-ups remain outside these runtime changes. T13 has
partial publication/verification evidence here, not a completed release gate.

The [fixture follow-up review](fixture-followups.md) records the additional
source findings and proposed cases. Its recommended next batch is assigned-arrow
JSX ownership and lowercase namespace JSX members, with exact owner/range/edge
oracles, nearby same-name decoys, and a parser-to-persistence-to-MCP diagnosis.
Those source findings have not yet been turned into executed Rust regressions.
