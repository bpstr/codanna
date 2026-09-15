# Workspace implementation and qualification

Canonical requirements: [isolated workspaces](multi-workspace.md).
User behavior: [workspace guide](../workspaces.md) and [MCP guide](../workspace-mcp.md).

## Required experience

Use one reusable local MCP entry, open an arbitrary independent project, and make
an unscoped query. Its own private code graph initializes automatically. No project
names, manual registry, repository inventories, or initial indexing commands are
required. Never return another project's context by default. Missing trustworthy
cwd/client-root context is an error, not a last-used-project fallback.

## Implemented audit corrections

| Finding | Implementation | Executable witness |
| --- | --- | --- |
| Manual first indexing | Staged local code-only bootstrap and readiness states | Fresh plain projects without settings/index commands; first source arrives later |
| Broad parent captures child | Nearest independent session boundary | Combined ancestor cannot capture an opened child |
| Recursive query discovery | Ancestor-only resolution; bounded snapshot refresh | Irrelevant unreadable descendants do not affect queries |
| Discovery bypasses admission | Admission before routing; closure-owned permits | Cancelled waiter cannot release a running blocking worker's permit |
| Bad parameters restart readers | Preserve backend MCP/tool-error categories | Bad call then immediate valid call on the same reader |
| Whole-RPC lifecycle mutex | Shared loading and bounded concurrent RPCs | Overlapping calls; cancelling one leaves the other alive |
| Eager model loading | Strict lite readers and lazy facilities | Lexical/statistics calls make zero embedding requests |

## Continuation implemented on the branch

- Native code-only watching, offline catch-up, follower visibility, writer
  election/handoff, and refusal of a conflicting explicit force rebuild.
- Updated CLI/MCP/bootstrap/watch code writers use an OS-backed lease outside
  replaceable storage; blocking work retains ownership until actual completion.
- Canonical-directory-bound recall matches the explicit transcript importer.
  No manual labels or automatic access to private conversation directories.
- Document storage/source/hit validation before model use, with bounded metadata
  invalidation of lazy stores and cached absence. Code-only tools stay usable
  after auxiliary-index refusal.
- Cancellation-owned HTTP/HTTPS sessions: DELETE/expiry releases full or unpolled
  receivers; each session has independent bounded stream capacity.
- Generic `.mcp_stdio.json` and capability guidance match actual routing.
- Focused live/recall process tests are included in Linux and macOS watcher jobs.

These describe code, not a blanket final-head/platform qualification result.

## Qualification gates

- [ ] Focused workspace CLI, bootstrap, discovery, MCP, live-watch, recall,
  document refresh, lazy loading, cancellation, and session tests pass on final head.
- [ ] Default/all-feature suites, no-default-features build, strict Clippy,
  formatting, and documentation checks pass on that same head.
- [ ] Linux/macOS process-lifecycle witnesses and the advertised Windows support
  level are qualified. A type-check is not a native process regression result.
- [ ] Measure warm routing separately from indexing/model loading; report actual
  latency/memory rather than inferred numerical speedups.
- [ ] Expand deterministic stress witnesses for delayed exits, admission, storage
  contention, source growth, cancellation, and recovery.
- [ ] Keep PR draft until its scope, limitations, and claimed behavior match evidence.

Older binaries and direct third-party storage access do not participate in the
new advisory lease protocol. Separate stdio frontends are not a shared daemon.
Normal read/status paths never silently repair corrupt data or spend inference.

## Separate extensions

Generation-qualified references, repository-partitioned graphs within intentionally
combined workspaces, automatic document ingestion, external source adoption,
scoped notifications/resources/mutation jobs, shared authenticated HTTP workers,
and optional cross-workspace discovery remain distinct workstreams. They must not
change the basic zero-configuration independent-project requirement.

Use temporary synthetic data and disabled/mock inference for all automated checks.
Preserve user files, source boundaries, settings, and old indexes on refusal.
Record exact CI commits and outcomes in the PR. Do not merge or deploy implicitly.
