# Retrieval, body representations and rebuild qualification

This #66 integration combines #62 at
`8b8f08c4bee87264665f0df02134089b7632b49f` and #65 at
`1f8cc2e4031c4ad167bd409414d15bda62827da9`. The clean merge was audited in
[run 35790623163](https://github.com/bpstr/codanna/actions/runs/35790623163).
Internal integration PR #67 merged only into the new qualification branch;
commit `2b7a0c61a33e7f80d051cf3cc137b0e57e32f5f2` preserves both histories.
Main, upstream and all original feature branches remain unchanged.

## Cross-feature process contracts

`tests/support/retrieval_body_cases.rs` extends the existing joined loopback
fixture used by `semantic_rebuild_reuse`. Each CLI child has a cleared environment,
a disposable HOME, synthetic source, and no provider credentials. Documents are
explicitly disabled; their independently configured backend is not under test.

Actual `codanna mcp search_ticket_context --args ... --json` processes verify:

- Lexical-only ticket requests do not initialize the configured embedding backend.
- Body semantic retrieval is opt-in. An explicitly enabled, unscoped request sends
  one initialization probe and one query input, never source inputs or a rebuild.
- Multiple body segments return each parent once before the requested limit.
- Scoped related implementations work on the same build as body persistence.
  Scoped semantic retrieval remains explicitly unsupported and contacts no backend.
- Missing scopes stay empty, with no global fallback or provider initialization.
- Changing source policy or corrupting saved semantic metadata leaves lexical
  results intact, marks semantic retrieval unavailable, and makes no provider call.
- Every ticket query preserves existing index files, directory membership and
  modification times. This is not an assertion that all possible query operations
  are free of side effects in every configuration.

The mock vectors deliberately distinguish inputs containing `beta`. They test
identity, segmentation, parent deduplication and request boundaries, **not model
quality**. Body policy remains opt-in and comment policy remains the default.

## Newly reproduced failed-reload bug

The combined `IndexFacade::load_semantic_search` retained a previous in-memory
semantic object when a later reload found corrupt or removed metadata. Generic
load failures only logged a warning. Consequently, the old object could still be
queried or republished despite the explicit reload having failed.

The repair marks that generation incompatible on generic load failure and when
its manifest disappears after an earlier successful load. Existing data remains
available for recovery, but the established query/save guards prohibit serving
or republishing it. A later valid reload restores normal use. An absent store on
first load is still optional; no new directory or model is created. Lexical
retrieval remains available throughout, and corrupt on-disk bytes are preserved.

`tests/semantic_reload_retention.rs` exercises these public facade/persistence
boundaries for both comment and body admission policies using fixed vectors.
No backend or inference is involved. The removed/corrupt state is not described
as a successfully empty semantic index.

## Executed verification and preserved failures

[Run 35791641833](https://github.com/bpstr/codanna/actions/runs/35791641833),
source checkpoint `aec3904bf5afc87f13bc57792f0d7be313537e1b`, applies only the
reviewed repair against the pinned facade blob. The same normalized three-test
fixture changed from **1 passed / 2 failed before to 3 passed / 0 failed after**.
Both failing baselines reported that old vectors remained usable. Assertions
after those failures are not claimed as reached in the baseline.

The candidate then passed **40 selected tests**, zero failures or ignored cases:
three reload tests, thirteen comment/body rebuild and cross-feature CLI tests,
seven dimension contracts, twelve comment-planner CLI tests and five body-planner
CLI tests. Formatting and strict all-target/all-feature Clippy passed.

[Combined run 35791641919](https://github.com/bpstr/codanna/actions/runs/35791641919)
previously passed **130 process/library selections** and the frozen outcome
checker before the reload repair. Its overall job did not pass: new reload-fixture
formatting remained and artifact upload rejected `::` in selection filenames.
The committed workflow now uses portable filenames without weakening the
nonzero-test checks. Those failed steps are not silently relabeled successful.

An earlier process fixture compared scoped and unscoped raw lexical scores.
Adding a scope contributes to the underlying query score independently of the
semantic/related flags. `aec3904` corrects only that oracle to compare flag off/on
within the identical scope. No production scorer was changed to satisfy the test.

## Relevance remains a separate quality gate

The same five frozen modules, 152 symbols, twenty questions and ten owner labels
retain oracle SHA-256
`129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`.
The combined run recorded:

| Evidence | Operational | Paraphrase | Combined |
| --- | ---: | ---: | ---: |
| Direct owner Hit@5 | 7/10 | 1/10 | 8/20 |
| Owner among five direct plus up to six related | 9/10 | 1/10 | 10/20 |

Direct MRR@5 is 0.267 combined. The larger related surface is not improved Hit@5.
All twenty direct arrays stay unchanged by the related opt-in, and that run
required zero generation retries. Questions have influenced selection and are
not an independent holdout. The original quality gate remains **false**.
Adding fixed body vectors does not establish a new semantic/paraphrase result.

## Committed-source provenance

The three published blobs were independently compared with the verified source
archive before the GitHub commit:

| File | SHA-256 |
| --- | --- |
| `src/indexing/facade.rs` | `0ff552b06c0662c058c7e42f8a0bf3cea95276417802092ef68461959e556675` |
| `tests/semantic_reload_retention.rs` | `8b1714effbd37a3fc37dd120a49d5382686846483832b892f6e543473ebb2b10` |
| `tests/support/retrieval_body_cases.rs` | `516f7176942c6d1ef95516c2797a0812ae85d227bb8d793dbd65cb4e6da09403` |

The one-time merge audit, pinned patch recipe and contents-write candidate
workflow are removed. Retained `retrieval-body-qualification.yml` runs committed
sources with read-only permissions. It adds the reload regressions to the combined
contracts and separately executes the repository `full-test.sh` gate. Final-head
results are recorded on #66 after execution; staged passes are not automatically
final-head or release passes. The repository formatter commit is preserved.

No production index was inspected/rebuilt, no paid model was called and no
embedding policy was silently enabled. The reload guard and retrieval integration
do not require re-embedding a valid existing index. Broader branch integration,
release packaging for the planner, controlled performance/RSS measurements and
independently judged semantic relevance remain open. #63 approval-required runs
were not approved or bypassed.
