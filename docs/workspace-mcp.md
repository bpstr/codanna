# Isolated local MCP workspaces

Implementation is under review in PR #34. See the PR for the exact tested commit
and CI outcome. The normal entry is one reusable configuration:

```json
{"mcpServers":{"codanna":{"command":"codanna","args":["workspace","serve"]}}}
```

Open a project in the client and query it. No workspace registration command,
per-project path in MCP configuration, initial indexing command, or repository
inventory is required for supported local layouts. Names are runtime data.

## Selection and first use

Supported client roots take precedence over the subprocess launch directory.
Without roots support, the launch cwd supplies the project context. A server
launched from HOME with no roots can connect and list tools but cannot invent a
project context. An independent project must never fall back to another graph.
A running process cannot observe later cwd changes in a different process;
clients must send root updates or launch the MCP process in the correct project.

Nearest independent checkouts/configurations win. A containing index with source
`.` does not adopt an independently opened child. A plain non-Git client root is
valid, including an initially empty directory; an unrelated ancestor's settings
cannot widen it. An intentionally combined root remains usable when that root
is actually opened or explicitly selected. Multiple unrelated client roots are
ambiguous: no files are created until a single scope is selected.

An unscoped tool call may prepare settings and registration for its validated
local session root. The first knowledge query schedules initial code-only
indexing and can return `result.status = "indexing"`, `result.ready = false`.
Retry the same query shortly. Initialization and tools/list themselves do not
index or load models.

Empty roots and folders containing only unsupported files return `empty`. They
are checked again on subsequent use, so adding the first enabled source file does
not require setup commands. Discovery consults the existing language registry
and language-pack detector without creating parsers or models. A notes-only
folder does not repeatedly spawn an indexer, and a zero-file staging generation
is not published as a ready index. A genuinely indexed file with zero symbols
is still a valid file, not an empty project.

Automatic indexing disables embeddings for that indexing job, preserves existing
settings, and never force-rebuilds nonempty data. New automatic settings disable
semantic search until deliberately configured. It uses a private staging index,
a per-workspace OS bootstrap lock, at most two indexing threads, a 50,000-file /
512 MiB discovery ceiling, and a five-minute job deadline. Discovery counts
unsupported entries toward its resource budget and propagates malformed ignore
rules rather than silently broadening the source set.

The child may inspect one overflow file beyond the normal file limit, but a
generation exceeding that limit is rejected before publication. Configuration is
compared with its setup snapshot before publication, so a changed source policy
cannot publish the old build as current. Failed or oversized jobs return recovery
guidance without replacing an existing index. These checks are not a transaction
across arbitrary external source/configuration edits during a build.

Explicit `codanna index` remains available for larger projects and recovery.
Bootstrap locking coordinates automatic initial builders; it is not a claim
that every legacy CLI writer/watch path uses the same ownership protocol.

`workspace` (ID/alias) and `project_path` are mutually exclusive per-request
overrides. A tool-argument path may select an already registered workspace but
cannot bootstrap an arbitrary filesystem location. Overrides never change the
next call's default. HOME, filesystem roots, and broad system directories are
not automatic project choices. No network listener is introduced.

## Runtime and performance

Queries never run the diagnostic repository inventory. Root selection walks only
applicable ancestors. Root responses are cached only when the client promises
root-change notifications; a notification invalidates future requests. Other
clients are queried each time. MRTR continuation tokens are single-use, expire,
and are bound to the original arguments and root generation.

Admission starts before discovery and diagnostics: sixteen requests per router,
with four blocking filesystem jobs. Blocking closures retain their own permits
after their callers cancel. Filesystem refresh has a separate deadline; a timeout
cannot forcibly interrupt a running kernel filesystem operation.

One shared load per workspace serves concurrent callers. The lifecycle lock is
not held across tool RPCs. Each reader permits four concurrent requests. Bad
backend parameters retain their original MCP error category and do not restart a
healthy reader. Query cancellation targets the individual backend request rather
than closing a peer used by other calls. Generated-router validation that returns
tool-error content also retains that existing wire contract.

Warm readers reuse metadata snapshots for one second. Refresh checks file
metadata; configuration is reparsed and a reader replaced when the corresponding
snapshot changes. This is bounded refresh consistency, not a transactional view
of concurrent external writes. Registry selection is still checked on each
request. Removing recursive scans and model initialization does not mean that
warm routing performs no filesystem operations or that a speedup has been measured.

Readers use strict lite facade loading. Lexical search/statistics do not initialize
semantic models. Existing semantic data loads on the first semantic operation;
documents load on first document/context use. Shared facility initialization
survives cancellation of its first waiter and retains failures rather than
starting duplicate work. No load error creates a replacement empty index.
Recall remains disabled until workspace-specific bindings exist.

The router owns at most four live reader processes. A process permit remains held
until child exit is observed, including during eviction. Initial index jobs have
a separate one-job-per-router limit. Slots are pinned during queries/startup;
idle slots are reclaimed on subsequent activity. Shutdown cancels jobs and waits
for child cleanup. Separate stdio clients still have separate router pools: this
is not a machine-wide shared daemon or an authenticated multi-workspace HTTP server.

## Results, compatibility, and limits

Every tool result carries a visible and structured workspace identity. Keep that
identity on symbol-ID follow-ups. Raw IDs are not globally unique and generation-
qualified references remain future work. Repository-partitioned resolution inside
an intentionally combined workspace is not provided merely by this router.

Ordinary `codanna serve`, explicit configuration, and manual index commands retain
their separate modes. The router exposes tools, not unscoped mutations, resources,
watch subscriptions, or a new HTTP endpoint. First-use cache creation is reflected
in tool annotations; the new mode is not advertised as strictly read-only.

Automatic bootstrap establishes an initial code index; it is not continuous
source watching. Once a populated index exists, later source edits still use the
existing explicit indexing/watching modes. Reloading a published index generation
is different from indexing changed source files.

## Verification

```bash
cargo test --test workspace_mcp --all-features
cargo test --test workspace_bootstrap --all-features
cargo test --test workspace_auto --all-features
cargo test --test workspace_cli --all-features
cargo test --lib workspace_ --all-features
```

The MCP suite includes fresh non-Git projects with identical symbol names and no
settings/indexing calls; separate cwd-based sessions; a broadly indexed parent;
an empty project receiving its first file; invalid-argument recovery; and root
cache invalidation. A deterministic controlled-reader test proves concurrent
RPC overlap and cancellation of one request without closing another's peer.
Lexical queries are tested against a configured local mock embedding endpoint
and must make zero requests.

The bootstrap suite covers unsupported-file-only projects receiving their first
source, preserved empty source configuration and ignore rules, empty/overflow
publication rejection, changed configuration, and cancellation preserving
existing storage. Fixtures clear credentials and disable inference. Test code
alone is not a passing result; consult commit-specific CI evidence in the PR.
