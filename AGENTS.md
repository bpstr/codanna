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
- `UPSTREAM.md` for the dated upstream comparison ledger, not as proof of current GitHub status.

## Upstream ledger and overlap

Before an upstream review or an upstream-derived fix, verify this fork's current GitHub parent/source repository and both default branches. Read the current fork default branch, recent merges, and all open fork PRs, including drafts. Compare relevant implementation diffs and fixtures, not just titles or commit SHAs. Reuse existing work instead of creating a competing implementation; record unavoidable shared-file integration points explicitly.

Update the affected entries in `UPSTREAM.md` in the same PR as an upstream-derived change, a changed downstream implementation, or a newly verified upstream disposition. A PR that changes tracked behavior must either update its ledger entries or explain in its description why their recorded evidence and status remain accurate. Check the ledger again after rebasing and before declaring the PR ready.

Keep upstream status and downstream status separate: open proposal, closed unmerged, merged on the default branch, and released are different upstream states; shipped on the fork default branch, implemented only on a PR branch, and prepared fixtures are different downstream states. An open issue does not prove its fix is absent, and green upstream CI does not validate an adapted fork patch. Do not describe this PR's changes as shipped before merge.

Record the review date, coverage window, full inspected fork/upstream commit SHAs, relevant source links, equivalent-fix evidence, open-PR overlaps, and any rebuild or migration requirement. Name tests actually executed and their results separately from suggested or unexecuted tests. Refresh materially changed entries rather than appending repeated reports or growing delivery logs.

Never advance a review date or replace a known revision with a current-looking value without checking it. When access, pagination, or execution is incomplete, retain the last verified evidence and mark the affected status or coverage as unverified. A stale entry is a reason to recheck GitHub, not to assume nothing changed.

Upstream is read-only unless the user explicitly authorizes an upstream write. Fork maintenance does not authorize upstream comments, issues, PRs, pushes, or merges. Never propose the fork's full divergent history as an upstream contribution.

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
