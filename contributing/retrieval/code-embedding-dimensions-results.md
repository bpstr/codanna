# Code embedding dimension verification

## Pinned baseline and failure reproduction

Base: #64 at `3496ba1fbf24e5ec8be531a653204c6e9a219327`.
First committed fixture run:
[35781934800](https://github.com/bpstr/codanna/actions/runs/35781934800),
source `2226d36475b3a52b95245ad1f86599802b191d0d`.

Seven process/persistence contracts compiled and executed: **2 passed, 5 failed,
0 ignored**. The supported 1/4096 journal and 4097-dimensional generic document
backend controls passed. Rejection contracts found unsupported code dimensions
accepted by CLI/planner paths and filesystem artifacts created by a failed first
save. Assertions following an earlier failure were not reached; preservation and
probe-count claims below belong to the candidate execution, not that baseline.

## Executed candidate

[35782534256](https://github.com/bpstr/codanna/actions/runs/35782534256),
source checkpoint `1e767ddea99c094dbc7c49d7ff3b4a421d329f9b`, first applied
formatting to the fixture, reproduced **2 passed / 5 failed**, then applied only
the reviewed recipe to pinned runtime Git blobs. The same normalized fixture
SHA-256 was checked before and after:

`d5f884788a019621eab852272e78598d94344402dd1c7667a860af6dc9dc2b14`

Candidate result: **117 passed, 0 failed**, with **one pre-existing ignored
configuration test** (`config::tests::test_layered_config`, which changes process
CWD). No new regression is ignored, retried or weakened. Formatting and strict
all-target/all-feature Clippy passed. Rust 1.98.1, x86_64-unknown-linux-gnu.

| Selected group | Passed | Ignored |
| --- | ---: | ---: |
| New dimension CLI/backend/persistence contracts | 7 | 0 |
| Existing comment/body rebuild reuse CLI | 11 | 0 |
| Existing comment planner CLI | 12 | 0 |
| Existing body planner CLI | 5 | 0 |
| New shared code-dimension boundary unit contract | 1 | 0 |
| Planner/runtime input parity | 5 | 0 |
| Symbol representation contracts | 16 | 0 |
| Semantic journal contracts | 5 | 0 |
| Configuration contracts | 55 | 1 |

### Asserted behavior

- Configured dimensions 0, 4097 and 16384 are rejected before any provider
  request or destructive force setup, for comment and body modes.
- Unknown remote dimension 4097 produces one initialization probe and **zero
  source inputs**. The pre-existing index retains identical file bytes,
  directory membership and modification times.
- Invalid environment overrides are rejected; a valid override of an invalid
  file setting preserves cache reuse and sends only the ordinary probe.
- Planner rejection leaves the complete workspace unchanged. Invalid first
  indexing creates no index; explicitly disabled semantic indexing remains off.
- Unsupported first and replacement saves fail before new artifacts; a stored
  invalid manifest is not rewritten to make it readable.
- Dimensions 1 and 4096 retain checkpoint, delta and reopen behavior.
- The shared generic document backend still accepts the separate 4097-dimensional
  mock input. This does not assert a universal document storage maximum.

The existing frozen-body input summary stayed 102 eligible body parents, 117
prepared inputs and 116,547 UTF-8 prepared bytes, versus 37 comment inputs and
4,423 bytes. Provider tokens and semantic relevance were not measured. Mock
initialization probes and source input items are not billing estimates.

## Published source provenance

Each candidate source file was archived, independently rehashed after download,
and matched to the immutable Git blob returned by GitHub. Only the verified
source blobs are included in the runtime commit; no branch update is performed
by the verifier.

| File | SHA-256 |
| --- | --- |
| `src/semantic/mod.rs` | `ad3ea3a88622ea4fb2f8cb83fca0a34f3a9e3afcd9e24cff9370b8df8405bf85` |
| `src/semantic/code_dimension.rs` | `98071623380fee7be9754061ab12728966b102bf78695b255548eecba3041c4a` |
| `src/semantic/journal.rs` | `25c23770f7a945116ef2d2f985fe9706cf57aea8383b01e3c6b41e165538200f` |
| `src/indexing/facade.rs` | `960d686ddfec467932dab7232ba286ea298abf90daf793b2c1c3844cfa96e264` |
| `src/main.rs` | `3674eb82888bebb98768af2f5166d6d66fb907ef631846765cd0e764af37eb04` |
| `src/rebuild_plan.rs` | `2cda09d69a7464285be3a7587a79c34339f3e309bf6a38ca243016777efafcfc` |
| `src/indexing/pipeline/stages/semantic_embed.rs` | `1c06a7134b344b203fbbfc79a92e00fea2cf33f28b40d1cd098264a860076c5c` |
| `tests/code_dimension_contract.rs` | `d5f884788a019621eab852272e78598d94344402dd1c7667a860af6dc9dc2b14` |

## Final committed-source checks

The temporary recipe and contents-write candidate workflow are removed when the
verified runtime is committed. Retained `code-dimension-contracts.yml` is
read-only and runs the committed source directly. It repeats the selected
contracts above and adds existing CLI force-path and emission-version gates.
These additional gate results and the overall final-head workflow are pending
at this publication checkpoint, and must not be inferred from the staged pass.

This is a code-format/cost-safety slice, not an all-branches release approval.
No production index, paid inference, automatic body-policy opt-in, main/upstream
merge or unsupported-store migration is performed. #63 approval-required checks
remain untouched. Independent relevance evaluation and the combined lexical/
scope/body release remain separate work.
