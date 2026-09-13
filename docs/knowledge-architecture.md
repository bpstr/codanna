# Structural architecture discovery

The architecture command summarizes the validated evidence graph without LLM-generated descriptions:

```sh
codanna-knowledge architecture --graph .codanna/workspace-knowledge.json --hubs 20
```

It returns connected-component count, deterministic structural communities, graph hubs, cross-repository edge count, unresolved-reference count, and explicit limitations.

Communities use deterministic label propagation over non-candidate relationships with stable ordering/tie-breaking. Their label is the highest-degree member's existing source label. These are navigation aids, not domain truth, team ownership, or subsystem correctness. Hub degree is connectivity only; it does not imply runtime importance, performance cost, or business criticality.

Candidate links never join components or communities. Unresolved references can split what is conceptually one subsystem, which is surfaced in the report rather than silently inferred away.
