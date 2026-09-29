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

## Upstream ledger freshness

`UPSTREAM.md` is a dated comparison receipt, not a live source of repository state.
Before upstream-related fixes, dependency adoption, or an upstream review:

1. Verify this fork's current GitHub parent, source, and default branch, and the
   upstream default branch. Record the inspected fork/upstream commit SHAs and
   coverage window; do not reuse yesterday's repository identity or branch assumptions.
2. Inspect current fork code, recent merges, and **all relevant open PR diffs**.
   Detect equivalent implementations even when SHAs differ. Do not duplicate a
   pending PR or describe its fixtures as shipped behavior.
3. Recheck the status of affected upstream issues/PRs and inspect their actual
   code, tests, and actionable comments. Distinguish open proposals, closed-unmerged
   work, changes on the default branch, and released fixes.
4. Update affected `UPSTREAM.md` rows in the same change as the adaptation, including
   downstream evidence paths, pending-versus-shipped state, compatibility/rebuild
   requirements, and tests actually executed. Prepared or skipped tests are not passes.
   Preserve unrelated historical evidence; keep one compact current snapshot instead
   of appending growing run logs.
5. Before opening or refreshing the PR, recheck fork main and open-PR heads for
   overlap. If they moved, review the new diffs before claiming coverage.

Ordinary fork work must also update an affected row when it changes the recorded
behavior, compatibility, or evidence path. During PR lifecycle work, reconcile
pending rows with the verified merge/close result; do not leave merged work marked
as an open proposal. This does not require a full upstream scan for every code edit:
keep the last upstream review date intact unless that review was actually repeated,
and mark an affected upstream status unverified when it could not be checked.

Never advance `Last reviewed` or inspected SHAs without completing those reads.
If access or coverage is incomplete, retain the last successful review and add a
scoped gap; missing data is not evidence of no change. Unrelated tasks need not
rewrite the ledger solely because its date is old. A no-change upstream review
must still record what was inspected and why no adaptation was needed.

Upstream access is read-only by default. A fork change does not authorize upstream
branches, comments, issues, PRs, or merges. Use the explicitly requested fork as the
PR base repository, not GitHub's suggested upstream base.
