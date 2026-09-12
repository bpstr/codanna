# Cross-repository workspace contracts

The workspace layer merges multiple validated knowledge snapshots without collapsing repository-qualified identities. Duplicate repository ids are rejected.

```sh
codanna-knowledge workspace \
  --graph ../assign-core/.codanna/knowledge.json \
  --graph ../assign-web/.codanna/knowledge.json \
  --openapi-repo assign-core \
  --openapi-path ../assign-core/openapi.json \
  --out .codanna/workspace-knowledge.json
```

OpenAPI support intentionally starts narrow: version 3.x JSON, operationIds, component schemas and properties. It does not execute generators and does not infer API compatibility from prose. Cross-repository consumers are linked only when an indexed symbol label is an exact unique match for an operation/schema/field label. Multiple matches are retained as unresolved candidates rather than guessed.

This is a conservative first bridge for generated clients. It does not yet prove that a symbol was generated from that exact contract revision; the relationship basis is `resolved`, not `explicit`. Future generator-specific metadata can upgrade this evidence when manifests or generated headers are available.

The workspace snapshot retains each repository revision independently. Change-context queries can be scoped to a single repository or use the combined graph. Contract source bytes are bounded by the same source limit as normal knowledge ingestion.
