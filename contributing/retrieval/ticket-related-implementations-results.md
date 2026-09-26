# Related implementation evidence: executed results

## What changed, and what did not

This batch is stacked on #56 at `87a5ae381540147c1049c348f55d30f82c585fa8`.
An explicit `include_related_code: true` adds one-hop resolved indexed Calls as
separate related evidence. Default responses and all direct code items remain
unchanged. No ranking score, owner boost, provider request, or index rebuild was
introduced. Scoped requests do not traverse until graph filtering is qualified.

This baseline does not contain the separately implemented #59 lexical stemmer
or #57 scope snapshot changes. Independent branch improvements must not be added
together as if a combined build had been evaluated.

## Frozen relevance result

Same five unmodified Rust modules and 152 symbols, ten owner judgments, and two
phrasings per task as #58. The oracle SHA-256 remains
`129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`.
Every owner was independently resolved by exact path/name before measurement.
The full direct `code.items` arrays are byte-equivalent JSON before/after opt-in
for every question, including scores, contribution evidence and ordering.

| Evidence surface | Operational | Paraphrase | Combined |
| --- | ---: | ---: | ---: |
| Direct owner Hit@5, disabled and enabled | 5/10 | 1/10 | 6/20 |
| Owner anywhere in up to 5 direct + 6 related | 8/10 | 1/10 | 9/20 |

The second row has a larger evidence budget. It is **not** an improvement in
Hit@5, not a ranked top-11 claim, and not completion of the original 90% retrieval
quality target. These already-inspected questions are not a fresh holdout.

Recovered operational witnesses:

| Owner | Direct seed | Direct seed rank | Related rank | Stored call-site line |
| --- | --- | ---: | ---: | ---: |
| `collect_all_files` | `run_incremental` | 3 | 4 | 184 |
| `capture` | `conversation_context` | 1 | 2 | 78 |
| `render` | `hardening_workspace_recall_rejects_foreign_reply_before_rendering_text` | 1 | 1 | 229 |

Each row is supported by an existing indexed Calls edge and current indexed
source/target identities. The test helper is not excluded merely because it is
a test: its call is useful provenance, not proof that it owns the implementation.
Related rank is deterministic seed/location order, not semantic relevance.
All source positions refer to the frozen fixture, not a live Assign checkout.

No additional paraphrase owner was recovered. Empty direct candidate pools have
no seeds and cannot be repaired by following edges. More body input in #55/#60
or rank fusion in #56 is not assumed to solve that remaining vocabulary gap.

## Actual test execution

[Run 35747142990](https://github.com/bpstr/codanna/actions/runs/35747142990)
checked out `104f54648542be31dfcbe8136095e473c29c7f0b`, verified exact original
integration blobs, applied the reviewed wiring and formatting, and executed the
runtime/test blobs now committed in this source checkpoint.

| Selected target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| New related-code library/schema/handler contracts | 7 | 0 | 0 |
| New actual CLI text/JSON/scope/error contracts | 2 | 0 | 0 |
| New 20-query handler relevance/compatibility measurement | 1 | 0 | 0 |
| Existing ticket-code fusion contracts | 14 | 0 | 0 |
| Existing facade/MCP scope contracts | 7 | 0 | 0 |
| Existing scope CLI contracts | 4 | 0 | 0 |
| Existing workspace MCP ticket client contracts | 2 | 0 | 0 |
| Existing tool catalog/schema/guidance parity | 1 | 0 | 0 |
| Total | **38** | **0** | **0** |

Strict all-target/all-feature Clippy and final formatting checks passed. A
nonzero selected-test guard prevents an empty filtered library run from being
called acceptance. The new tests use disposable indexes, fixed local Calls,
and isolated CLI environments; no embedding provider, model, transcript import
or production workspace is used. Document indexing is disabled in the fixtures.

The first staged run `35746632821` stopped at compilation because `SymbolId`
does not implement `Ord`. The correction sorts/deduplicates by numeric IDs;
that failure is not a before/after behavioral defect reproduction.

The staged workflow published only immutable Git blobs, checked against the
locally computed Git IDs. Final branch updates use the connected GitHub app.
Temporary wiring/workflow files have been removed. The retained workflow is
read-only and reruns committed sources directly, without applying any patch.
A final-head rerun and combined-release qualification are separate from this
staged execution record.

## Tested source identities

| Path | Git blob | SHA-256 |
| --- | --- | --- |
| `src/mcp/tools/ticket_context.rs` | `1a25aa31d2802509193a85d7362bc2e2c08db3e5` | `52bd0a0e7ca27254ff41eb859698c060224859b94e2e16a3387c4d0543047bef` |
| `src/mcp/tools/ticket_related.rs` | `2374167ecb1e9e870f85c2ec92f57b92fd229e48` | `71bf65644b266510828da4e4b39a61609569e04728127808ae7f84d1f3d912a1` |
| `src/mcp/tools/ticket_related_tests.rs` | `1fbb4b79e7a7e5b605328ad7f4c39d6b1f00b079` | `3a5bf79128f66b59817466f44ff42e7c456e7a9a0c59711b3e5218aaad1acb6e` |
| `tests/ticket_related_cli.rs` | `72ff67140ec829df9f498ee7c3c6c8f17a087065` | `a33571932e42b786463be31183642cecd7677f4c577126e83506a1a1fa0ba4da` |
| `tests/ticket_related_relevance.rs` | `6ee0a3cbf89960b5e52d8ee37cf769c8f51e1bd9` | `3d471e548702fadf8273e5361bcb516adf3cf931e8b2b27168febb161cbcae9d` |

## Remaining boundaries

Scoped graph expansion is explicitly unsupported, not silently broadened.
Only outgoing Calls, one hop and three seeds are traversed; no data-flow,
callback-reference, JSX-Uses or semantic relatedness claim follows. Missing
target documents and budget errors stay visible. Reader generations are
instance-local observations, not persisted code/vector freshness stamps.
Independent larger labels, controlled cold/warm latency/RSS, scoped snapshot
integration, and semantic relevance evaluation remain open. No merge into main,
upstream modification, paid inference or production reindex was performed.
