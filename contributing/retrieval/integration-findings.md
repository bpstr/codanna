# Codanna integration findings and implementation boundaries

Audited on 2026-09-22. This is a point-in-time map of the Assign integration findings, not a claim that open branches have been merged or validated together.

## Existing work

| Finding | Responsible work | Boundary |
| --- | --- | --- |
| A: undocumented implementations have no comment-based vector representation | #55, stacked on #54 | `feature/symbol-body-representation` at `05968af72f23d5040853b044558148b4e28f2afe` already contains parser-snapshot capture, bounded parent-linked segments, source-policy identity, segment persistence and lifecycle changes. Do not replace it with the older standalone representation-builder draft. |
| B: separate context sections are not ticket-aware code fusion | #56, stacked on #53 | The audited head `053cbd199a5ca6da33bf3f73e363bc504cf689ef` contained the definition and workflow only. Runtime retrieval and contract tests belong here. Preserve the existing `search_context` behavior. |
| C: relevant owners are indexed but ranked below the requested window | #51/#52; scope in #53 | Reuse bounded lexical candidate collection and ranking. Their synthetic results are not production Assign measurements. An embedding rebuild is not a remedy for lexical candidate ranking. |
| D: successful graph traversal does not prove extraction completeness | #44; extractor repairs in #46/#48 | Keep traversal completion, available indexed edges, source coverage and freshness separate. A document-derived identifier must not create a graph relationship. |

Adjacent work remains separate: #47/#50 cover cache reuse/admission; #49 covers offline rebuild planning; #54 distinguishes eligible symbols, actual vector presence and unknown freshness.

## Implementation sequence

1. Keep the fuller #55 implementation and its opt-in source-policy contract. The default remains comment-based indexing. A different input policy is a different compatibility identity, not an interchangeable cache entry.
2. Add bounded ticket retrieval in #56 without rewriting #52/#53. Fuse source ranks rather than adding raw lexical and cosine scores. Resolve document identifiers to indexed code before admission, and expose all contributing sources.
3. Validate contracts on the exact committed source, including the MCP tool catalog and real indexed fixtures. A filtered command that runs zero tests is not acceptance.
4. Validate the combined dependency stack before any production evaluation. The old comment-policy cost planner must not be presented as a body-policy estimate.

## Conservative evidence contract

A matching indexed symbol proves an indexed name match, not current implementation ownership, runtime execution, or a dependency edge. Document evidence is untrusted data, never a new instruction or executable query syntax.

A ticket response that does not traverse the graph must say `query_status: not_run`. Source coverage and indexed source revision remain unknown unless the index actually persists and verifies the relevant provenance. Reader generation counters are not Git revisions and cannot establish freshness.

Scoped lexical and identifier-anchor collection must apply the subtree filter before candidate collection. Semantic scope must either be implemented and explicitly described as bounded/post-filtered, or reported as unsupported without widening the user's scope.

## Validation and cost boundary

The historical 2,122 embeddings / 119,630 symbols observation is not eligible-symbol coverage. Unit tests, contract results, synthetic relevance scores, real Assign measurements and unexecuted checks must remain separately labelled.

No production reindex, paid-provider experiment, merge of open PRs, or upstream modification is authorized by this implementation record. Tests use deterministic local fixtures. See `ticket-code-fusion.md` for the retrieval checklist and #55's `symbol-body-validation.md` for body-policy rollout constraints.
