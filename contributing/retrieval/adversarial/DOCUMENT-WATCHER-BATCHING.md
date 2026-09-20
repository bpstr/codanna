# Document watcher batching

Configured document collections now coalesce source, directory and
`.codannaignore` events before retaining individual source paths. A pending map
holds one entry per affected collection; a dispatch detaches the due entries and
reconciles each collection once through the existing `index_collection` path.
This keeps creation, deletion, recreation, collection overrides and ignore policy
in the same discovery and transaction rules as explicit indexing.

The quiet window uses the configured watcher debounce, capped at two seconds.
Continuous arrivals can move a collection's deadline only as far as two seconds
after its first pending event. The existing 100 ms drain cadence dispatches due
work. This bounds when work becomes eligible, not end-to-end indexing latency:
an earlier mutation, a large collection scan or embedding work can delay it.

Paths also handled by code or configuration handlers retain their existing
per-path route. Document-only source events do not enter that shared debouncer.
The native event channel remains capped at 256 entries. Overflow schedules
filesystem-truth reconciliation, including empty, removed and newly populated
document roots. Detached work never clears events or overflow flags received
while a scan is running.

Ingress rejects events inside the configured index directory, including custom
index paths. Other source and subtree events use the ignore library's incremental
path matcher with the same `.codannaignore` options as collection discovery. This
checks a path and its policy ancestry without walking the collection. Fresh
matchers observe policy changes immediately and avoid retaining historical event
directories. Policy and configured-root events remain eligible. In particular,
broad patterns such as `**/*.json` cannot repeatedly schedule indexing from an
ignored generation or state file.

A failed collection remains pending even if no later source event arrives.
Retries begin after 250 ms and double to a 30 second cap. New events preserve the
retry deadline while updating the pending work. Backoff is checked by the regular
drain; no retry sleep blocks event handling or cooperative shutdown. A successful
batch publishes one index-refresh notification covering every source that the
collection scan changed.

Root watches are repaired before the corresponding truth scan. The parent watch
survives a root deletion, and a recreated subtree gets new native registrations
even when its path strings already appear in the registry. Nested collection
roots also observe inherited `.codannaignore` policy at workspace ancestors.
Document watch roots, including those parents, do not expand the code roots
passed to indexing during an index-refresh notification.

The overlap controls also exposed workspace-relative code actions being read
against the process working directory. Mutations now resolve those filesystem
paths against the watcher workspace and delete the corresponding stored source
key. Notifications retain their original path representation.

When a removal event covers a configured code root, the watcher verifies its
absence with `symlink_metadata` before cleaning that root's stored sources in one
batch. A sibling root remains intact. Dangling symlinks and metadata errors do
not count as verified absence; ordinary discovery still rejects missing or
unreadable roots instead of treating a failed scan as an empty inventory.

## Reproducible workload comparison

`document_burst_reduces_measured_collection_scans_and_file_checks` writes 64 local
one-chunk sources. It executes the previous one-collection-call-per-event pattern
against that settled fixture and then dispatches the same 64 source events
through the collection queue. The old pattern is executed by the test; it is not
estimated from the number of events. Discovery-phase callbacks count baseline
scans, and returned `files_processed + files_skipped` count completed file checks.
The new dispatch reports the same work counts and logs elapsed milliseconds.

The regression requires these deterministic counts:

| Work | Prior dispatch pattern | Collection dispatch |
| --- | ---: | ---: |
| Source events | 64 | 64 |
| Collection scans | 64 | 1 |
| Completed file checks | 4,096 | 64 |
| Live source chunks | 64 | 64 |

The test prints elapsed times with `--nocapture` for inspection but does not gate
on time or claim a general speedup. It uses lexical storage; it does not measure
provider latency, real-model quality, filesystem read bytes or cold-cache disk
I/O. Watch-directory traversal is separate from the collection content checks.
Failed scans can perform partial work that the current store statistics do not
report.

The first measured reconciliation sample reported 683,994 microseconds for the
prior pattern and 38,809 microseconds for the collection dispatch. The timers
cover the reconciliation calls; the latter starts after routing the event burst.
These times are descriptive samples, while the work counts above are the
regression contract.

## Controls and commands

`src/watcher/document_batching_tests.rs` adds direct-event and explicit-clock
controls for a 4,096-path burst with only two pending collections, a continuous
event stream, root recreation with concurrent source and ignore-policy changes,
ancestor ignore policy, idle retry, bounded retry backoff, queued events and
overflow during a blocked local mock embedding call, overlapping source handlers,
actual code/document ownership overlap, code-root isolation on reload, and
shutdown with a future retry deadline. Additional controls cover relative code
creation/deletion notifications, ignored broad-pattern artifact events, and
custom managed-index event exclusion. The overlap control also requires an
unrelated sibling code source to survive deletion, and a Unix control proves a
dangling symlink does not enter the observed-root cleanup path. The earlier
document watcher tests retain their source creation, deletion, recreation and
collection-override assertions.

Run these without provider credentials or embedding environment overrides:

```bash
cargo test --locked --lib watcher::unified::document_ -- --nocapture
cargo test --locked --lib watcher::unified::
```

All inputs are temporary local fixtures; the embedding controls return fixed
vectors or an injected failure. They do not load a model, use a provider or wait
for operating-system event delivery.

## Remaining limits

Collection definitions and chunking defaults remain startup snapshots. Editing
document collections in the configuration still requires a server restart;
these tests do not claim document-configuration hot reload. Inherited ignore
policy above the workspace boundary is not newly watched, and collections
outside the workspace retain their nearest existing parent watch.

Each due collection still walks its entire configured inventory and checks its
source contents. This change removes repeated work within a burst; it does not
provide per-source incremental discovery. The map is bounded by configured
collections, with an additional detached batch while scans run. A collection can
still be arbitrarily large, and the shared code/configuration debouncer retains
its existing per-path behavior. Document mutation continues to hold the store's
write guard for the scan, so competing users of that shared store can wait for
publication.
