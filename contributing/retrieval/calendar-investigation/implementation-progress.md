# Calendar investigation implementation progress

Updated 2026-09-22 (Europe/Budapest). This is a navigation checkpoint for
[PR #43](https://github.com/bpstr/codanna/pull/43), **not completion of all fourteen
task groups**. Implementations remain on the fork's feature branches. This work
has not merged or deployed them, rebuilt a production index, or used paid inference.

The [original two-batch record](https://github.com/bpstr/codanna/blob/a5661ffd51a87a8ad785261829d567a884f71f19/contributing/retrieval/calendar-investigation/implementation-progress.md)
preserves the first #44/#45 execution history, exact hashes and then-pending gates.
Those historical pending statuses must not be read as the current status of later
integration branches. Each PR's current-head CI remains authoritative.

## Current integration checkpoints

| Slice | Committed head | Observed verification and boundary |
| --- | --- | --- |
| [#62: lexical recovery, workspace scopes and related implementations](https://github.com/bpstr/codanna/pull/62) | `8b8f08c4bee87264665f0df02134089b7632b49f` | Combined source has 66 selected passing contracts. The retained final-source workflow [35768636082](https://github.com/bpstr/codanna/actions/runs/35768636082) passed. Direct owner Hit@5 is 8/20; five direct plus up to six related results expose 10/20. These are different evidence budgets, not additive quality gains. |
| [#64: body-aware planner and rebuild cache reuse](https://github.com/bpstr/codanna/pull/64) | `3496ba1fbf24e5ec8be531a653204c6e9a219327` | Current-head Quick Check, representation and body-cache workflows passed. Integrates #63/#60 with #50/#47 while preserving histories. Controlled body-pressure rebuilds send 272 genuinely missing source inputs instead of 1280; repeated seven-segment input is embedded as three unique texts, without losing seven source-range records. This is not a bill or relevance score. |
| [#65: code dimension rejection before inference and publication](https://github.com/bpstr/codanna/pull/65) | `1f8cc2e4031c4ad167bd409414d15bda62827da9` | The exact-source candidate passed 117 selected contracts, strict Clippy and formatting; one pre-existing CWD-sensitive configuration test remained ignored. Seven new tests changed from 2 passed / 5 failed to 7 passed / 0 failed. Final-head Quick Check passed; the expanded read-only workflow [35783603865](https://github.com/bpstr/codanna/actions/runs/35783603865) is pending at this snapshot. |

The code-dimension repair enforces the existing journal's 1..=4096 range before
known-invalid probes, before source inference/force clearing after an unknown
probe, and before checkpoint artifacts. Planner and code runtime share effective
environment/configuration validation. The generic document backend keeps its own
contract. Neither cache capacity nor the code format limit was increased.

## Relevance evidence, not only cache work

The frozen task corpus contains five real Rust modules, 152 indexed symbols, ten
source-inspected path/name owners and twenty operational/paraphrase questions.
The original oracle SHA-256 remains:

`129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`

[#58](https://github.com/bpstr/codanna/pull/58) measured direct 6/20 Hit@5 and
MRR@5 0.210, exposing the limits of the earlier vocabulary-matched synthetic
21/21 corpus. [#59](https://github.com/bpstr/codanna/pull/59) selected normalized
whole-word/identifier-part/English-stem coverage from measured alternatives:
direct 8/20, MRR@5 0.267, with all previous successes retained. Raw-score-only
and file-quota alternatives were not shipped because they lost successes.

[#61](https://github.com/bpstr/codanna/pull/61) added bounded one-hop Calls evidence
separately from direct ranks. #62 now measures it together with lexical recovery
and pinned workspace scoping: direct 8/20 versus direct-or-related 10/20.
Paraphrases remain **1/10** in both surfaces. These questions influenced selection
and are not an independent holdout. Passing fixtures are not passing the original
90% Hit@5 / 0.75 MRR@5 quality requirement.

Body capture and planning in [#60](https://github.com/bpstr/codanna/pull/60) and
[#63](https://github.com/bpstr/codanna/pull/63) establish available inputs, not a
semantic model's relevance. For the entire five-file corpus the recorded planner
comparison is 37 comment inputs / 4423 UTF-8 bytes versus 117 body segment inputs /
116547 bytes for 102 body parents under a 2048-byte budget proxy. Those are not
provider token counts or a workspace-wide cost estimate. Comment mode remains
the default; no model evaluation or body-policy opt-in is implied.

## Next implementation and qualification sequence

1. Complete #65's current-head read-only gates, including existing CLI force-path
   and emission-version tests. Retain actual failures and approval-required states;
   never turn a successful staged run into an unobserved final-head pass.
2. Qualify the #62 query integration with the #65 body/cache/planner integration
   on an isolated branch. Verify actual dependency ancestry and remaining original
   parser/graph branches, rather than assuming the two trees contain all #43 work.
   Keep lexical-only default behavior and no implicit provider/reindex transitions.
3. Evaluate the unresolved relevance gap using additional independent source-grounded
   judgments and genuinely available extra retrieval evidence. Do not add separate
   branch gains, expand budgets without cost evidence, or present fixed-vector tests
   as real semantic quality. A paid model experiment still requires explicit
   approval, a hard request/spending cap and a stop condition.
4. Run the all-branch release matrix: persistence and corruption recovery, CLI/MCP
   behavior, watcher/lifecycle parity, default/all/no-default-feature compatibility,
   read-only planner packaging, and controlled cold/warm latency and memory checks.
   A combined release and one recoverable production rebuild come after those gates,
   not after every small PR.

## Original task groups still need bounded closure

T01 has progressively stronger frozen corpora, not a complete real-workspace
acceptance suite. T02 truthful graph evidence and T03/T04 JSX/import/ownership
repairs have their own PRs and negative controls; related-code diagnostics do not
prove every graph tool or language complete. T05/T06 ranking experiments and T07
scope integration are implemented but the overall quality gate remains unmet.
T08 now distinguishes eligibility, representation/input policy and vector presence;
unknown code/vector generation alignment must not be labeled fresh. T09 still
needs closure of all result-parity and unavailable-versus-empty recall cases.
T10 routing, T11 full lifecycle/watch parity and T13 release qualification are not
closed by a focused test pass. T12/T14 product/caller follow-ups remain outside
these runtime changes. Browser-scroll argument errors from the original capture
remain caller-side evidence, not newly inferred Codanna parser defects.

The [original fixture follow-up review](fixture-followups.md) is retained as a
historical source of cases. Later PRs supersede its then-unimplemented status;
use their recorded failing baselines and committed-source results for current
coverage. No production source, private transcript or provider credential is part
of the automated fixtures.
