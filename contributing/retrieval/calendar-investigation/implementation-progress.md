# Calendar investigation implementation progress

Updated 2026-09-23 (Europe/Budapest). This is a navigation checkpoint for
[PR #43](https://github.com/bpstr/codanna/pull/43), **not completion of all fourteen
task groups**. Implementations remain on the fork's feature/integration branches.
No main/upstream merge, deployment, production rebuild or paid inference is
claimed. Integration-only merges preserve dependency histories on new branches.

The [original two-batch record](https://github.com/bpstr/codanna/blob/a5661ffd51a87a8ad785261829d567a884f71f19/contributing/retrieval/calendar-investigation/implementation-progress.md)
preserves the first #44/#45 execution history, exact hashes and then-pending gates.
Those historical pending statuses must not be read as the current status of later
integration branches. Each PR's current-head CI remains authoritative.

## Current integration checkpoints

| Slice | Committed head | Observed verification and boundary |
| --- | --- | --- |
| [#62: lexical recovery, workspace scopes and related implementations](https://github.com/bpstr/codanna/pull/62) | `8b8f08c4bee87264665f0df02134089b7632b49f` | Combined source has 66 selected passing contracts. Final-source workflow [35768636082](https://github.com/bpstr/codanna/actions/runs/35768636082) passed. Direct owner Hit@5 is 8/20; five direct plus up to six related results expose 10/20. Different evidence budgets, not additive quality gains. |
| [#64: body-aware planner and rebuild cache reuse](https://github.com/bpstr/codanna/pull/64) | `3496ba1fbf24e5ec8be531a653204c6e9a219327` | Current-head Quick Check, representation and body-cache workflows passed. Integrates #63/#60 with #50/#47 while preserving histories. Controlled body-pressure rebuilds send 272 missing source inputs instead of 1280; repeated seven-segment input is embedded as three unique texts, without losing seven source-range records. Not a bill or relevance score. |
| [#65: code dimension rejection before inference and publication](https://github.com/bpstr/codanna/pull/65) | `1f8cc2e4031c4ad167bd409414d15bda62827da9` | Final-source [35783603865](https://github.com/bpstr/codanna/actions/runs/35783603865) passed 128 selected tests; one pre-existing CWD-sensitive configuration test remained ignored. Quick Check, formatting and strict Clippy passed. Seven new contracts changed from 2 passed / 5 failed to 7 passed / 0 failed. |
| [#66: combined retrieval, body persistence and failed-reload protection](https://github.com/bpstr/codanna/pull/66) | `1be917708e1ee9e1b36b683e2c1cad7f53d93461` | Internal #67 merge preserves #62 and #65 histories. Before the reload repair, 130 process/library selections and the original relevance checker passed, but formatting/artifact upload did not. The pinned repair run passed 40 contracts, formatting and strict Clippy; the same reload fixture changed 1 passed / 2 failed to 3 passed / 0 failed. Current-head Quick Check passed. Final combined selected and full-test jobs [35792815064](https://github.com/bpstr/codanna/actions/runs/35792815064) are still running at this checkpoint. |

#65 enforces the existing code journal's 1..=4096 range before known-invalid
probes, before source inference/force clearing after an unknown probe, and before
checkpoint artifacts. Planner and code runtime share effective configuration
validation. The generic document backend keeps its own contract.

#66's two new actual CLI cases verify no provider calls for lexical-only or
unsupported scoped semantic requests, one probe plus one query input for explicit
unscoped semantics, one result per body parent, and lexical fallback for corrupt
or source-policy-incompatible stores. Fixed vectors establish behavior, not
semantic relevance. Query fixtures preserve index bytes, paths and mtimes.

The new failed-reload guard prevents retained old vectors from remaining usable
or republishable after corrupt or removed metadata. Existing data stays preserved
for recovery; a valid reload re-enables semantics and lexical search remains
available. A missing first-load store remains optional.

## Relevance evidence, not only cache work

The frozen task corpus contains five real Rust modules, 152 indexed symbols, ten
source-inspected path/name owners and twenty operational/paraphrase questions.
The original oracle SHA-256 remains:

`129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`

[#58](https://github.com/bpstr/codanna/pull/58) measured direct 6/20 Hit@5 and
MRR@5 0.210, exposing the limits of the vocabulary-matched synthetic 21/21 corpus.
[#59](https://github.com/bpstr/codanna/pull/59) selected normalized whole-word,
identifier-part and English-stem coverage from measured alternatives: direct
8/20, MRR@5 0.267, with all previous successes retained. Raw-score-only and
file-quota alternatives were not shipped because they lost successes.

[#61](https://github.com/bpstr/codanna/pull/61) added bounded one-hop Calls evidence
separately from direct ranks. #62 and the #66 combined build retain direct 8/20
versus direct-or-related 10/20. Paraphrases remain **1/10** on both surfaces.
These questions influenced selection and are not an independent holdout.
Passing fixtures are not passing the original 90% Hit@5 / 0.75 MRR@5 requirement.

Body capture and planning in [#60](https://github.com/bpstr/codanna/pull/60) and
[#63](https://github.com/bpstr/codanna/pull/63) establish available inputs, not a
semantic model's relevance. Across the entire five-file corpus, the recorded
planner comparison is 37 comment inputs / 4423 UTF-8 bytes versus 117 body segment
inputs / 116547 bytes for 102 body parents under a 2048-byte budget proxy. These
are not provider tokens or workspace-wide cost estimates. Comment mode remains
the default; no real-model evaluation or production body-policy opt-in is implied.

## Remaining qualification sequence

1. Complete #66's exact-head read-only selected and repository full-test jobs.
   Retain any actual failure and distinguish a passing candidate from a passing
   committed-source build. Earlier #63 approval-required runs remain untouched.
2. Audit and integrate the original parser/graph/API branches separately, rather
   than assuming #66 contains all #43 work. An ancestry comparison of #46's
   `fix/jsx-owner-resolution` head `b570e1a` against #66 still reports divergence
   and eleven #46-side commits not in #66. This is an ancestry fact, not proof
   that no equivalent change exists elsewhere; code/tests need comparison before
   selecting merges. Preserve existing feature histories and new negative controls.
3. Evaluate the unresolved relevance gap with additional independent source-grounded
   judgments and genuinely available extra retrieval evidence. Do not add separate
   branch gains, expand budgets without cost evidence, or present fixed-vector tests
   as real semantic quality. Paid model experiments still require explicit approval,
   a hard request/spending cap and a stop condition.
4. Finish the all-branch release matrix: persistence/corruption recovery, CLI/MCP,
   watcher/lifecycle parity, default/all/no-default-feature compatibility,
   source-built planner packaging and controlled cold/warm latency/RSS. A release
   and one recoverable production rebuild follow those gates, not every small PR.

## Original task groups still need bounded closure

T01 has progressively stronger frozen corpora, not a complete real-workspace
acceptance suite. T02 truthful graph evidence and T03/T04 JSX/import/ownership
repairs have their own PRs and negative controls; related-code diagnostics do not
prove every graph tool or language complete. T05/T06 ranking experiments and T07
scope integration are implemented but the overall quality gate remains unmet.
T08 distinguishes eligibility, representation/input policy and vector presence;
unknown code/vector generation alignment must not be labeled fresh. T09 still
needs closure of all result-parity and unavailable-versus-empty recall cases.
T10 routing, T11 full lifecycle/watch parity and T13 release qualification are not
closed by focused passes. T12/T14 product/caller follow-ups remain outside these
runtime changes. Browser-scroll argument errors from the original capture remain
caller-side evidence, not newly inferred Codanna parser defects.

The [original fixture review](fixture-followups.md) is retained as historical
context. Later PRs supersede its then-unimplemented statuses. Use their actual
failing baselines and committed-source results for current coverage. No production
source, private transcript or provider credential enters these automated fixtures.
