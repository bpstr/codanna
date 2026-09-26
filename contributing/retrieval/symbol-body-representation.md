# Symbol implementation representations

## Finding audit (22 September 2026)

The main collector at `ddb5ae61a72938d82cceaf42123dc7a88bfe3417` emits an embedding candidate only for `RawSymbol.doc_comment`, and the candidate is exactly that comment. Undocumented implementations therefore have no direct representation through this path. A better embedding model cannot recover input that was never indexed.

| Finding | Existing work | Remaining work |
| --- | --- | --- |
| A: comment-only inputs | #54 reports eligibility, vector presence, and unknown freshness. #47/#50 repair compatible-cache reuse/admission; #49 plans costs offline. | Versioned, deterministic implementation representations; source-policy compatibility; controlled opt-in; fixtures. |
| B: multi-source versus fused retrieval | #45 validates context limits; #53 adds lexical subtree filtering. | Bounded lexical/semantic fusion and verified document-derived code anchors. Separate implementation PR. |
| C: present but low-ranked code | #51 measures candidate loss; #52 implements bounded candidate collection and coverage ranking; #53 scopes before ranking. | Do not duplicate these changes or claim their synthetic outcomes are production Assign results. |
| D: graph correctness/completeness | #44 adds indexed-evidence metadata and conservative unknown coverage; #46/#48 repair JSX ownership/resolution and shadowing. | Preserve those contracts; do not relabel traversal completion as source completeness. No duplicate extractor changes here. |

All listed PRs were open and unmerged at the audit. This PR is stacked on #54 so eligibility reporting can follow the actual selected source policy rather than retaining a comment-only denominator.

## Definition

Introduce an explicitly opt-in source policy for meaningful declarations: functions, methods, components/handlers represented by those kinds, and structural types. Leave the legacy comment policy as the default. Do not include every local binding merely to increase coverage counts.

A representation identifies its version, workspace-relative path, available qualified/module name, kind, signature, documentation, and implementation source within explicit limits. Preserve literal routes, event names, and configuration keys as source evidence, not generated explanations. Source bytes must come from the same parse snapshot, not a later filesystem reread.

Large implementations require bounded segmentation with parent identity retained. The implementation must document whether segments are independently searchable and must not call truncation complete body coverage. Byte budgets are not tokenizer-exact billed-token estimates.

The source policy and its limits participate in compatibility identity. A comment-only vector must never silently count as equivalent to a body-derived vector. A policy switch requires an explicit rebuild; queries must not silently rebuild or send new source inputs. No LLM summaries are generated.

## Acceptance checklist

- [ ] Verify eligible kinds and source-range behavior with synthetic undocumented implementations.
- [ ] Keep legacy comment inputs byte-for-byte compatible by default.
- [ ] Generate deterministic, bounded UTF-8-safe representations from the parse snapshot.
- [ ] Include signature/path/documentation/body evidence without guessing unavailable qualifications.
- [ ] Preserve parent identity for bounded body segments and explain coverage limits.
- [ ] Bind the selected source policy and budgets into semantic compatibility/cache identity.
- [ ] Reject incompatible persisted semantic state rather than silently accepting old vectors.
- [ ] Update #54 eligibility reporting to the selected source policy.
- [ ] Test body-only edits, path/signature changes, invalid/partial ranges, Unicode boundaries, deletions, and rebuild/reopen behavior.
- [ ] Run provider-free or loopback-mocked tests; record exact checked source and any unexecuted gates.

## Cost and rollout boundaries

No real Assign source, production index, provider credential, or paid inference belongs in fixtures. This PR does not authorize a real-workspace rebuild. Combine approved cache, graph, ranking, and representation changes only after combined-build verification; retain a recoverable backup and separately approve a capped dogfood run. #49's current planner describes the legacy source policy and must not be used to estimate body-policy costs without a matching extension.

## Status

Definition committed before implementation. Acceptance boxes are intentionally unchecked until supported by code and execution evidence.
