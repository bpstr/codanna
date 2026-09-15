# Isolated workspaces for coding-agent projects

**Status:** Architecture and acceptance contract for PR #34. Automatic local
setup/indexing, registry administration, and local MCP routing are implemented
on the review branch. The complete optional architecture is not yet qualified.
See the [workspace guide](../workspaces.md), [MCP guide](../workspace-mcp.md), and
[delivery checklist](multi-workspace-implementation.md) for current limitations.

## 1. Purpose and required experience

Use one project-agnostic MCP definition:

```json
{"mcpServers":{"codanna":{"command":"codanna","args":["workspace","serve"]}}}
```

Open an independent local project in a coding agent and query its code. Local
client roots or inherited launch cwd select the workspace. A fresh project must
prepare its configuration, registration, and initial code index automatically;
manual workspace IDs, repository lists, and `codanna index` are not prerequisites.
During setup return indexing/empty/error status, never another project's results.

A workspace is an independent graph/index and knowledge boundary. Normally it is
the opened project or checkout. It can also be a monorepo or an intentionally
opened combined source root. No customer's project names, directory topology,
or business/product hierarchy are part of this feature. All examples and fixtures
must be synthetic; aliases and canonical locations are runtime data.

## 2. Invariants

1. Each workspace has an independent index/graph and mutation boundary.
2. Resolve a request once; keep that scope immutable until the request completes.
3. Never switch process-global cwd or environment to route concurrent requests.
4. Keep result and exact-reference ownership explicit. Equal symbol names or
   local IDs do not identify the same object across independent indexes.
5. A containing index, equal repository names, or common parent directory is not
   evidence that independently opened projects belong to the same graph.
6. An unavailable or initializing workspace never falls back to another graph.
7. First-use setup is local, bounded, code-only, and non-destructive. No paid
   embedding batch or force rebuild is an implicit onboarding requirement.
8. Supported code, document, recall, cache, and watcher operations use the same
   workspace context. Disable unsupported recall rather than mix namespaces.
9. Preserve existing explicit configuration, source exclusions, and local data.
10. Readable metadata and successful routing are not proof of every optional
    feature, complete indexing, or measured performance.

## 3. Discovery and identity

The local session root is the authority for automatic setup. An exact opened
workspace configuration is validated. Source subdirectories may resolve to their
nearest checkout/project manifest. An unrelated ancestor's configuration alone
must not widen an independently opened plain project. A malformed unrelated
ancestor likewise must not invalidate a valid nearer boundary.

Diagnostic discovery and query resolution are different operations. Only explicit
diagnostics/indexing may inventory descendants. Routine root selection performs
bounded ancestor checks, not repository enumeration. A model-cache-only `.codanna`
directory is not a workspace. Directory symlinks must not silently extend source
ownership. HOME, filesystem roots, and broad system directories are not implicit
projects; ordinary project directories beneath them remain valid.

Intentional multi-root operation remains available by opening/selecting the
containing root itself. It does not make that root the automatic owner of every
child checkout. No sibling scan or hand-maintained membership list is needed for
the normal independent-project workflow.

Reuse `~/.codanna/projects.json` and its existing IDs. Aliases are mutable selectors,
not identity. Canonical locations distinguish separate clones/worktrees even when
names or remotes match. A validated move may retain its ID; a second live checkout
must not silently share mutable storage. Registry updates hold an OS-backed lock
through the full read-modify-write transaction, use atomic replacement, and
reject stale snapshots, corruption, and unsupported versions without data loss.
Older binaries do not participate in the new coordination protocol.

## 4. First-use indexing

Handshake and tool enumeration remain cheap. An unscoped local request can prepare
a validated session's metadata; the first knowledge request schedules its initial
code-only index. Arbitrary `project_path` tool arguments can select an existing
registration, not authorize bootstrap of unrelated filesystem locations. Validate
an entire client-root set before creating any state. Ambiguous roots require a
scope choice rather than merging graphs.

Use the existing indexing engine with a private staging configuration/generation.
Disable semantic indexing for this job, preserve existing project settings, and
publish only after success and validation. A per-workspace bootstrap lock prevents
independent automatic sessions from publishing competing initial generations.
Never recursively delete active storage to take ownership or heal a failed read.

Admission checks count visited entries/files/bytes, respect ignore rules, and use
the existing language registry and language-pack detector. Unsupported-file-only
folders remain empty without repeatedly launching indexers. When an enabled
source file arrives, the next use can bootstrap it. Zero symbols in a genuinely
indexed file is not equivalent to having no source files.

Bound worker threads, discovery, total job duration, and file count. An overflow
witness prevents a capped pass from being published as a complete allowed index.
Reject publication when the configuration changed since setup. Preserve existing
nonempty indexes and report actionable recovery errors rather than force rebuilding.
These guards are not a transaction over arbitrary concurrent filesystem edits;
complete shared writer coordination remains a separate gate.

Automatic first indexing is distinct from continuous file watching. After an
index is populated, subsequent source edits currently use the established
explicit indexing/watching modes. Reopening a newly published index generation
does not itself index newly changed source files.

## 5. Runtime and Rust implementation rules

Reuse an isolated reader per active workspace with fixed cwd and configuration.
Production modules live under `src/cli/workspace/mcp`, not cross-directory
`#[path]` aliases. Each stdio frontend has its own bounded reader pool; one MCP
configuration does not imply a machine-wide daemon or one process for all clients.

Use explicit loading/indexing/ready/empty/busy/failed states. Concurrent callers
share initialization; dropping one waiter does not restart it. Lifecycle locks
cover short state transitions, not asynchronous RPCs. Bound simultaneous queries
per reader and target cancellation at the individual request, not its shared peer.
Preserve typed backend parameter/protocol errors instead of treating every error
as a crashed process or imposing a cooldown on correctable tool arguments.

Admission must cover discovery, diagnostics, refresh, and queries. Blocking work
owns its permit until the closure actually exits, even after cancellation or a
caller deadline. Do not claim that timing out a `spawn_blocking` handle stops an
already running kernel filesystem operation. Bound traversal and check cancellation
between operations wherever possible.

Validate immutable executable identity once. Use cached metadata snapshots and
coalesced refresh rather than rereading/hashing all files or rebuilding commands
on every warm query. Preserve scope validation when optimizing. Do not claim zero
filesystem work: root/registry validation and bounded refresh still perform I/O.
Measure warm routing, cold index loading, and semantic initialization separately.

Load a lite facade for lexical/statistics queries. Initialize existing semantic
facilities only when a semantic operation needs them; load documents lazily on
relevant calls. Facility initialization is shared and retains errors, so canceled
waiters cannot duplicate model/backend initialization. A failed read never creates
a replacement empty index.

Track physical reader permits through observed child exit, not merely cache
removal. Pin active calls and startup during eviction. Shutdown cancels owned
jobs and awaits transport/process cleanup. Model files, model runtime memory,
and workspace vectors are separate resources; do not introduce a new inference
service or duplicate downloaded assets to implement workspace isolation.

## 6. MCP and output contracts

Explicit workspace ID/alias or registered `project_path` overrides scope for one
request only. Without an override, capability-checked client roots take precedence
over launch cwd. Cwd is a fallback only when the client does not provide roots.
A server launched from HOME with no trustworthy root signal must return a scope
error; it cannot infer another process's later directory changes.

Support bounded legacy roots RPC and negotiated modern MRTR root input. Cache
root responses only when the client promises change notifications. Invalidation
uses a root generation; continuations are random, expiring, single-use, and bound
to original tool/arguments/generation. Root updates affect future requests, not
an already executing request. Reject malformed, remote, excessive, ambiguous,
or changed input without an unrelated fallback.

Reuse generated backend tool schemas and keep strict argument validation.
`list_workspaces` and `get_workspace` do not load every index/model. First-use
cache writes must be reflected honestly in tool annotations; this mode is not
strictly read-only. Every routed result contains readable and structured workspace
ownership. Preserve backend tool errors and JSON-RPC error categories.

Keep the returned workspace ID on symbol-ID follow-ups. Generation-qualified
references remain future API work; raw IDs must be looked up again after a full
rebuild. Namespace future resource URIs, continuations, subscriptions, telemetry,
and receipts just as carefully as results. Until scoped mutations/resources/
subscriptions exist, reject them rather than forward to an arbitrary reader.

## 7. Optional follow-on capabilities

These are not prerequisites for the basic independent-project experience:

- Repository-qualified provenance and verified cross-repository relationships
  inside an intentionally combined workspace. Do not create graph edges from
  equal names or semantic similarity. Row changes need versioned compatibility
  and migration/rebuild rules, not mixed old/new persisted semantics.
- Workspace-bound conversation recall. Imported history needs mandatory namespace
  filters; historical text is evidence rather than current instructions. Current
  automatic/selected/router launches disable inherited recall. Document collections
  also retain workspace ownership and cannot silently fall back to another scope.
- One writer/watcher authority across every legacy CLI and MCP path, persistent
  watch leases, scoped subscriptions, and durable reindex receipts. Registry locks
  and bootstrap locks alone do not qualify this larger lifecycle.
- A shared authenticated HTTP service/daemon. Reuse existing auth, host/origin,
  session ownership, TLS, and visibility controls. Never expose registry-wide
  filesystem access through a new unauthenticated endpoint.
- Explicit federated search across workspaces with bounded fan-out, authorized
  targets, labelled partial failures, and evaluated ranking. Search aggregation
  never implicitly merges graph topology.

## 8. Verification and delivery

Start with two fresh temporary plain project directories containing identical
symbol names but different content. No `.codanna`, registry, manual indexing, or
project-specific MCP settings may precede connection. Unscoped queries from each
must initialize its own graph and never return the other's content. Repeat with
Git projects, broad ancestor indexes, aliases, and roots supplied from HOME.

Cover empty and unsupported-file-only projects receiving their first source;
existing exclusions/configuration; malformed ignore files; changed configuration
at publication; cap overflow; canceled initialization; two connections initializing
the same root; invalid arguments followed immediately by valid requests; shared
initialization and overlapping RPCs; request-local cancellation; metadata refresh;
physical child cleanup; and no embedding calls for lexical tools.

Use synthetic fixtures, cleared credentials, and mocked/disabled inference only.
Do not index real user projects, load their secrets, or spend paid inference in
automated validation. Run focused tests, repository quick/full checks, and platform
witnesses. Record exact commit/run evidence and separate code inspection from
executed tests. Numerical speedup claims require actual benchmarks.

Keep incomplete qualification gates visible in the linked checklist and PR.
Nothing in this contract authorizes merging, deploying, or replacing an installed
binary without a separate user request.
