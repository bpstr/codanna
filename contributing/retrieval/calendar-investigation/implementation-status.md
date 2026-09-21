# Calendar investigation implementation status

Updated 2026-09-22. This is an implementation checkpoint, not a completion claim
for the full [action plan](action-plan.md). The investigation remains in PR #43;
runtime changes are split into focused PRs in the bpstr fork. Nothing is merged
or deployed by this checkpoint.

## Published batches

| Slice | Pull request / branch | Checkpoint |
| --- | --- | --- |
| T01 graph-contract corpus and T02 MCP wording/metadata | [#44](https://github.com/bpstr/codanna/pull/44), `fix/calendar-graph-evidence` | Implemented; six focused tests pass at `4795f45ff7aad053b34e4d5eb1a60c0e5e5b0ba7`. |
| T09 field-specific context-limit diagnostics | [#45](https://github.com/bpstr/codanna/pull/45), `fix/context-limit-diagnostics` | Implemented; Quick Check passes at `ae44c713c1dc5c7f877237c3652d88e74cf4fb74`. Focused runtime test result not yet observed in this checkpoint. |
| Fixture review and next investigations | [fixture follow-ups](fixture-followups.md) | Source findings and proposed regression matrix recorded; not runtime reproductions of JSX or ranking failures. |

Important graph commits: `b307280` freezes the synthetic corpus; `b546153` adds
public-handler regressions; `e156aa7` implements indexed-evidence output;
`4795f45` verifies the frozen hashes and executes the exact recorded test binary.
Formatting and harness corrections have their own commits. Request-validation
commits include `18737f0` for regression inputs, `8247018` for the shared validator,
and `ae44c71` for the focused CI job. Existing branch changes were preserved;
no forced ref update was used.

## Executed graph evidence

The [pre-change run](https://github.com/bpstr/codanna/actions/runs/35664366650)
ran the six tests with unchanged runtime source from `ddb5ae6`: **3 passed,
3 failed, 0 ignored**. The failures first reached the absent structured graph
contract. They do not prove that later assertions in those tests were reached.

The [post-change focused run](https://github.com/bpstr/codanna/actions/runs/35664941685)
used branch head `4795f45ff7aad053b34e4d5eb1a60c0e5e5b0ba7`, GitHub merge checkout
`9f95c09f6ab695916e85c5740184a69b5242f76b`, and Rust 1.98.1 on Linux x86-64:
**6 passed, 0 failed, 0 ignored**, test execution 0.68 seconds. That duration is
not a retrieval latency benchmark. Corpus and oracle hashes are unchanged.

| Recorded input | SHA-256 |
| --- | --- |
| Executed graph test binary | `b2085352233d376c3f07fc8f821b3c8a91bf9e5efaf47eb839d7d0a69a90007a` |
| MCP symbol tool source | `1a6fb847ac221cd7661c4fa5944822ef275be2278a5bdc19a6fcce6085406167` |
| Graph regression test source | `6d63f800ddc56348b616d63ad24aaa93d1b9c529623bd29bcd01eefbb2f27e2f` |
| External oracle | `ff70add6eed4ab621f22fad74182e1370c5b0592df69981752f1cf3af35114d4` |

The runner hashes the compiler-reported executable and invokes that binary
without another Cargo build. This closes the earlier harness provenance gap.
Semantic search is disabled; the fixtures do not use models, providers, private
transcripts, or production indexes. The tool response's index generation and
source coverage remain unknown; the binary hash does not make them known.

[Quick Check](https://github.com/bpstr/codanna/actions/runs/35664941715),
[Hardening](https://github.com/bpstr/codanna/actions/runs/35664941687), and
[Review security regressions](https://github.com/bpstr/codanna/actions/runs/35664941640)
passed on that head. The [full suite](https://github.com/bpstr/codanna/actions/runs/35664941675)
was still running when this checkpoint was recorded. Do not infer an all-green
merge gate from the focused pass.

## What remains open

T01 is partial: the current corpus is a graph-contract slice, not the required
multi-domain ranking/semantic/workspace evaluation. T02 is partial: successful
MCP graph replies now describe indexed evidence, but dangling endpoints,
unavailable storage, all transport surfaces, and CLI parity need dedicated
checks. T09 is partial: field diagnostics are implemented, while structured
result completeness, ANSI-free document previews, and source-status parity are
not completed by this change. T03–T08 and T10–T14 are not closed.

The next graph investigation should follow [the fixture review](fixture-followups.md):
assigned and nested arrow ownership; lowercase namespace/member JSX versus
intrinsic tags; alias/barrel identity; then persistence and incremental parity.
The extractor's source-level ownership gap is a lead, not proof of the exact
failure stage in the private Assign index.

One additional harness improvement is to execute cases directly from the JSON
oracle. The current test harness uses its selectors but manually implements the
case assertions; a count check alone cannot prevent case/expectation drift.
Require every case ID to execute exactly once, reject unknown expectation types,
and compare exact target identities and forbidden edges. Preserve frozen inputs
while recording runs in a separate results file. These are proposed follow-ups,
not capabilities already provided by the six passing tests.
