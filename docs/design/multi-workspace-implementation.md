# Workspace implementation and qualification

Canonical requirements: [isolated workspaces](multi-workspace.md).
User behavior: [workspace guide](../workspaces.md) and [MCP guide](../workspace-mcp.md).

## Required experience

One reusable local MCP configuration serves arbitrary independent coding-agent
projects. Open a project, make an unscoped query, automatically prepare its private
code index, and never receive another project's context by default. No named
product topology, manual registry, repository inventory, or first indexing command
is required for the ordinary local case. Examples and fixtures must be synthetic.

A stdio process learns its launch cwd; supported client roots provide authoritative
session context and changes. A missing trustworthy context signal is an error,
not permission to choose the last-used or only other indexed project.

## Audit corrections implemented on the review branch

The audit at `b34f414d` found seven correctness/performance gaps. The subsequent
implementation addresses them as follows; these entries describe code, not a
blanket assertion that every release/platform test has passed.

| Finding | Implementation | Executable witness |
| --- | --- | --- |
| First use required manual setup | Local session-root bootstrap, staged code-only indexing, readiness states | Fresh non-Git projects with no settings or indexing calls; empty project later receives source |
| Broad parent could capture a child | Nearest independent boundary; plain client root ignores bare ancestor configuration | Broad combined parent above an independent opened project |
| Recursive query discovery | Ancestor-only query resolution; inventory restricted to diagnostics; bounded metadata refresh | Unreadable irrelevant descendant does not affect routing; client-root cache invalidation |
| Discovery bypassed admission | Request admission before routing; blocking permits held by actual closures | Cancelled waiter cannot release a still-running blocking worker permit |
| Bad parameters restarted readers | Preserve tool-error results and typed JSON-RPC errors | Invalid call followed by immediate valid call; controlled backend error-code preservation |
| Slot lock serialized reads/startup | Shared loading state; lifecycle guard released before bounded concurrent RPC | Two blocked reads overlap; cancelling one leaves the other alive |
| Lexical lookup loaded models | Private strict lite reader; cancellation-independent lazy semantic/document initialization | Lexical tools make no configured embedding-endpoint calls; cancelled lazy waiter does not duplicate initialization |

The reader process manager holds physical capacity until child exit is observed,
not merely until removal from a map. Independent router processes are not a shared
daemon. Bootstrap locks coordinate the new initial-index lane, not every existing
CLI or watcher writer. Invalid/partial existing data is never automatically cleared.

## Qualification gates

- [ ] All focused workspace CLI, automatic discovery, MCP, lazy-initialization,
  cancellation, and blocking-budget tests pass on the final review commit.
- [ ] Full default/all-feature suites, strict Clippy, formatting, and documentation
  checks pass on that same commit; earlier formatted CI checkouts are not substitutes.
- [ ] Windows and macOS process-lifecycle witnesses, copied/worktree roots,
  malformed configuration, unavailable filesystems, shutdown, and crash paths
  are qualified for their advertised support level.
- [ ] Measure warm routing separately from initial indexing/model loading. Report
  observed timings and memory rather than implying a measured speedup from code review.
- [ ] Expand deterministic stress coverage for process admission, delayed child
  exit, indexing cancellation, source growth, and competing explicit writers.
- [ ] Keep the PR draft until review scope, documented limitations, and all claimed
  behavior match executable evidence. No implicit merge, deployment, or installation.

## Optional capabilities outside basic independent-project onboarding

The following remain separate work. They must not delay or redefine the basic
first-use/isolation requirement, and must not be described as implemented merely
because the local router is usable:

- Workspace-qualified recall and strict document collection/provenance policies.
- Generation-qualified references and explicit repository-partitioned resolution
  inside intentionally combined multi-repository workspaces.
- Shared authenticated HTTP workers with existing principal, origin, TLS, and
  workspace-visibility constraints.
- Complete cross-process CLI/watch writer ownership, scoped notifications,
  subscriptions, and durable user-requested reindex job APIs.
- External source-root adoption and optional cross-workspace search/groups.

Use deterministic synthetic data and disabled/mocked inference for every automated
check. Preserve user files, current configuration, source boundaries, and old
indexes on refusal. Record exact commits and observed CI results in the PR.
