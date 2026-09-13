# Codanna roadmap

This roadmap records planned Codanna capabilities that are not yet complete.
Items remain proposals until their implementation and acceptance criteria are
verified. Completed work should move to the changelog rather than remaining as
an open roadmap item.

## Index planning and dry runs

**Status:** Proposed

Make `codanna index --dry-run` an accurate, side-effect-free preview of the work
that a real indexing run would perform.

### Planned behavior

- Report the total number of files that would be indexed, with separate counts
  for new, modified, renamed, and dependency-invalidated files.
- Report unchanged files and files that would be removed separately from the
  indexing total.
- In `--force` mode, count every eligible file; for an incremental run, count
  only files that would actually be processed.
- Apply the same supported-language, ignore, root, overlap, and `--max-files`
  rules as the real indexing pipeline.
- Handle both directory and single-file inputs without indexing either one.
- Fail when discovery is incomplete instead of presenting a partial count as a
  complete plan.
- Avoid loading embedding models, initializing provider integrations, or doing
  other work that is unnecessary to calculate the plan.

### Implementation direction

- Introduce a shared index-plan representation used by both dry-run reporting
  and real indexing so their behavior cannot drift.
- Reuse incremental discovery and classification for existing indexes, while a
  fresh index or `--force` plan treats all eligible files as new work.
- Route dry runs through an early read-only CLI path. It must not create an
  index, update `settings.toml`, synchronize configured paths, save metadata, or
  mutate an existing index.
- Keep human-readable output concise and make the summary available as stable
  structured output for scripts.

### Acceptance criteria

- Fresh, incremental, forced, directory, and single-file plans report the same
  file set that the corresponding real run would process.
- New, modified, renamed, invalidated, unchanged, and deleted cases have
  deterministic fixture coverage.
- Ignore rules, overlapping roots, file limits, unsupported files, and
  traversal failures have explicit coverage.
- Tests prove that dry runs leave configuration and index contents unchanged.
- Automated validation uses deterministic fixtures and mocked or disabled
  inference transports; it never spends paid inference credits.

## Shareable generated indexes

**Status:** Proposed; format and compatibility design required

Allow a generated Codanna index to be exported, transferred, and safely loaded
in another checkout without copying a live `.codanna/index` directory by hand.

### Phase 1: Portable bundle format

- Define a versioned, transport-neutral archive with a manifest containing the
  Codanna version, emission-semantics version, storage/schema compatibility,
  semantic model metadata, source revision, indexed roots, artifact sizes, and
  SHA-256 checksums.
- Store source identities and indexed paths relative to the workspace. Reject
  or explicitly omit out-of-workspace sources rather than embedding unusable
  machine-specific paths.
- Include committed Tantivy artifacts and only the semantic base/delta
  generations referenced by the active semantic journal.
- Exclude locks, temporary and staging files, orphan generations, and derived
  resolver caches that can be rebuilt locally.
- Specify compatibility and migration rules before treating bundles as a
  stable public artifact format.

### Phase 2: Safe export and import

- Add explicit export and import commands rather than treating direct directory
  copies as supported behavior.
- Export from a consistent committed snapshot so concurrent indexing cannot
  produce a mixed bundle.
- Import into a staging directory and validate archive paths, checksums,
  versions, source identity, semantic dimensions, and index readability before
  promotion.
- Rebase portable metadata to the destination workspace and rebuild excluded
  machine-local resolver data.
- Atomically promote a validated import. Refuse to replace an existing index
  unless replacement was explicitly requested, and keep the previous index
  recoverable when replacement occurs.

### Phase 3: Distribution integrations

- Keep the bundle format independent of any storage provider.
- Add optional CI artifact, release asset, object-storage, or cache-service
  integrations only after local export/import is stable and measured.
- Document trust, retention, confidentiality, signing, and provenance rules
  before indexes containing private source-derived data are distributed.

### Acceptance criteria

- An index exported from one checkout can be imported into another checkout at
  the same source revision and returns equivalent deterministic text and graph
  query results.
- Optional semantic data loads only when its format, model, backend, dimension,
  and referenced generations are compatible.
- Corrupt, truncated, path-traversing, stale-source, and incompatible bundles
  fail before changing the active index.
- Export excludes process-local locks, temporary files, stale generations, and
  absolute source-machine paths.
- Bundle size, export/import duration, peak memory, and compatibility behavior
  are measured on small and production-shaped deterministic fixtures.

## Recommended sequence

1. Complete the shared, read-only index planner and make dry-run trustworthy.
2. Use that planner and the existing metadata gates to define source identity
   and bundle compatibility.
3. Implement and qualify local export/import.
4. Add distribution integrations only when the portable format is stable.
