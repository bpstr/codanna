# Document embedding storage and recovery

This report describes the document-storage changes made after
`dd8e247`. The earlier [document embedding review](../adversarial/document-embedding-review.md)
is historical evidence for its stated baseline and repair run; its results and
remaining-work list describe that earlier implementation. This report explains
the subsequent generation, recovery, compaction and diagnostics behavior. Measured
before/after churn results are reported in the [storage measurements](STORAGE-MEASUREMENTS.md).

The implementation is in [store.rs](../../../src/documents/store.rs),
[generation.rs](../../../src/documents/generation.rs) and the bounded copy helpers
in [vector/storage.rs](../../../src/vector/storage.rs). The public indexing and
search method signatures remain unchanged.

Managed index files are excluded from document discovery. Every `DocumentStore`
excludes its own base directory, and `open_from_settings` also excludes the shared
`settings.index_path` root, covering both code and document artifacts. These
directories are pruned before traversal; canonical file paths are checked again
so explicit includes and file aliases cannot bypass the exclusion. A broad
`**/*.json` collection can therefore retain intended JSON sources without
ingesting its own state or generation metadata, even when a custom index location
has no ignore rule. A subsequent collection reconciliation also removes any
previously tracked managed files through the normal deletion transaction.

## Publication and recovery

Tantivy's atomic metadata commit now identifies one immutable document generation.
The commit payload has the form
`codanna-documents-v1:gen-<unique-name>.json`. That generation file contains the
source states, collection ownership, chunking fingerprints, embedding identities,
next chunk ID and the list of immutable vector segments used by the commit.

All paths below are relative to the configured index's `documents/` directory.

| Artifact | Purpose |
| --- | --- |
| `tantivy/` | Chunk metadata and the authoritative generation name in its commit payload. |
| `generations/gen-<unique-name>.json` | Immutable source state, vector segment references and vector write counters for one publication. |
| `vectors/segment-<unique-name>/segment_0.vec` | An immutable vector segment. A changed transaction initially writes only its new embeddings. |
| `state.json` | A compatibility mirror of committed source state. Modern recovery uses the generation named by Tantivy. |
| `next-chunk-id.json` | Durable upper bound for reserved chunk IDs, independent of whether the transaction commits. |
| `generation.lock` | Serializes document mutations and protects their unpublished files from collection. |
| `publication.lock` | Protects opening and publishing a consistent committed snapshot. |

A collection index, single-file reindex, file removal or collection deletion uses
the same transaction:

1. Hold the document writer lock and check that this store still represents the
   committed generation. If another writer has advanced it, return an error asking
   the caller to reopen the store before indexing.
2. Stage Tantivy changes and source-state changes. Write new embeddings into a
   private segment, with collection input spooled to a managed temporary file.
   Embedding errors leave the previous committed generation available.
3. Take the publication lock. Flush the new vector segment and relevant directory
   entries. If compaction is due, write and flush a replacement containing live
   vectors.
4. Write and flush the immutable generation JSON and its directory entry.
5. Commit Tantivy with that generation's name in the payload. This is the point
   that decides whether reopening selects the old or new generation. Tantivy's
   commit implementation also synchronizes its directory.
6. Reload the store's reader, install the matching vector handles and atomically
   replace the `state.json` mirror. Release the writer and collect obsolete files.

Before step 5, reopening selects the previous generation. After step 5, reopening
selects the new source state and vectors even if the process never replaced
`state.json`. A missing or stale mirror is repaired when possible. An unwritable
mirror can make the indexing operation report an error after its generation has
committed; the committed results remain available and reopening can defer mirror
repair. Retrying indexing after correcting the mirror path preserves one live
generation.

Handled failures restore uncommitted source tracking and roll back the Tantivy
writer. If a metadata commit succeeded but a later operation failed, recovery
loads the generation named by the actual commit. Error paths release the writer.
If authoritative recovery itself fails, cleanup is deferred so it cannot remove
files that might belong to a committed generation.

Generation and legacy-state JSON reads are bounded to 128 MiB before decoding;
the small ID reservation file is bounded to 64 bytes. A generation with an
unknown payload format, invalid file name, duplicate segment reference, more than
eight segments or mixed vector dimensions is rejected. A referenced artifact that
is missing or malformed produces an error. Recovery does not guess a replacement
generation from directory order or from the compatibility mirror.

## Query snapshots and independent writers

Document readers reload explicitly. A query snapshot captures both a Tantivy
`Searcher` and shared handles to the immutable vector segments. Every filtering,
ranking and metadata-hydration operation uses that pinned searcher. Reindexing or
compaction cannot advance an existing query's metadata independently of its
vectors.

A new store takes the publication lock only while opening the committed state.
It can open that state while another process performs embedding work under the
writer lock. It attempts the writer lock without waiting before collecting
orphans; if a build is active, it leaves that build's staging files alone. Opening
can still wait for the publication phase, which includes compaction and durable
writes. Source scanning and embedding generation are outside that phase.

An already-open store continues to represent its captured generation if an
independent writer commits a later generation. Reopen it to adopt the later
state. Its next mutation detects the mismatch and returns an error rather than
overwriting newer source tracking. The persistent server's own mutations update
its store after each successful commit.

Workspace restrictions remain attached to query snapshots. The updated privacy
regression checks both relevant views: a previously captured query retains its
local generation, while a fresh query containing foreign provenance is rejected.
Reopening and binding a contaminated index to that workspace is also rejected.

## Chunk ID reservation

Chunk IDs are reserved in blocks of 4,096. Before allocating the first ID in a new
block, the upper bound is atomically written and synchronized. A reopened store
starts at the greater of its committed next ID and that durable reservation.
IDs consumed by a failed batch or a terminated process are therefore not reused.
Unused IDs at the end of a reserved block can be skipped after reopening; IDs are
monotonic identifiers, not a count of live chunks.

The allocator checks the nonzero `u32` range. Exhaustion returns a rebuild error
instead of wrapping around and colliding with existing IDs. The reservation file
is part of the store and must be preserved when relocating or restoring it.

## Incremental vectors and compaction

An ordinary embedding transaction appends a new immutable segment without copying
the historical vector file. Semantic scoring visits only the segments referenced
by its generation and scores the IDs selected by the metadata filters.

Compaction runs when either the referenced physical record count exceeds twice
the live embedded chunk count or the segment count exceeds eight. It copies live
records into one replacement segment in batches of at most 64 vectors, omits
deleted/replaced IDs and deduplicates legacy IDs using their first occurrence.
When no live embedded chunks remain, the new generation references no vectors.
This keeps each successfully published active generation at no more than twice
its live vector count and at no more than eight segments.

These bounds apply after a successful mutation. An opened legacy index can exceed
them until its next mutation invokes compaction. Compaction can temporarily keep
the old segments, the new delta and the replacement on disk together. Complete
generation state is still serialized on publication, and an unchanged collection
scan can still publish source state and metadata. The improvement removes the
per-edit copy of all historical vector payloads; it does not make every write
proportional only to the changed source text.

Obsolete generations, unreferenced vector directories and managed abandoned
spool/state temporary files are collected after publication or during a safe
reopen. A crash before commit can leave these files behind, but their presence
does not make them part of the active generation. Failed deletions are deferred
and retried on a later cleanup opportunity.

An open query retains its old memory maps until it finishes. On Unix, obsolete
files can be unlinked while those maps remain usable, but the underlying blocks
remain allocated until the maps are released. On platforms that prevent deletion
of mapped files, the obsolete file can remain visible until a later cleanup.
Consequently, the active-generation bound is not a bound on temporary write
space, all storage held by arbitrarily long-lived queries, or failed filesystem
deletions. File counts and `.vec` file lengths also exclude Tantivy, source-state
JSON, the embedding cache and filesystem allocation overhead.

## Diagnostics

The public method
`DocumentStore::embedding_diagnostics(&self) -> EmbeddingDiagnostics` requires no
model load or embedding request. `EmbeddingDiagnostics` is exported from
`codanna::documents` and is serializable. The CLI includes it as `embedding_index`
in the JSON returned by `documents stats`.

```bash
codanna documents stats docs
codanna documents stats docs --json
codanna documents status --json
```

Replace `docs` with an indexed collection name. Collection counts refer to that
collection; `embedding_index` describes the shared document index across all
collections. `documents status` continues to describe recent run heartbeats and
progress, independently of the generation diagnostics.

| Diagnostic field | Meaning |
| --- | --- |
| `generation` | Generation name represented by this store, or `null` for a legacy/unpublished store. |
| `identity` | Persisted or configured backend/model/input-policy identity. Configured identities incorporate the explicit model revision and preprocessing policy; endpoint details are digested. |
| `vector_segments` | Number of immutable segments referenced by this store's generation. |
| `physical_vectors` | Physical records in those segments, including records that have become obsolete since their segment was written. |
| `live_vectors` | Chunks whose source states are marked as embedded. |
| `unembedded_chunks` | Stored source chunks whose source states are not marked as embedded. |
| `new_vector_bytes` | Vector record payload bytes written for newly embedded chunks during the last publication. |
| `compacted_vector_bytes` | Vector record payload bytes copied during compaction during the last publication. |

The two byte counters are per publication, not cumulative, and use
`records × (4 + 4 × dimension)`: a four-byte ID plus float components. They exclude
segment headers, filesystem allocation, JSON, Tantivy writes and temporary input
spools. A publication without vector changes can report zero for both counters.

Completeness counts reflect source bookkeeping. Opening a configured semantic
store additionally checks the actual required vector IDs without decoding or
scoring the full vectors. A missing required ID clears the completeness marks so
indexing can backfill; an unchanged physical record count cannot hide that gap.
The CLI statistics path opens without a model, so its bookkeeping counts are not
an exhaustive corruption audit. Invalid incoming vector lengths/nonfinite values
are rejected before publication, and scoring rejects a nonfinite persisted value
instead of ranking it.

## Migration and recovery operations

A pre-generation store without a Tantivy generation payload opens through its
existing `state.json` and, when present, `vectors/segment_0.vec`. The first
successful mutation writes a generation manifest. Compatible legacy vectors can
be referenced as a legacy segment until compaction replaces them. Storage-format
migration does not require re-embedding an otherwise compatible, complete index.
It also cannot establish the consistency of an older store that was already
interrupted between independent artifact writes; rebuild such a store if its
prior completeness is uncertain.

Embedding compatibility is checked separately. A changed backend/model/input
identity or dimension is rejected before semantic query vectors can be mixed
with the stored corpus. The complete-input policy and explicit model revision
participate in configured identities, so adopting those changes can require a
fresh document index even when the model name and dimension are unchanged.
Legacy vectors without a recorded identity also require rebuilding for semantic
use. With `semantic_search.enabled = false`, intact metadata remains available to
lexical queries without loading a model.

To refresh or retry the currently configured compatible store:

```bash
codanna documents index --collection docs
codanna documents index --all
codanna documents search "authentication flow" --collection docs --limit 5 --json
```

`--force` replaces selected source chunks under the existing compatible identity;
it does not override an identity mismatch. To rebuild with an incompatible model
or input policy, select a new index directory and index the configured
collections there. For the CLI, the top-level `index_path` setting chooses the
shared index root, and documents live beneath its `documents/` child; changing
that setting also changes the code-index location. Keep a consistent copy of the
previous index when a rollback is needed. The library API can instead open a new
document-store base path directly.

Keep the Tantivy directory, generation JSON, referenced vectors and ID reservation
together when copying or restoring a store. A copy taken while publication is
active needs an appropriate filesystem snapshot or quiesced writer. Copying only
`state.json` is insufficient. Old binaries do not implement generation publication;
alternating old and new writers against the same store is unsupported, and there
is no downgrade migration.

## Verification and precise limits

[generation_tests.rs](../../../src/documents/generation_tests.rs) uses fixed local
vectors. Its subprocess helper calls `std::process::exit(86)` at a selected
boundary, without running Rust destructors. The hooks exist only in the unit-test
build and do not expose a production termination switch.

The termination fixture executes 19 cases: seven for a source update, six for
file removal and six for collection deletion. Each publication action covers
before/after vector publication, before/after Tantivy commit and before/after
state-mirror replacement. Updates also terminate after a staged embedding batch,
exercising abandoned input-spool and vector-segment cleanup. Every case reopens
the store, checks one matching live generation and unaffected collection data,
then retries safely.

Other deterministic controls exercise a failing second embedding batch, durable
ID reservation, stale/missing/unwritable mirrors, malformed references, missing
vector IDs, nonfinite stored values, opening queries during a build, and 100
updates with an original query kept alive through compaction and reopen.

Run from the repository with provider credentials, `.secrets` and
`CODANNA_EMBED_*` overrides absent from the test process. These targets use local
mock vectors or bounded loopback fixtures; do not opt into ignored provider/model
tests.

```bash
cargo test --locked --lib documents::store::generation_tests -- --nocapture
cargo test --locked --lib documents::
cargo test --locked --test document_embedding_regressions
cargo test --locked --test document_backend_regressions
./contributing/scripts/quick-check.sh
./contributing/scripts/full-test.sh
```

The subprocess tests establish recovery from termination at the listed completed
operation boundaries. They do not simulate an electrical power cut, torn sectors,
filesystem corruption, a kill at every instruction inside a filesystem call, or
arbitrary interference from another program editing persisted files. File and
directory synchronization establish the intended ordering on filesystems that
honor those operations. Unix directory entries are explicitly synchronized;
Windows does not use that directory-fsync path, and power-loss durability there
has not been established by these tests. Network and unusual filesystems require
their own validation of locking, atomic replacement and flush semantics.

Fixed vectors prove storage consistency, bounded active vector growth and correct
reuse of a captured generation. They do not establish real-model relevance gains
or a production latency improvement. Releasing the Tantivy writer after each
mutation allows independent writers to proceed, and can add setup cost compared
with retaining one writer for the lifetime of a store.
