# Evidence-linked knowledge (opt-in)

The `codanna-knowledge` companion reads Codanna's existing code graph and links it to local documentation. It does not replace, migrate, lock, or write the existing Tantivy/vector indexes. No LLM calls, external URL ingestion, or repository script execution.

## Install and use

Build the companion explicitly (the normal `codanna` executable is unchanged):

```sh
cargo build --release --locked --bin codanna-knowledge
# Put target/release/codanna-knowledge beside your existing codanna binary.
codanna index src
codanna dump > /tmp/code-graph.jsonl
codanna-knowledge index --root . --repo assign-core --dump /tmp/code-graph.jsonl
codanna-knowledge links 'upload'
```

Omit `--dump` to invoke `codanna dump` in `--root`. A complete begin/result/summary stream and matching counts are required. Regenerate the dump after code indexing. A source range mismatch aborts publication; unchanged ranges cannot prove an old index is fresh, and this limitation is returned explicitly.

The companion currently requires an explicit `--bin` in Cargo commands because the package contains two binaries. Existing installed `codanna` commands and release packaging remain unchanged; the companion must be built/installed separately.

## Relationships

* Code files, code symbols, test symbols, documents, sections and rationale comments have repository-qualified stable identities, source spans and SHA-256 source hashes.
* ATX Markdown headings and duplicate heading anchors, inline relative links, exact backticked symbol mentions, and ADR/RFC references resolve conservatively.
* `// WHY:`, `# WHY:`, `-- WHY:` and corresponding `NOTE:` comments become rationale nodes. Inside a function, the smallest enclosing symbol owns the comment; comments outside symbols explain their file. No heuristic claim that a preceding comment belongs to the following function.
* Imported code edges are `resolved`, explicit textual references are `explicit`. Neither is proof of runtime correctness. Test-source classification is not executed test coverage.
* Name collisions are returned as unresolved references with candidates. Unknown inline code can be a literal and is not declared a broken symbol reference.
* Markdown images, reference-style links, escaped/percent-encoded paths, Setext headings and language-specific comment parsing are not a complete CommonMark/compiler implementation. No remote links are fetched. Fenced code blocks do not create document links.

Every edge has its extraction method and evidence. Both incoming and outgoing edges are available through `links`; use a returned id to disambiguate labels.

## Safety and lifecycle

Only regular UTF-8 files within the repository root are read. Traversal and escaping symlinks are rejected. Discovery honors `.gitignore` and `.codannaignore`; generated index, Git, target and node_modules directories are excluded. Limits: 20,000 files, 2 MiB per source file, 256 MiB total source/dump/snapshot bytes. Limits abort rather than silently truncate index coverage. Excerpts are explicitly bounded to 2,048 characters.

Snapshots are complete replacements, validated before publication, written to a unique temporary file in the target directory, synchronized and atomically renamed. Unix directory entries are synchronized after rename. Failed builds preserve the prior snapshot. Concurrent builders do not share a staging inode; the last successful complete snapshot wins. Build coordination/freshness against simultaneous source edits must be handled by the caller.

`schema_version` is checked on every load. Rebuild incompatible snapshots. Absolute repository roots are not persisted in the graph.

## Validation

`cargo test --locked --bin codanna-knowledge` covers explicit links, rationale, ambiguity, duplicate headings, code fences, deletion/replacement, identity after line shifts, malformed graphs, truncated dump streams, atomic replacement and symlink escape. The dedicated Knowledge features workflow runs on each feature-stack branch. It does not claim to replace the existing full repository test suite.
