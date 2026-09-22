# Symbol scope verification checkpoint

Date: 2026-09-22. PR #53 is stacked on #52 and remains a draft.
This checkpoint records executed GitHub CI, not local Rust runs or a production
index rebuild. Sources/HOME/config/indexes are disposable; inference is disabled.

## Frozen public-boundary baseline

Run [35712442454](https://github.com/bpstr/codanna/actions/runs/35712442454),
head `0cb02df4198a0289563d8179682cae5908c942e5`, merge checkout
`26492f432f688d44fe90b82e4089befae5fa1f98`: **5 passed / 2 failed**.

The seven-test fixture SHA-256 was
`8ffb667b894da1a78e19672c0aba0d296ed4a0157975c979b57dfcb7922a03d9`.
That fixture hash is unchanged in the passing runs below.

Two genuine failures were reproduced after correcting the earlier fixture's
workspace-root assumption:

1. Invalid `code_path_prefix` became a successful code-unavailable section,
   then continued to document/recall sections.
2. Incremental indexing from a process outside the workspace read a stored
   `src/...` key against process cwd and failed with a missing-file error.

The original fixture omitted `src/` even though its workspace root was the parent
of `src`. That was a test-oracle error, not evidence that scope filtering should
silently retry paths against arbitrary indexed roots. The corrected baseline
keeps the wrong-prefix case as an explicit no-match control.

## Repairs

`fd9161ecbac450d12d4864431e363f64923fb1c6` returns invalid context scopes as
MCP tool errors before other source retrieval. Other unavailable-source errors
remain separate.

The READ and discovery changes preserve relative storage keys while resolving
filesystem reads against the configured workspace. This covers unchanged/modified
content checks, rename hashing, and serial/parallel source reads. Existing
regular-file and byte-limit guards remain in place. No global cwd mutation is
used to make tests pass.

`2e406850b4d5d3d69a310a86dc5724682d0c87e1` admits the new scope fields in
the MCP CLI catalog. `9d35a6075bafe8605bc67970d72029fb16b5814e` applies scope
in JSON search collection, validates typed requests before dispatch, preserves
context error envelopes, and makes text tool failures exit unsuccessfully.

## Executed results

Run [35715537710](https://github.com/bpstr/codanna/actions/runs/35715537710),
head `9d35a6075bafe8605bc67970d72029fb16b5814e`, merge checkout
`412c8a4c5be19c388bcfbdb5689ba20c40b98f17`: all **21 selected tests passed**.
This run did not yet execute the separate CLI target. Its later formatting gate
failed; behavioral passes are not presented as an overall green workflow.

Run [35716198996](https://github.com/bpstr/codanna/actions/runs/35716198996),
head `fd346a2027ae7ce33889d1fb9da80c620f88710c`, merge checkout
`8ed32fba4b56592711b6809db7252920ac5a64fd`: all **25 selected tests passed**:

| Group | Passed | Failed |
| --- | ---: | ---: |
| Direct facade/MCP scope, boundaries, lifecycle and errors | 7 | 0 |
| Real CLI text/JSON, retrieve, invalid/missing scopes | 4 | 0 |
| READ unit contracts, including byte limits and cwd independence | 8 | 0 |
| Workspace-relative incremental modification/rename | 1 | 0 |
| Existing incremental timestamp cases | 2 | 0 |
| Existing ambiguous/unique rename pairing | 3 | 0 |

The CLI target checks actual JSON contents and exit status; merely including a
scope in a request schema is not treated as implementation coverage.

| Tested source | SHA-256 |
| --- | --- |
| `src/storage/tantivy/query.rs` | `cfc42eb6d2714a1b09d1b15325ab1b7b4dab482dee8a47280af27c8d5f077c36` |
| `src/indexing/pipeline/stages/discover.rs` | `92a0652d7a884bffed94ca6895811e2d4b5061cd0e8f2ae5a0ba581feb6fec17` |
| `src/indexing/pipeline/stages/read.rs` | `8560afa58236eb3e276bf4d8f09f6ad0ac497e1f32737f24acc2d29058ffdf6f` |
| `src/mcp/tools/context.rs` | `f2d4a513a13c66fde87672d212424c2df6feddd1b57ec7985275ecc3a9030df2` |
| `src/cli/commands/mcp.rs` | `19639d932010dead01c97e157e9be9e8c54b753fc633808e5bff3253804d7da4` |
| `tests/symbol_scope_cli.rs` | `2b34b47365da084b124990bcda05ad9f1bc51670cc554d41b248b0f7d5a88ef5` |

Rust 1.98.1, x86_64-unknown-linux-gnu. Logs are retained in each run's
`symbol-path-scope` artifact. No executed-binary digest is claimed here.

## Remaining release gates

Current-head formatting, strict Clippy and wider/combined release checks must be
verified separately from the pinned behavioral runs above. Formatter commits are
preserved; a later commit is not automatically covered by an earlier green run.

T07 still needs large-index scope-scan costs, concurrent replacement snapshot
consistency, and broader external-root/workspace-alias controls. Exact lookup
remains independent of discovery scope; no archive/reference preference or
source-diversity heuristic was silently added. This is not completion of all
fourteen PR #43 task groups, and no branch was merged or force-pushed.
