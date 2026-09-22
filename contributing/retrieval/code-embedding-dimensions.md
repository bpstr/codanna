# Code embedding dimension contract

The code semantic journal can read dimensions **1 through 4096**, inclusive.
This is a persistence contract, not the optional embedding cache's larger limit
and not a universal constraint on the shared document embedding backend.

## Failure prevented

Before this change a remote code backend could accept an unsupported dimension,
spend source-inference work, and publish a first checkpoint that the journal
reader rejected on reopen. A failed first save could also leave directories or
other artifacts. Forced indexing initialized the backend after clearing the
previous lexical index, so discovering an invalid backend then was too late.

The regression baseline at `2226d36475b3a52b95245ad1f86599802b191d0d`
ran in [35781934800](https://github.com/bpstr/codanna/actions/runs/35781934800):
**two controls passed and five rejection/preservation contracts failed**.
Failures include accepted invalid CLI/planner configurations and an artifact
created by a rejected first save. An assertion after an earlier failed assertion
is not claimed as reached; the source call ordering independently identifies the
force-clear exposure.

## Validation boundaries

One code-specific validator is used by the code backend factory, source embedding
stage, journal reader and writer, and read-only source planner.

An explicit `CODANNA_EMBED_DIM` overrides `semantic_search.remote_dim`. A known
invalid value is rejected before constructing a remote client or probing it.
An omitted dimension remains unknown to the planner. Actual indexing may need a
single initialization probe; an unsupported result is rejected before embedding
any source or clearing the old code index. The validated backend is reused after
facade construction instead of sending a second probe.

Indexing with semantic search explicitly enabled no longer silently completes a
lexical-only rebuild when preflight cannot initialize the requested backend.
Disable semantic search deliberately for a lexical-only index. Disabled semantic
mode does not contact an endpoint merely because an unused dimension is invalid.

A loaded semantic generation must still match the prepared backend's dimension
and complete model/input identity. This change is not permission to reuse vectors
from a different model or representation.

Before any first/empty/replacement checkpoint creates a directory, lock, vector
file or manifest, the writer validates its metadata dimension. Existing invalid
stores are not rewritten to make them loadable. Supported 1- and 4096-dimensional
checkpoints retain their existing format and delta/reopen behavior.

## Planner and document boundaries

An unsupported configured code dimension is a configuration error, not a
successful inventory with zero inputs or cache misses. The planner exits nonzero
with the supported range and leaves workspace contents and modification times
unchanged. It does not query a provider to resolve an unknown dimension.

The generic `build_embedding_backend` remains usable by document embedding code.
Only the code facade uses `build_code_embedding_backend`. A 4097-dimensional
joined-loopback control exercises that generic backend without code-journal
persistence; this does not claim a new document storage format or maximum.

## Regression evidence

`tests/code_dimension_contract.rs` exercises actual CLI children, public
persistence APIs and a joined loopback endpoint. Child environments exclude
provider credentials. Controls cover both comment and body source policies,
configured 0/4097/16384 values, environment precedence, an unknown oversized
probe, old-index byte/directory/mtime preservation, a missing initial index,
disabled mode, invalid first/replacement saves, unsupported stored manifests,
and the supported lower/upper boundaries.

Tests count initialization probes separately from source texts. No real provider
or production index is used. This is format/cost safety, not a semantic relevance
benchmark or a promise that a real rebuild is free.

## Compatibility and remaining work

No dimension limit is increased and no index-format migration or re-embedding is
introduced by this validation change. Existing invalid code stores must be kept
for investigation or recovery rather than overwritten automatically.

The body-input/cache integration and lexical/scope/related-code integration remain
separate release slices. Independent relevance labels, controlled latency/RSS,
all-branch integration, and source-build planner packaging are not qualified by
these dimension contracts. The poor paraphrase results are unchanged.
