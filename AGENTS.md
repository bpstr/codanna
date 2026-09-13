# Agent Contributor Guide

Codanna is a local code-intelligence MCP server and CLI. Keep changes focused, local-first, and independently verifiable.

## Discover

Use Codanna MCP symbol and relationship tools before text search when the repository index is available. Use `rg` for literals, diagnostics, configuration, prose, and any gaps in the index.

Read the narrow source of truth for the work:

- `README.md` for product behavior and supported modes.
- `CONTRIBUTING.md` and `contributing/README.md` for the contribution workflow.
- `contributing/development/guidelines.md` for Rust changes.
- `contributing/development/language-support.md` for parser or language work.
- `.mcp_stdio.json`, `CLAUDE.md.example`, and `.codannaignore` for MCP, agent integration, and indexing behavior.

## Safety boundaries

Treat MCP transports, index persistence, document/RAG indexing, embedding configuration, file watching, install scripts, release packaging, and parser grammars as compatibility-sensitive.

Automated tests, CI, smoke probes, benchmarks, and fixtures must use deterministic local data and mocked provider transports. Keep provider credentials and `.secrets` out of test processes. Paid inference is reserved for an explicitly approved manual dogfood run with a hard request or monetary cap and a stop condition.

Preserve user work in dirty trees. Do not commit generated indexes, caches, model files, or unrelated formatting changes.

## Verify

Start with the smallest test that proves the changed behavior, then expand in proportion to risk. The repository gates are:

```bash
./contributing/scripts/quick-check.sh
./contributing/scripts/full-test.sh
```

Use `./contributing/scripts/auto-fix.sh` only when its broad mechanical edits are intended and reviewed. For docs-only work, run `git diff --check` and validate every referenced path or command.

Keep pull requests scoped to one behavior. Add a regression fixture for behavior changes and update documentation for user-facing commands, configuration, MCP contracts, or language support.

Write plain-English imperative commit subjects without Conventional Commit prefixes; for example, `Fix watcher shutdown` rather than `fix(watch): ...`.
