# Scoped search snapshots and external-root identity

Follow-up to #53 for T07 in #43. Query-time only: no index schema change,
embedding work, source-tree scan, provider access, or implicit rebuild.

## Correctness contract

A scoped search uses one pinned Tantivy Searcher for its file registrations,
file-ID filter, TopDocs collection, and result hydration. A reload during a
query cannot make its scope refer to a different snapshot from its results.

Display-path shortening is not workspace identity. A separately indexed
external checkout may display a relative `src/active` path, but it must not
enter an explicit `src/active` scope in the configured workspace. Legacy
absolute stored paths under that workspace still work. External paths remain
available to unscoped search.

**Explicit `.` means the configured workspace.** It is no longer a no-filter
shortcut when external roots are registered. In-workspace-only indexes retain
root/unscoped equivalence. Scopes are retrieval constraints, not authorization.

Exact file matches and slash-delimited descendants occupy separate sorted
ranges: a backup file or `calendar-old` directory cannot broaden a scope.

## Bounded warm inventory

One sorted file-registration inventory is shared by all prefixes for a reader
generation. Cache hits reuse the same Arc rather than reloading every stored
file document. Prefix lookup uses binary searches followed by the matching
ranges. File-ID terms still grow with the selected subtree.

Retained cache admission is limited to **25,000 files and 4 MiB logical
storage**. The byte calculation includes path/vector capacity, not allocator
metadata or complete process RSS. Larger inventories return complete results
but are not retained; their cold temporary construction is not a 4 MiB peak
memory guarantee. A slow older reader cannot replace a newer retained inventory.

## Executed baseline and candidate

[Run 35735214232](https://github.com/bpstr/codanna/actions/runs/35735214232)
started at `04079236a5658d6611b912bcceeec1d809e0d453` and applied a reviewed,
base-blob-checked wiring patch before candidate execution.

- Three public external-root contracts: **1 passed / 2 failed before**, then
  **3 passed / 0 failed after**. The source expectation was unchanged; formatting
  was mechanical. Failures directly observed external checkout names in the
  explicit subtree/root results.
- Seven existing facade/MCP scope tests and four real CLI scope tests: **11 passed**.
- Three new pinned-snapshot/cache contracts: **3 passed**.
- **17 selected candidate tests passed**, formatting and strict
  all-target/all-feature Clippy passed. No ignored tests in these groups.

The warm inventory fixture registered 2,048 files and queried eight distinct
prefixes: each returned 256 file IDs and reused one inventory allocation.
Reported logical retained bytes: **101,293**. Another fixture registered 20,000
long paths, exceeded the retained-cache budget, and still returned every ID.
These are deterministic fixture observations, not production latency/RSS claims.

The first run 35733936035 reached the same expected 1/2 baseline but stopped
because the harness inherited shell errexit. The corrected harness captured
that failure explicitly. It was not a candidate behavior failure.

### Verified candidate source hashes

| Path | SHA-256 |
| --- | --- |
| `src/storage/tantivy/query.rs` | `513201f6f23a7d32168ecafb8e4b74ea9fab85bcfa97999fcef86258af7737ff` |
| `src/storage/tantivy/mod.rs` | `a1e55a7d5c6745f4ee4626ff0a6a191237dcb9f0bc505b5e98db7fab83a1375e` |
| `src/storage/tantivy/scope.rs` | `89969301b9711d27dd8cc1cdaa14f69e1da45aac9b1e6ddb638de09d87fe5d71` |
| `tests/scoped_search_snapshots.rs` | `9f5a3fe0babd121caf8ba0f8999c50341acbbf5f5ec0292fc9ba8eb411b1f7a5` |

These exact candidate blobs are committed through the connected GitHub app.
The temporary source-handoff workflow is removed; retained CI is read-only and
runs the committed files without patching them. Final-head execution is recorded
on the PR, not inferred from this staged candidate.

## Remaining boundaries

Cache generation is reader-local to the owning DocumentIndex, not a persisted
cross-index generation. Concurrent cache misses can duplicate temporary work.
Broader canonical/symlink/native-path aliases, cold oversized-inventory peak RSS,
and full combined-branch release checks remain separate. This does not change
ranking weights, source diversity, embedding eligibility, or claim completion
of every T07 acceptance case.
