# Offline rebuild preflight verification

Implementation and operating boundaries: [preflight guide](rebuild-cost-preflight.md).
Work is in [PR #49](https://github.com/bpstr/codanna/pull/49), stacked on #47.
No production workspace, provider credentials, model inference, or paid endpoint
was used by the new preflight and cache-pressure fixtures. Existing rebuild CLI
controls use a local synthetic HTTP server, not a real embedding service.

## Executed candidate

[Run 35672580830](https://github.com/bpstr/codanna/actions/runs/35672580830)
compiled and executed source head
`e496b0daff546d119e1b60c62c1a91c19a6de9d2`, GitHub merge checkout
`78473232f2a62a35983e2fc76e19c7cd3462d910`, on Rust 1.98.1 / Linux x86-64.
Its downloaded `index-plan-contracts` artifact includes complete test output and
tested sources.

| Target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `index_plan_cli` | 12 | 0 | 0 |
| `rebuild_plan_pressure` library tests | 3 | 0 | 0 |
| Existing `semantic_rebuild_reuse` CLI groups | 5 | 0 | 0 |

All **20 tests passed**. The workflow then stopped at formatting; strict lint
execution in that run was skipped. Do not label that workflow overall successful.
The repository formatter's `0b0fd5e` commit changes line wrapping only in the
planner and two fixture statements, and was preserved rather than overwritten.
The subsequently extended workflow also checks compilation of the standalone
planner without default features and its help command; those checks are not
claimed as executed by the earlier run.

| Tested source | SHA-256 |
| --- | --- |
| `src/rebuild_plan.rs` | `7f9785b4584be21e45982dcc67ff081c96b1ac195acc155fc2a7b6d307b6a70c` |
| `src/rebuild_plan_pressure.rs` | `fd65b76a8ac910a1caa987f9e2d7142c898fc300deeef1c205a96d48b3127db2` |
| `src/bin/codanna-index-plan.rs` | `1efaaf2a5decc2ac553aaa4af30be1431edf7a500f446e1c1d21affc2a4d8e6c` |
| `tests/index_plan_cli.rs` | `844756d2e9e093b76616ffca291739083ff1c5eab11048218e42319d51094d32` |

Hashes identify the tested source bytes, not a released binary or atomic index
generation. Formatted source hashes differ without changing test expectations.
Local Rust execution was unavailable; the results above are GitHub CI evidence.

## What the new contracts establish

The twelve subprocess tests check zero connections to the configured embedding
endpoint and unchanged workspace file bytes, directory membership and mtimes.
They cover absent/corrupt indexes, matching and changed source inputs, duplicate
inputs, overlapping roots, ignore rules, incompatible/corrupt caches, local and
remote-unknown identity states, disabled semantic search, rejected inputs,
non-regular optional files, malformed configuration and escaping source roots.
Mixed native Lua and generic Zig coverage remains explicit; tokenizer encoding
failure is classified as input-policy rejection, not a measured token overrun.

The three cache experiments reproduce 4,096 snapshot matches dropping to zero
reuse during a particular 4,500-input scan, and a 1,337-entry byte-limited cache
at dimension 3,072. They are fixed-vector cache-policy experiments, not real CLI
rebuild ordering, billed-token, memory RSS or model-quality measurements.
Snapshot matches remain opportunities, not promised savings.

## Earlier failures remain attributed

At `be659744`, the three pressure tests passed but CLI compilation failed in a
fixture-only SHA-256 formatting helper. That is not a Codanna runtime defect.
At `4120494`, nine CLI tests passed; the tenth incorrectly assumed Lua lacked a
native parser. It was corrected by retaining Lua and adding generic Zig, not by
weakening the partial-coverage requirement. The final twelve-test candidate adds
encoding-failure and non-regular-path controls. This is a new-command acceptance
suite, not a claim that the original code had this CLI and failed these tests.

## Previously pending full-suite checks

During this batch the following earlier full-suite runs were checked to their
successful completion. Each passed formatting, strict Clippy, no-default-feature
compilation, default/all-feature tests, CLI checks and the documentation build.

| PR / exact head | Successful full-suite run |
| --- | --- |
| #44 / `4795f45ff7aad053b34e4d5eb1a60c0e5e5b0ba7` | [35664941675](https://github.com/bpstr/codanna/actions/runs/35664941675) |
| #45 / `ae44c713c1dc5c7f877237c3652d88e74cf4fb74` | [35665415231](https://github.com/bpstr/codanna/actions/runs/35665415231) |
| #46 / `b570e1a72e6c301436b6ac6439269d6b6930f8b8` | [35667982611](https://github.com/bpstr/codanna/actions/runs/35667982611) |
| #47 / `23bb9b8594dc39689fb19bf77ee9b9a5ae9f3f75` | [35669945017](https://github.com/bpstr/codanna/actions/runs/35669945017) |

Tag-only release builds were skipped, not verified. Comments on each PR record
the updated observation. Independent branch passes do not establish a combined
release, complete T03/T09 coverage, deployment or real-index correctness. No PR
was merged as part of this verification. The final #49 checks are tracked on its
current head; the original #43 ranking, admission-policy, graph and lifecycle
follow-ups are not closed by these results.
