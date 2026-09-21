# Calendar retrieval action plan

Status: proposed; documentation-only PR. Owner: Codanna maintainers. Updated 2026-09-22. Evidence: [consolidated findings](evidence.md). All tasks below are open. No runtime, parser, ranking, configuration, index, or test changes are included in this packet.

## Sequence and decision gates

1. Freeze reproducible evidence and preserve failing baselines (T01). Do not tune against a moving Assign index.
2. Correct misleading empty-result contracts and isolate graph failures (T02–T04). These P1 correctness tasks precede interpreting a zero as safe change scope.
3. Compare bounded lexical ranking variants (T05–T06), then improve result scope and semantic diagnostics (T07–T08). Accept changes only against an unchanged oracle and adversarial controls.
4. Clarify validation, rendering, routing diagnostics, and historical lifecycle disposition (T09–T11).
5. Hand the separate product gap to Assign owners and publish final verification evidence (T12–T13).

Tasks may be split into focused implementation PRs after this plan is approved for implementation. Owners name responsible areas, not assigned people. P1/P2/P3 describe impact; dependency order controls when work can be verified. Follow [the fork/upstream policy](../../../UPSTREAM.md): route generic parser fixes as possible upstream candidates, while workspace routing and fork-specific ranking remain downstream unless separately proposed. This plan targets the fork only.

## Tasks and acceptance criteria

### T01 — Freeze a deterministic retrieval baseline

- [ ] P1, retrieval/QA; covers all findings. Record exact binary SHA/version, source revision/dirty diff identity, index generation/schema, roots and exclusions, freshness, capability negotiation, and each complete request. Separate runtime evidence from a checkout merely containing similar code.
- [ ] Create a small hand-authored corpus with active, archive/reference and test subtrees; generic settings/subscription variables; calendar settings owner/consumers; duplicate Calendar symbols; aliases/barrels; named and arrow components; a peer workspace with conflicting names. Keep labels, expected answers and reports outside the indexed corpus. Do not copy private Assign source.
- [ ] Preserve exact raw responses locally and commit safe compact normalized evidence. Resolve symbol IDs per run by path/name/kind/owner; never reuse IDs 29951 or 28841 as fixture constants.
- [ ] Port calendar settings, calendar feature, backend inheritance and integration-binding intent into separately labeled cases. The integration query needs an independently identified owning implementation before a rank target is claimed. Keep missing implementation/route controls as unsupported-negative-claim tests.
- [ ] Completion: versioned fixture/oracle hashes, one observed baseline for each executable case, explicit Not run/Unavailable states, and a candidate-stage failure classification. Existing failures remain failures.

### T02 — Make empty graph answers truthful

- [ ] P1, MCP/graph; C03; depends on T01. Define empty as no resolved indexed Calls/Uses/dependents for this generation, not no syntactic calls or no possible effect.
- [ ] Preserve complete, budget-exceeded, unavailable/error and empty distinctions. Include graph/index scope and freshness where known; use unknown when completeness cannot be established. Structured metadata must not equate complete traversal with complete source coverage.
- [ ] Completion: empty, missing symbol, unavailable graph, bounded traversal and genuine isolated-symbol fixtures produce distinct accurate text/metadata across relevant tool surfaces. Existing budget/error tests still pass.

### T03 — Trace and repair JSX impact end to end

- [ ] P1, TypeScript parser/resolver/storage; C04; depends on T01, output contract T02. Trace source AST → raw Uses with source range → import/owner resolution → stored edge → reverse traversal → MCP response. Locate the first missing or misbound stage before choosing a fix.
- [ ] Include direct/named/namespace aliases, barrel re-exports, project aliases, function and assigned-arrow owners, nested components, namespace JSX and two unrelated Calendar definitions. Intrinsic lowercase JSX must not become a component edge. Require bounded conservative resolution when a target is ambiguous.
- [ ] Completion: each expected resolvable consumer reaches the correct Calendar through Uses/composition, at correct depth and source location; decoy workspace/reference components remain unconnected. Verify fresh index, save/reopen, import-only edits, deletion/recreation and watch/incremental parity. Do not turn JSX rendering into direct Calls.

### T04 — Separate local member-call resolution from external calls

- [ ] P1 investigation, parser/resolver/MCP; C03 and H02; depends on T01. Compare namespace-imported local functions, object/class methods and shadowed receivers with external React/browser calls whose targets are absent.
- [ ] Add a separate Go interface-dispatch reproduction for S5's CreateTask finding. Keep it a distinct language work item; prove receiver/type identity rather than globally matching a method name.
- [ ] Completion: resolvable local calls have correct Calls edges and reverse symmetry after persistence/incremental edits; ambiguous or external targets never bind to namesake local symbols. Decide whether unresolved call-site evidence is retained/exposed or explicitly unsupported. Accurate T02 wording is mandatory either way; an external call is not required to invent an indexed target.

### T05 — Explain symbol scoring and candidate loss

- [ ] P2, retrieval; C02; depends on T01. Capture candidate count/order and score explanations for generic settings/development/subscription and relevant owners. Measure the effects of OR clauses, n-grams, analyzed fields, whole-query fuzzy clauses, duplicates, and TopDocs truncation separately.
- [ ] Compare explicit identifier, typo, phrase, Boolean query, natural-language topic, short token, snake_case and camelCase behavior. Preserve existing query syntax and exact kind/language/module filters.
- [ ] Completion: each representative miss is classified as absent corpus, absent candidate, low rank, duplicate-budget loss or wrong scope; publish score evidence rather than treating raw score magnitude as confidence.

### T06 — Evaluate bounded ranking improvements

- [ ] P2, retrieval; C02; depends on T05. Compare the ranked alternatives below against frozen tuning and holdout sets; run ablations rather than shipping all knobs together.
- [ ] Completion: meet the acceptance table, preserve every correctness invariant, and report candidate/latency/memory costs. Retain the simpler winning approach; keep failed variants and unrun semantic quality clearly labeled. Update both search_symbols and search_context code retrieval through shared logic rather than divergent rankings.

### T07 — Add explicit within-workspace scope and useful diversity

- [ ] P2, retrieval/MCP; C06; depends on T01/T05. Specify a consistent path/subtree filter for relevant search tools before adding a schema field. Existing module is an exact module term, not a path prefix. Apply filters before top-k selection so omitted candidates cannot consume the budget.
- [ ] Make active/archive/reference preferences explicit and opt-in or transparently configured. Keep reference code searchable; do not remove legacy/kaneo or hard-code Assign-specific ownership. Prefer distinct source/owner evidence when repeated generic locals would fill the budget; preserve exact symbol lookup and sparse-result fallback.
- [ ] Completion: scoped queries contain no out-of-scope result, broad queries still discover reference code, and exact duplicate names remain disambiguatable by path/owner. Tests cover empty scope, escaping paths, aliases and same-name definitions in separate workspaces.

### T08 — Report semantic eligibility and freshness accurately

- [ ] P2, indexing/MCP; C05; depends on T01. Report total symbols, eligible symbols under the actual input policy, unique embedded symbols, skipped/pending counts, model/input-policy identity and code/vector generation alignment. Preserve manual freshness when automatic refresh is unsupported.
- [ ] For missed settings/UI/Core candidates, first check eligibility and vector presence. Distinguish absent vector, stale generation, low similarity and excluded scope. Review whether name/signature/path/context input would expand useful coverage before proposing any model replacement.
- [ ] Completion: fixed-vector fixtures prove metadata, filtering, stale/error handling and explicit lexical fallback behavior without claiming model relevance. No query silently rebuilds an index or enables a provider. Any future real-content quality evaluation is separately authorized; it is not a completion claim of prepared tests.

### T09 — Make validation and text/JSON output consistent

- [ ] P2, MCP; C07–C09; depends on T01. Keep all three context limits at 1–10 unless a separately measured budget change is accepted. Add field-specific errors and request-schema boundary fixtures for 0, 1, 10, 11, wrong type, unknown field, and omitted defaults. The current schema already declares the bounds.
- [ ] Provide useful structured results with symbols/IDs, paths/ranges, scores and source status; make text-only absence explicit instead of ambiguous result:null. Plan backward-compatible/versioned treatment before changing the wrapper. Retain one reliable workspace identity and error status.
- [ ] Render MCP previews without terminal ANSI escapes while preserving CLI styling. Keep optional recall unavailable distinct from zero matches; never discover/import private transcripts automatically.
- [ ] Completion: plain-text/structured-output parity tests cover successful, empty, partial and failed sections; schema errors identify the field; payload size remains bounded. Existing text consumers and workspace-scoped follow-ups remain compatible or receive an explicit migration contract.

### T10 — Measure routing work before optimizing it

- [ ] P2 diagnostic, workspace/MCP; C01; depends on T01. Count client roots requests, registry reads, metadata refreshes and worker loads for explicit ID, explicit path, roots with/without notifications, launch-cwd fallback, roots continuation and a changed-root session.
- [ ] Completion: known branches match the evidence table, explicit selectors never request roots, cacheable roots are refreshed only after invalidation, changed/ambiguous roots cannot reuse another workspace, and warm calls reuse readers. Record repeated isolated timings before considering registry caching. No workspace-selection rewrite is justified by the label alone.

### T11 — Reconcile historical lifecycle findings with current main

- [ ] P1 triage, persistence/readers; H01/H02; depends on T01. Compare REVIEW-0016's rc1-era behavior with current [index lifecycle contracts](../adversarial/index-lifecycle.md), [embedding/storage work](../embedding-improvements/README.md), [collection reload](../embedding-followups/README.md), and [upstream overlap](../../../UPSTREAM.md).
- [ ] Use disposable local stores and fixed embeddings to check repeated force replacement has no duplicate chunks/stale symbol-vector mappings, replacement failures retain a consistent generation, and long-lived readers observe the intended replacement. Preserve failed historical evidence.
- [ ] Completion: each historical finding is Reproduced, Already covered with executed evidence, or Not reproduced with stated scope. Open only the missing regression/implementation slice. Do not use an empty-store rebuild of the real Assign index as a substitute for fixing replacement semantics.

### T12 — Hand off the Assign contract gap

- [ ] P1, Assign Core/Web/API owners; A01; independent product work. Reverify dirty-tree evidence before implementation; reconcile inheritance persistence, mutation, effective values and Account-vs-Workspace precedence across API, Web, SDKs and documentation. Check timezone/locale along with first day of week.
- [ ] Completion in Assign: explicit/inherited toggles, default changes, missing defaults and API-mode reload/propagation are tested against the accepted contract; publication/deployment status is recorded separately. Keep planned Workspace Calendar feature distinct from current date-picker settings. This Codanna PR records the handoff only and does not create an Assign task or change product files.

### T13 — Publish verification and close only proven tasks

- [ ] P2, maintainers/QA; depends on the implemented slices. Run the smallest relevant deterministic regression, then required repository gates for that implementation. Keep corpus/oracle hashes stable between before/after reports.
- [ ] Completion: each finding links its implementation, exact-source test results, remaining limitations and state (accepted, implemented, verified, deployed where applicable). Doc-only intake validation is diff hygiene and link checks; it is not a Rust/retrieval pass. Check session disk usage and remove only inactive reproducible session artifacts.

## Ranking alternatives to compare

The first implementation experiment should be lexical and bounded. None of these alternatives is selected or implemented yet.

| Variant | Proposed mechanism | Expected benefit | Risk/control |
| --- | --- | --- | --- |
| R0 | Current query parser, n-grams and exact top-k | Reproducible baseline | Preserve as measured failure evidence. |
| R1 | Bounded overfetch, then distinct meaningful query-token coverage over stored candidate fields; exact identifier and explicit query syntax retain separate treatment | Prevent one repeated generic word from winning a multi-concept query | Overfetch alone cannot recover candidates outside its pool. Count unique terms; do not reward repeated query tokens. Use a documented hard ceiling, initially compare 4 × and 8 × the requested result limit, capped at 200 candidates for ordinary small-limit discovery. |
| R2 | Exact/full identifier and identifier-token evidence before substring/fuzzy matches; evaluate camel/snake splitting and explicit field weighting | Preserve symbol lookup while reducing n-gram score inflation | If a new analyzed field is needed, specify schema migration/reindex compatibility; do not silently alter old stores. Preserve typo and code-fragment tests. |
| R3 | Bounded all-concept candidate probe plus broader OR fallback, merged by identity | Recover candidates starved out of the original OR pool | Global AND over long natural-language requests is too restrictive; explicit Boolean syntax must retain its meaning. Stopword/query policy needs tests and transparent scope. |
| R4 | Soft symbol-kind/owner/source diversity after semantic relevance evidence | Reduce dozens of unrelated local variables while retaining useful controls | Exact queries for settings/subscription variables must still succeed. No blanket variable/test/archive exclusion and no popularity-only boost. |
| R5 | Later, separately evaluated rank fusion of lexical and available semantic candidates | Recover paraphrases when coverage is sufficient | Gate on T08. Fuse ranks with provenance rather than adding incompatible BM25/cosine scores; missing semantic source is explicit. Fixed vectors test mechanics only. |

R1–R4 require ablation on this code-search path; document-ranking success elsewhere is not sufficient. Prefer an initial source-compatible rerank over a schema migration if it meets the same targets. Improve candidate recall before optimizing final ranking weights. Learned/paid rerankers, model swaps and automatic embedding expansion are deferred.

## Proposed verification contract

These are proposed release criteria for future implementation, not observed results:

| Dimension | Minimum evidence |
| --- | --- |
| Relevance | At least 20 manually labeled positive topic queries plus identifier/negative controls across multiple domains/languages. Keep at least one third of topic queries held out during tuning. Report Hit@5 >= 0.90 and MRR@5 >= 0.75 on positives, plus nDCG@10 for graded labels; report all metrics per query family so aggregates cannot hide a regression. |
| Calendar owners | For the dedicated calendar-settings fixture, owning settings definition and a real consumer appear in the first five; generic unrelated settings variables do not crowd out all domain evidence. Calendar-feature fixture distinguishes implemented preferences from planned feature documentation. |
| Integration intent | Freeze owner/consumer labels before scoring; S1 currently supplies symptoms, not sufficient positive oracle evidence. |
| Identifier compatibility | Exact, typo, mixed case, camel/snake, special code-fragment and explicit Boolean controls all pass. Relevant exact variable queries remain supported. |
| Isolation | Zero out-of-request-workspace results; zero out-of-explicit-path results; zero decoy graph edges. Broad within-workspace reference matches are allowed and labeled. |
| Graph | All required local Calls/Uses edges correct after reopen/incremental edits; external/ambiguous targets stay unresolved; budget/unavailable states never become successful zero-impact claims. |
| Performance | On a fixed machine/corpus, record cold setup separately and at least 30 warmed repetitions per query family. Report median/p95 plus peak memory and candidate counts. Investigate >20% p95 regression before acceptance; approve any exception with measured relevance benefit. No latency conclusion from S4's single calls. |
| Existing behavior | Preserve existing lexical/structural invariants and required repository checks; compare actual outcomes, not just a historical 29/29 report. |

Prepared responses, deterministic fixtures, fixed vectors and mocked transports govern automated validation. No provider credentials, .secrets, subscription proxies, fixture recording, or paid model grading are authorized. The explicit task/workspace prohibition controls even if an external policy document describes broader local allowances. Real-content dogfood requires its own exact scope, user-approved cap and stop condition. Prepared evidence cannot establish model quality, real usage, settlement or deployment qualification.

## Implementation boundary

This PR is complete when evidence provenance, finding disposition, task dependencies, acceptance criteria and documentation links are reviewable. Every implementation checkbox stays open. It does not schedule work, modify ranking, rebuild indexes, import conversations, change Assign settings or run the proposed evaluation.
