# Assign calendar retrieval: consolidated findings

Status: investigation recorded; remediation proposed, not implemented. Reviewed 2026-09-22 against Codanna `ddb5ae61a72938d82cceaf42123dc7a88bfe3417`. Owner: Codanna maintainers. Next steps: [action plan and task checklist](action-plan.md).

## Evidence and limits

This packet combines the two user-supplied request captures, Assign REVIEW-0020, the completed Codex task **Investigate Codanna calendar results**, the user-directed ASB-1 task transcript, and source inspection in this checkout. The related REVIEW-0016 supplies historical index-lifecycle context. No Assign implementation, index, configuration, or transcript store was changed. No tests, builds, embedding queries, model evaluations, or provider calls were run for this packet. MCP symbol discovery and source reads supported the investigation; they are not a reproduction of the Assign failures.

The destination is a public Codanna repository. This packet retains compact tooling evidence and finding summaries, not the private conversation, application source, or full internal report. Source attachment hashes identify what was reviewed without introducing local filesystem dependencies:

| Source | Identity | Interpretation |
| --- | --- | --- |
| S1: integration-binding context capture | SHA-256 `0d77103d44bdc4f7715ee22afc44b4d9595f2d71e737ed4172e6a685284c96cd` | Rejected context limit, then noisy search output. Original paste omits arguments; S7 recovers the rejected conversation_limit value. |
| S2: calendar feature capture | SHA-256 `64d96893e9eb86f917bd7362b347d021e2ebdeaff158a43add15fe4dd54c2330` | Broad symbol results, exact CalendarPage miss, repeated workspace metadata. |
| S3: REVIEW-0020, dated 2026-09-21 | SHA-256 `add34bfe1233436a3d53d3eca3d7c9949de5ed202d83fb47948f241c6b338f68` | Ten request families, report-local exact excerpts, source cross-checks, and reported fixture tests. |
| S4: Investigate Codanna calendar results | Completed task read through Codex task history on 2026-09-22 | Confirms every recorded Codanna call explicitly supplied the Assign `project_path`. Several examples in S3 omit that selector for brevity. |
| S5: REVIEW-0016, dated 2026-09-20 | Historical review of Codanna rc1 `fc7d934` | Scope cleanup, force-rebuild and long-lived-reader findings; not proof those defects remain in current main. |
| S6: user follow-up, 2026-09-22 | Exact reported text: `Accessibility action requires an element index or point` | Initially reported as appearing inside Codanna output; user then directed investigation to ASB-1, possibly find_symbol. Attribution corrected by S7 below. |

S7: the authorized **Fix ASB-1 git integration** task was read on 2026-09-22. Its raw completed-tool events supplied server/tool identity, arguments and result envelopes omitted from the desktop summary. Only relevant tooling evidence is retained below; the private transcript remains outside this repository.

S3 inspected dirty Assign repositories. Its source revisions include architecture `aceb19ee2fb315f59ec190437d65359f3a5a7b0a`, Web `a4c4bc0bb027dbb79855ec26b66742e4a8a91914`, and Core `a0185f9b6bf94ea5d6d00b2ab62082eb651be04c`, each plus uncommitted changes. Its shortened per-file hashes are provenance clues, not independently reproducible full digests. The exact running Codanna binary hash and index generation were not captured. Historical numeric symbol IDs must be resolved again against each tested generation.

## Results retained from the reports

This table is a normalized summary, not a verbatim transcript or a new test run. Query strings are retained exactly; response ordering is abbreviated explicitly.

| Case | Tool and query/target | Observed result | Disposition |
| --- | --- | --- | --- |
| S1-A | search_context, rejected request | `failed to deserialize parameters: context limit must be between 1 and 10` | Valid bounded-input rejection. S7 confirms conversation_limit: 0; retry with 1 succeeded. |
| S1-B | search_context: `integration binding creation subscription github development activity create default incoming subscription webhook eligible targets task development activity local development server fixtures Reboot FRS-1` | Code ranks 1–2: `development`, score 296.29; subsequent generic `subscription` symbols around 244–246. Documents mix current integration architecture, legacy briefs, and other integrations. | Weak code relevance; mixed document relevance. No justified implementation oracle yet for this integration query. |
| S2 | search_symbols: `calendar feature`, limit 20 | `calendarModule` 121.38; `featureContent` 80.22; Assign and two reference `Calendar` functions 5.12; further partial matches. | Generic partial matches dominate. |
| T1 | get_index_info | 119,630 symbols, 3,812 files, 55,623 relationships; 2,122 AllMiniLML6V2 embeddings, 384 dimensions; updated 21 hours earlier; `semantic-manual`. | Correct workspace/status. Counts are historical, not current measurements. |
| T2 | search_documents: `calendar settings location effects first day of week workspace feature calendar`, limit 10 | Canonical calendar/settings documents occupy leading ranks; first score 0.690. | Strong document discovery in this sample. |
| T3 | search_context: `calendar settings where are they located and how do they take effect first day of week calendar feature`, limits 8/8/3 | Eight unrelated `settings` symbols; useful owning documents; recall index missing. | Code relevance weak, documents useful, unavailable source explicit. |
| T4 | semantic_search_with_context: `implementation of calendar settings first day of week and workspace calendar feature`, limit 8 | ScheduleSpec, weekContaining, WeekStrip, closedTasksRangeMs, TaskDateFilter, recallDates, accountCurrentWeek, toDay. | Real consumers retrieved; settings UI, distribution hook, and shared Calendar missed. |
| T5 | search_symbols: `calendar settings`, limit 20 | Ranks 1–10 generic `settings` variables; 11–12 settings modules; 13–20 more variables/parameters. | Discovery failure. |
| T6 | find_symbol useAccountPresentation; find_callers historical ID 29951 | One definition; 12 callers, including shared Calendar and date/week consumers. | Useful indexed reverse-call evidence. Not a proof of exhaustive runtime callers. |
| T7 | get_calls historical ID 29951 | Exact short response: `symbol_id:29951 doesn't call any functions` | Misleading empty-graph wording. Source has React hook and browser member calls; their external targets may not be indexed. |
| T8 | analyze_impact historical ID 28841, depth 3 | Exact short response: `No symbols would be impacted by changing symbol_id:28841` | False assurance relative to reported JSX consumers. Broken stage not established. |
| T9 | find_symbol CalendarPage, TypeScript | `No symbols found with name: CalendarPage` | Consistent with reported unimplemented full Calendar feature; a lookup miss alone cannot establish absence. |
| T10 | semantic_search_with_context: `backend resolves account first day of week inheritance from workspace default in GET me`, Go, limit 10 | Starts GetWorkspaceMemberByUsername, SetArchived, SwitchWorkspace; misses relevant current-account path. | Relevance failure; semantic search cannot prove missing implementation. |

S3 reports four deterministic Web test files, 18 passing tests, 4.48 seconds. Those establish the tested fixture-mode UI/date behaviors only. They do not validate Codanna correctness, API-mode inheritance, a running backend, or deployment. S4's individual tool durations range from 1 ms to 826 ms across the recorded calls; these are unreplicated task measurements, not routing-overhead benchmarks.

## Findings and reconciliation

| ID | Priority / owner | Finding and evidence | Confidence and disposition |
| --- | --- | --- | --- |
| C01 | P2 / MCP routing | Workspace scope is resolved per call. Repeated workspace text/JSON is response decoration, not evidence of another search or workspace switching. S4 used explicit project paths throughout. | Source-confirmed; no routing defect reproduced. Retain observability/performance task, not a routing rewrite. |
| C02 | P2 / symbol retrieval | Multiword topic queries return generic local names above domain implementation symbols. S1, S2, T3 and T5 show the same symptom. | Observed reports; shared lexical search path confirmed. Scoring cause is a strong hypothesis pending score explanations and ablation. |
| C03 | P1 / graph and MCP | Empty indexed calls/impact are presented as source-level absence or isolation. T7/T8 contradict reported source consumers/calls. | Wording confirmed. Resolve missing-edge cause separately; externally defined React/browser functions are not necessarily valid local graph targets. |
| C04 | P1 / TypeScript graph | Shared Calendar has no reported impact despite JSX consumers. | Reported regression. Current parser already extracts JSX Uses edges; investigate receiver/source ownership, aliases, resolution, persistence, and stale index before changing extraction. |
| C05 | P2 / semantic indexing | Sparse/manual semantic state limits discovery. Raw embeddings / total symbols is approximately 1.77%, but that is not coverage of eligible symbols. | Counts reported; eligibility, unique embedded IDs, model input policy and generation alignment remain unmeasured. Do not promise a model swap will fix it. |
| C06 | P2 / scope and ranking | Active, legacy, and reference subtrees intentionally share the Assign index. Generic Calendar names span them. | Confirmed paths/configuration and S5 policy. This is within-workspace scope, not observed cross-workspace leakage. |
| C07 | P2 / MCP output | Text results coexist with `result: null`; document previews expose ANSI escapes; multi-source failures are embedded in prose. | Wrapper/text behavior source-confirmed; escape sequences visible in S1. Improve machine-readable completeness and transport rendering. |
| C08 | P2 / MCP validation | Context-limit error omits which limit failed. S7 identifies conversation_limit: 0 as the rejected argument; retry with 1 succeeded. Limits are constrained to 1–10 in both schema and deserialization. | Source-confirmed. Invalid-input UX task, not evidence the bounds are wrong or missing. |
| C09 | P3 / recall | Conversation recall was unavailable without an explicit transcript index. | Expected optional capability. No automatic private-history discovery/import proposed. Reading S4 via the desktop task tool is separate from Codanna recall. |
| A01 | P1 / Assign Core + Web contract owners | S3 reports Web/API/docs promise Workspace-default Account inheritance while Core lacks persistence/effective-value resolution. | Accepted in originating report; static dirty-tree evidence, not reverified here. Separate product backlog, not a Codanna implementation task. |
| H01 | P1 triage / persistence | S5 reports document force rebuild appended stale chunks; code force rebuild retained stale semantic auxiliaries. | Historical. Reconcile with current transactional storage and lifecycle regressions before reopening or closing. |
| H02 | P2 triage / readers + Go graph | S5 reports stale long-lived reader after replacement and no reverse consumers for interface-mediated CreateTask. | Historical. Keep distinct from current router cache and JSX issues; reproduce against current main first. |
| U01 | P2 external follow-up / browser-action caller | S7 binds two exact accessibility errors to cua_repl.js browser scroll actions with malformed target arguments; a third corrected call succeeded. | Attribution resolved: these captured occurrences are not Codanna/find_symbol failures. Initial S6 placement report is superseded by recorded tool identity. Optional browser-caller hardening remains separate from Codanna remediation. |

### ASB-1 error attribution: exact tool-event evidence

The completed tool events identify **cua_repl.js**, plugin **unified-computer-use**, in-app browser backend. Both failing actions were titled “Inspect linked development card” and returned exactly `Accessibility action requires an element index or point` with `isError: true`.

| UTC on 2026-09-21 | Recorded action | Result |
| --- | --- | --- |
| 22:02:11.727 | `await tab.scroll({ deltaY: 800 })` | Failed: no target supplied in the expected argument position. |
| 22:02:18.994 | `await tab.scroll({ ref: 162, deltaY: 900 })` | Same error: the combined object was not accepted as a target. |
| 22:02:24.972 | `await tab.scroll(162, { deltaY: 900 })` | Completed, `isError: false`; target supplied separately. |

These are two failures, not four: the transcript records each tool event again as model-visible output. All ten find_symbol completed events in the inspected snapshot have `isError: false`, including CreateBinding, CreateSubscription, eligibleWebhookTargets and normalizeGitHubWebhook. That does not qualify their relevance or guarantee no failures in other tasks; it establishes the origin of these two reported occurrences. No browser action was replayed for this investigation.

The same transcript also resolves S1: at 21:36:12.734 UTC, Codanna search_context returned the context-limit error for code_limit 10, document_limit 10, **conversation_limit 0**. The following request changed only conversation_limit to 1 and completed. Both requests explicitly selected the Assign project path. This was caller/schema mismatch, not failed workspace selection. Supporting an explicit way to omit optional recall can be considered separately; zero is not currently valid.

### What each request does about workspace selection

[WorkspaceServer::call_tool](../../../src/cli/workspace/mcp.rs) resolves scope before forwarding and always prepends workspace identity to the final response. Backend `structured_content: None` becomes JSON `result: null` even when text search results are present. The wrapper is therefore misleading to machine consumers, but not a failed lookup.

[ScopeResolver](../../../src/cli/workspace/mcp/scope.rs) has these branches:

| Selection | Per-call work | Client roots request |
| --- | --- | --- |
| Explicit workspace ID/alias | Local registry lookup | None |
| Explicit registered project_path | Resolve path and inspect local registry | None; this is S4's observed branch |
| Automatic, roots + listChanged supported | Reuse cached roots until invalidation; resolve path and registry | Initial request and after change notification |
| Automatic, roots supported without listChanged | Resolve returned roots and registry | Each automatically scoped call |
| No roots support | Resolve launch cwd and registry | None |

Newer protocol roots exchange uses a continuation; older clients use roots/list. Local registry reads still occur when roots are cached. [WorkerPool](../../../src/cli/workspace/mcp/workers.rs) reuses workspace readers with bounded metadata refresh; scope resolution is not equivalent to loading an index or model every time. Existing [workspace MCP fixtures](../../../tests/workspace_mcp.rs) cover root-cache invalidation. They were inspected, not executed in this investigation.

### Ranking mechanism worth testing first

Both [search_symbols](../../../src/mcp/tools/search.rs) and the code section of [search_context](../../../src/mcp/tools/context.rs) call [IndexFacade::search](../../../src/indexing/facade.rs), which delegates to [DocumentIndex::search](../../../src/storage/tantivy/query.rs).

The latter searches name_text, doc_comment, signature and context; name_text uses [3–10 character n-grams](../../../src/storage/tantivy/mod.rs). It combines the parsed query with fuzzy clauses built from the whole original query, then takes exactly the requested TopDocs count. There is no post-search whole-query coverage ranking or diversity pass in that path. File path is stored, but is not one of those default searched fields. Module filtering is an exact term filter, not a general subtree-prefix filter.

The locked Tantivy version is 0.26.1. Its [QueryParser documentation](https://docs.rs/tantivy/0.26.1/tantivy/query/struct.QueryParser.html) describes default OR semantics; this call site sets neither conjunction-by-default nor field boosts. Many overlapping name n-grams can therefore contribute while only part of a topic is matched. That is a plausible explanation for high-scoring generic names, not a measured decomposition of the historical scores. Cross-query BM25 scores and cosine similarities are not comparable relevance probabilities.

The existing [29/29 lexical result](../embedding-improvements/LEXICAL-RESULTS.md) measures a separate document/knowledge acceptance corpus. Its diversity and coverage ideas are useful precedents, but its success does not qualify MCP symbol search. Preserve those historical results and their oracles.

### Relationship failure boundaries

[TypeScriptParser](../../../src/parsing/typescript/parser.rs) already has track_jsx_component_usage and extract_jsx_uses_recursive; the [parse stage](../../../src/indexing/pipeline/stages/parse.rs) emits Uses relationships from find_uses. Inspect named functions, assigned arrows, import aliases/barrels, and same-name components independently. A parser-only witness cannot establish persisted graph or MCP correctness.

[get_calls](../../../src/mcp/tools/symbols.rs) reads resolved Calls neighbors. It does not enumerate all syntactic call expressions or guarantee third-party/browser definitions exist in the workspace. T7 establishes unsafe wording; it does not by itself prove a missing resolvable local edge. Future fixtures must separate locally resolvable member calls from external/unknown calls. Do not fabricate graph edges by matching a global name.

## Product handoff boundary

S3 distinguishes implemented Account calendar-presentation preferences from a planned full Workspace Calendar feature. Existing pickers and week views consume preferences; a missing CalendarPage is consistent with that distinction. A01 should reconcile Account inheritance flags, effective values, persistence, API/Web contracts, and timezone/locale parity in Assign. Codanna should retrieve evidence for that discrepancy, not infer a product implementation from nearby semantic matches.

## Remaining unknowns

- Exact Codanna executable/index generation for S1–S4. S1 arguments are now recovered in S7.
- Scoring contributions and candidate truncation responsible for each broad-query miss.
- The first failing JSX/member-call stage in a fresh, persisted, and incrementally updated index.
- Eligible semantic denominator and whether missed definitions had vectors at query time.
- Which historical force-rebuild, reader and interface-call findings still reproduce on current main.
- Whether any additional accessibility errors exist outside the inspected ASB-1 snapshot. S7 resolves the two matching occurrences; it provides no evidence that Codanna inserted either message.

These unknowns are explicit tasks in the plan; this packet closes none of the implementation findings.

## Packet validation

The documentation-only change passed Git whitespace checks and a local Markdown target check (26 relative links across this packet and its retrieval index). The version-pinned Tantivy documentation link was opened successfully. No Rust or retrieval tests were run, and none of the proposed task checkboxes is complete.
