# Incremental indexing contracts

An unbounded indexing pass should produce the same declarations and resolved
graph as a fresh index of the same admitted source files. The original eight
lifecycle cases in `tests/adversarial/index_lifecycle/review_index_lifecycle.rs`
exercise missing and restored imports, an import-only barrel update, member
identity, multi-root edits, subsecond timestamps, malformed ignore rules, and
`max_files`.

`tests/index_lifecycle_regressions.rs` adds full graph comparisons after module
creation, edits, deletion, restoration, relocation, and import retargeting in
both root orders. It also checks persistence, root discovery, unrelated module
isolation, bounded inventories, and deferred consumer repair. These tests use
temporary source trees, disable semantic search, and require no model or remote
provider. The measured results belong in the adversarial result checklist;
this document describes the intended contracts and implementation choices.

## Dependency evidence and partial runs

Raw import records remain in the index even when they currently resolve to no
symbol. Incremental indexing scans those records once per change wave, builds a
reverse import map, and follows the transitive importer closure. Import-only
barrels participate through their file registration and imports. TypeScript and
JavaScript additionally retain explicit export surfaces. Cleanup deletes the
old generation's imports and exports by file ID before replacing its rows.

Relative imports use a normalized source-relative path. Normalized module
imports are matched against configured roots, roots actually walked, and roots
restored from metadata. Matching may schedule an extra namesake importer; the
language resolver must still establish the resulting symbol identity. This
avoids reparsing every source file when a target changes, while repairing
unchanged consumers that have never had a resolved edge.

The dependency matcher uses normalized import paths and file namespaces. It is
not a substitute for language-specific project resolution. Arbitrary module
aliases or provider paths that have no normalized correspondence to an indexed
file remain a separate resolution capability. The current reverse map is built
from persisted metadata for each change wave; a generation-checked cached map
is a possible performance improvement for very large repositories.

`max_files` applies to the actual input inventory across the whole multi-root
run. Roots retain argument order; files within a root use sorted paths, and
overlapping paths are deduplicated. A partial inventory cannot prove that an
omitted file was deleted. The same bounded policy applies to preflighted network
snapshots, which must never reopen additional source paths after admission.

When a bounded run changes a dependency used by an omitted file, it removes
that file's outgoing relationship output and persists a `pending_resolution`
record. Declarations remain searchable, but their incomplete graph is kept
free of old dependency-derived edges. `DocumentIndex::get_pending_resolution_paths`
exposes that state for diagnostics. The next unbounded indexing pass drains
these records even when every source hash is unchanged. Records survive a
save/reopen cycle and clear only after successful resolution and persistence.
An admitted pending file may also be repaired by a later bounded run. A
zero-file inventory leaves pending sources unopened.

Per-root `files_indexed` and `symbols_found` statistics are collected before
shared dependency replay. They report directly discovered or admitted work and
currently omit the extra importer reparses performed during that replay.
Bounded runs still honor their exact admitted-file limit because replay is
deferred. Graph-equivalence tests compare final index state independently of
those progress counters.

## Rebinding identity

Captured incoming edges may survive an ordinary body or line edit only when
the target's recorded file, name, kind, module, and owner still match. Class
owners and local function parents are mandatory identity boundaries. When an
old or new owner contains multiple same-named declarations, a unique recorded
signature is required. Ordinals and old line numbers are diagnostic values;
they do not identify a replacement after declarations move or reorder.

Rebinding also verifies that the source symbol still exists. Another root may
have replaced it after capture. Its new parse owns the replacement output, so
restoring an edge from the obsolete ID would create an orphan. Go structural
implementation relationships are rebuilt from live method sets and bypass
generic captured-edge restoration.

## File discovery and live policy changes

Stored modification times use nanoseconds. Older second-valued registrations
cannot compare equal to modern timestamps and therefore take the hash path.
A changed timestamp, a whole-second timestamp that may come from a coarse
filesystem, an unknown timestamp, or a future timestamp requires content
verification. A precise, equal timestamp in the past retains the stat-only
shortcut. Tools that deliberately preserve an exact timestamp can defeat that
heuristic; a dirty-file watcher event and explicit single-file indexing read
the bytes and compare hashes.

Malformed ignore rules can be attached to otherwise valid walker entries.
Both discovery modes treat those attached errors as failures before using the
walk to infer deletions. Nested `.gitignore` and `.codannaignore` events enter a
settled whole-root reconciliation wave. The watcher registers traversable
source directories even when all source files inside are currently excluded,
so a later policy change can make them visible again. These directory walks
occur at watcher preparation/reload, rather than during queries.

The deterministic watcher unit tests exercise explicit policy events and the
reconciliation path, including exclusion and reinclusion for both policy file
names. They do not claim coverage of operating-system-specific notification
delivery or ignored ancestor directories that the walker cannot traverse.
