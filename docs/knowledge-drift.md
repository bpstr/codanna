# Knowledge freshness and drift

`codanna-knowledge check` compares snapshot source hashes and repository revisions with current local roots.

```sh
codanna-knowledge check \
  --graph .codanna/workspace-knowledge.json \
  --root assign-core=../assign-core \
  --root assign-web=../assign-web
```

Exit policy is intentionally narrow: definite errors fail. Add `--fail-on-review` to also fail on review signals.

Definite errors currently include missing/escaping indexed source files and local Markdown links that the supported extractor explicitly failed to resolve. Review signals include changed source hashes, changed Git revisions, ambiguous references, and unresolved ADR/RFC references. A code or document change is never called a contradiction automatically.

If a repository root is omitted, the report contains `ROOT_UNAVAILABLE` info and makes no freshness claim for it. Hash checks use the same canonical root containment and source-size limits as indexing.
