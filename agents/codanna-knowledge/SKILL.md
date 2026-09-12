# Codanna knowledge workflow

Use Codanna's indexed evidence before broad file-by-file exploration when the knowledge snapshot is available.

## Before editing

1. For known changed files, call `get_change_impact` first. Treat its risk as structural guidance, not runtime severity.
2. Call `get_change_context` with the task and relevant file/entity seeds. Keep the response bounded; follow only evidence that matters to the requested change.
3. If a relationship is ambiguous or unresolved, inspect the candidates instead of guessing.
4. Use normal source reads to verify exact implementation before changing code.

## During review

- Use `find_dead_code` only to generate investigation candidates. Never delete solely because a symbol has zero incoming indexed edges.
- Use `codanna-knowledge check` for freshness/drift and distinguish definite errors from review signals.
- Use `codanna-knowledge architecture` for structural hubs/components, not business ownership claims.
- For multi-repository package relationships, run `codanna-manifests --root repo=path ...`; local package links are static manifest evidence, not proof of runtime loading.

## Efficiency rules

- Prefer one bounded graph query over repeated grep/read loops.
- Reuse entity IDs to avoid name ambiguity.
- Do not request large depth/node limits unless the first result is insufficient.
- Candidate edges, dead-code candidates, architecture communities and risk scores are suggestions; source-backed resolved/explicit evidence is stronger.
