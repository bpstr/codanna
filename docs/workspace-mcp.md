# Isolated local MCP workspaces

Implementation is under review in PR #34. Use one reusable configuration:

```json
{"mcpServers":{"codanna":{"command":"codanna","args":["workspace","serve"]}}}
```

Open a project and query it. No per-project names, registration, repository lists,
initial indexing command, or MCP paths are required for supported local layouts.

## Selection and first use

Capability-checked client roots take precedence over the subprocess launch cwd.
Without roots support, the launch cwd supplies project context. A HOME-launched
process without roots can connect and list tools, but cannot invent a project.
A long-running server cannot inspect another process's changing cwd; clients
must publish root updates or relaunch in their new project.

An independent opened child is not captured by a broad parent index. Plain
non-Git client roots are valid, including empty folders. Intentionally combined
roots remain usable when explicitly opened or selected. Multiple unrelated roots
are ambiguous; no state is created before a single scope is resolved.

Handshake and tool enumeration do not index or load models. The first knowledge
query prepares local metadata and schedules initial code-only indexing. A reply
may contain `result.status = "indexing"` and `result.ready = false`; retry the same
query. Empty or unsupported-file-only folders remain retryable when supported
source arrives. They do not repeatedly launch empty indexers.

Bootstrap uses private staging, a workspace bootstrap lock, a shared code-writer
lease, at most two indexing threads, a conservative 50,000-file/512 MiB discovery
ceiling, and a five-minute deadline. It honors exclusions, propagates discovery
errors, disables embeddings for that job, and preserves existing settings.
Overflow and changed-configuration checks precede publication. Existing nonempty
indexes are never force-rebuilt to recover a failed read. Larger or incomplete
indexes receive explicit recovery guidance. These are bounded safeguards, not a
transaction over arbitrary external filesystem changes.

`workspace` (ID or alias) and registered `project_path` are mutually exclusive
per-request overrides. Arbitrary tool paths cannot authorize new indexing.
Overrides never alter the next request's default. HOME, filesystem roots, and
broad system directories are not automatic project choices.

## Automatic freshness

New code-only workspaces continue with native source watching. Bounded catch-up
handles offline edits on connection or writer election. Watches are installed
between catch-up passes to close the scan/registration race. Warm searches never
repeat those source inventories.

One OS-backed lease elects the writer for each physical code index. Other clients
follow its committed changes and can take ownership after it exits. Updated CLI
indexing, MCP reindex, and HTTP/HTTPS watchers use the same lease. It lives outside
replaceable index contents and is held through actual blocking write completion.
A competing explicit force rebuild is refused before clearing storage. Older
binaries and direct third-party storage writers are outside this protocol.

Root settings/ignore changes trigger revalidation and reader reload. Freshness is
reported by `get_index_info`: watching, following, refreshing, disabled,
semantic-manual, or unavailable. Failed refresh is not presented as a current
empty graph. Explicitly disabled watching is respected. Existing semantic
configuration or data requires explicit semantic maintenance rather than silently
spending inference or replacing embeddings with a code-only generation.

## Runtime and performance

Admission covers discovery, diagnostics, refresh, and queries: sixteen requests
per router and four blocking filesystem jobs. Blocking closures retain permits
until actual completion even when the caller cancels. Timing out an await cannot
forcibly interrupt a running kernel filesystem operation.

Workspace loads are shared. Short lifecycle locks do not cover complete RPCs;
each reader allows four concurrent queries. Bad arguments preserve backend error
categories and do not restart a healthy reader. Cancellation targets only the
individual request, not the peer shared with other queries.

One-second metadata snapshots coalesce reader refresh. Four document metadata
checks also invalidate cached store absence and changed collection state without
walking or hashing document sources. Scope/registry checks still perform I/O;
no zero-I/O or unmeasured speedup claim is made. Refresh is bounded consistency,
not a transactional snapshot of arbitrary external writes.

Strict lite facade loading keeps lexical/statistics queries model-free. Semantic
facilities initialize on demand; document facilities load only on relevant calls.
Initialization survives cancelled waiters and retains errors. No failed load
manufactures an empty replacement index.

At most four reader processes are owned per router. Physical permits remain held
until observed child exit, including eviction. Bootstrap has a separate one-job
limit per router. Active work pins slots. Eviction/shutdown relinquishes a reader's
watch; another connected follower can elect itself. Cleanup waits for owned writes
and child processes rather than treating map removal as process termination.
Separate stdio clients retain separate pools; this is not a machine-wide daemon.

## Recall and document boundaries

Imported conversation recall is automatically bound to the canonical opened
project, not an inherited legacy label. The companion importer derives the same
namespace without manual workspace settings. Import remains an explicit choice
of one JSONL transcript; no private transcript tree is discovered automatically.
Reply scope is verified before rendering text, with bounded subprocess output
and a deadline. See the [workspace guide](workspaces.md) for usage.

Document storage, configured roots, persisted source paths, and returned hits are
validated against the code workspace before model use and at materialization.
Foreign or corrupt auxiliary state is refused without poisoning independent
lexical tools. Document indexing remains explicit. Newly published document
metadata invalidates a reader that previously cached no document store.

## Existing HTTP/HTTPS session shutdown

Project-bound network serving retains authentication, workspace ACLs, origin/host
checks, session ownership, and replay. Session-map locks are not held over handle
RPCs. DELETE/expiry cancels a session independently of full or unpolled SSE queues.
Closing a session drops receivers and releases permits even when the client never
polls again. Per-session capacity cannot consume another session's stream budget.
This is a lifecycle fix, not a new multi-workspace network endpoint.

## Output and remaining extensions

Every routed reply includes readable and structured workspace identity. Keep that
identity on symbol-ID follow-ups. Raw IDs are not globally unique; generation-
qualified references are future API work. The router does not partition graph
resolution between repositories intentionally combined inside one workspace.

Ordinary `codanna serve`, explicit configuration, and manual indexing retain their
separate modes. This router exposes tools, not unscoped mutations, resources,
subscriptions, or HTTP serving. Tool annotations reflect first-use cache writes
rather than advertising strict read-only behavior.

## Verification

```bash
cargo test --locked --all-features --test workspace_mcp --test workspace_bootstrap
cargo test --locked --all-features --test workspace_auto --test workspace_cli
cargo test --locked --all-features --test workspace_live --test workspace_recall
cargo test --locked --all-features --lib workspace_
```

Fixtures use synthetic temporary projects, cleared credentials, disabled/mock
inference, and execution deadlines. Coverage includes fresh roots and changes,
independent code/recall, edit/create/delete convergence, offline catch-up, follower
visibility before handoff, writer contention, ignore changes, document cache
invalidation, concurrent RPC cancellation, stream limits, and full/unpolled SSE
teardown. Consult exact-commit CI results; test source alone is not a passing run.
