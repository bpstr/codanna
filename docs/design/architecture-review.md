# Architecture review (2026-10-01)

**Status:** Proposal and planning record. Findings describe the tree at
`3571b3efc45f9ef275bfe57f06b87c18b323bc48` (`1.0.0-rc5`). They are not
implementation, launch qualification, or a restatement of the historical
`review.md` snapshot from 2026-09-12.

This note records structural issues that should be resolved before more MCP
tools, languages, or workspace product surface land on the same types.

## Current runtime shape

```text
CLI / MCP transports (stdio, HTTP, HTTPS, workspace router)
        |
        v
IndexFacade -- Pipeline (discover->read->parse->collect->index->resolve->embed)
        |
        +-- LanguageRegistry + Parser/Behavior + ProjectResolver
        +-- DocumentIndex (Tantivy symbols/graph)
        +-- Semantic journal + vector store + embed runtime
        +-- DocumentStore (markdown/RAG)
        +-- Watcher + CodeWriteLease
```

That split is already visible in the modules. Pipeline stages live under
`src/indexing/pipeline/stages/`. Languages follow parser + behavior +
definition + resolution. MCP tools are split across `symbols`, `search`,
`context`, `recall`, and ticket handlers. Persistence has
`EMISSION_SEMANTICS_VERSION` and an advisory write lease outside the index
directory.

The product has outgrown the crate shape. `src/` is about 75k lines in one
package. Four files carry a large fraction of the runtime:

| File | Lines | Role |
| --- | ---: | --- |
| `src/indexing/facade.rs` | 5369 | Index, query, graph, semantic diagnostics |
| `src/documents/store.rs` | 3671 | Document RAG store |
| `src/watcher/unified.rs` | 2400 | Watch, reload, catch-up |
| `src/semantic/simple.rs` | 1927 | Semantic search |

`IndexFacade` is used from CLI, MCP, watcher, documents, retrieve, rebuild
planning, and HTTP paths. It is the application kernel.

## What to keep

- Language isolation. Adding a language should remain a registry registration,
  not another factory match arm or `main.rs` change.
- Staged indexing. Discover/read/parse/collect/write/resolve/embed is the right
  model for parallelism and for the shared dry-run planner in `ROADMAP.md`.
- Artifact contracts. Emission-semantics versioning, semantic generations, and
  `CodeWriteLease` should stay the coordination model for CLI + MCP + watcher.
- Workspace invariants in `docs/design/multi-workspace.md`. One resolved scope
  per request, no process-global cwd mutation, no silent fallback across
  graphs, bootstrap without paid embeddings.
- Fork discipline in `UPSTREAM.md` and `tests/upstream_regressions.rs`.
  Reconstruct upstream-capable fixes from extracted crates rather than from the
  whole product history.

## Findings

### 1. One crate, many products

The package is a language frontend, Tantivy graph store, embedding engine,
document RAG system, two MCP servers, a workspace multiplexer, a
knowledge/recall product, and a CLI. Every tree-sitter grammar compiles into
the same binary.

**Change:** extract a Cargo workspace without changing behavior:

- `codanna-core` — `Symbol`, `FileId`, settings schema, errors
- `codanna-parse` — registry, languages, project resolvers
- `codanna-index` — pipeline, Tantivy, engine types
- `codanna-semantic` — embeddings, journal, vectors
- `codanna-documents` — RAG store
- `codanna-mcp` — tool contract and transports
- `codanna-workspace` — scope resolver, bootstrap, leases
- `codanna` — CLI and binary only

Start from existing modules. Feature-flag languages in `codanna-parse` so a
default build can ship a lean core.

### 2. `IndexFacade` is a god object

`facade.rs` coordinates storage and indexing and also owns symbol search,
call-graph and impact queries, semantic enablement, coverage diagnostics,
directory sync, and compatibility shims that swallow errors
(`get_symbols_by_file` returns `unwrap_or_default()`). Hidden storage failures
look like empty context to agents.

**Change:** split into three types:

```text
IndexWriter    // pipeline, sync, rebuild, lease-holding
IndexReader    // symbols, graph, search — fallible, no default-empty
SemanticIndex  // vectors + generation alignment
```

MCP and CLI query paths depend on `IndexReader`. Watcher and index commands
depend on `IndexWriter`. Query code must not take `&mut IndexFacade`.
`IndexFacade` may remain a temporary owner of the three pieces until the
80-method impl is deleted.

### 3. Two query stacks and two MCP servers

Query paths today:

- `src/retrieve.rs` — CLI symbol, callers, calls, implementations
- `src/retrieval/` — fused evidence
- `src/documents/` ranking
- `src/knowledge/` plus MCP ticket and recall tools

MCP frontends today:

- `CodeIntelligenceServer` in `src/mcp/server.rs`
- `WorkspaceServer` in `src/cli/workspace/mcp.rs`

The product claim is one tool surface. The code does not structurally
guarantee it.

**Change:**

1. Make `retrieval` the only query engine. `retrieve.rs` becomes a thin CLI
   adapter.
2. Extract an `McpToolKernel` that both servers call. Transports and workspace
   resolution stay outside. Tool JSON schema lives in one module.
3. Resolve a `WorkspaceHandle { reader, writer, docs, recall }` once per
   request and pass it into the kernel. Do not look up a process-global facade.

### 4. Storage naming and embedding layout

`DocumentIndex` is the code symbol and graph index. `DocumentStore` is
markdown/RAG. `SimpleSemanticSearch` is nearly 2k lines. Embedding policy is
split across crate root (`embedding_cache.rs`, `embedding_input.rs`,
`embedding_runtime.rs`), `vector/`, and `semantic/`.

**Change:** rename `DocumentIndex` to `CodeIndex` or `SymbolIndex` in a planned
compatibility window. Keep `DocumentStore` for RAG. Rename
`SimpleSemanticSearch` to `SemanticIndex`. Move the crate-root embedding
modules under `semantic/`.

### 5. Global process state fights multi-workspace

`get_registry()` is a `'static Mutex<LanguageRegistry>`. Provider setup is
assembled in `main.rs`. `Settings` is a 1,200-line blob passed as
`Arc<Settings>` everywhere. Workspace design forbids mutating process-global
cwd to route concurrent requests; a global registry and a process-wide settings
object are the same class of problem.

**Change:** construct `LanguageRegistry` once and pass an immutable
`Arc<LanguageRegistry>`. Split `Settings` into `RuntimeConfig` (paths, leases,
MCP bind) and `IndexConfig` (languages, ignore, embeddings). Keep provider
construction next to the language registry. Shrink `main.rs` to clap dispatch
and `cli::commands`.

### 6. Error model is pre-modular

`src/error.rs` still exports `IndexError`, `ParseError`, `StorageError`, and
`McpError`, plus legacy storage aliases while `storage/error.rs` has the live
type. `lib.rs` re-exports both.

**Change:** one error type per crate, `#[from]` across boundaries, no silent
`ok()` or `unwrap_or_default()` on index reads at the library edge. CLI and MCP
map errors to exit codes or MCP error data. Shrink the public library surface.

### 7. Watcher, embeddings, and HTTP are file-shaped coordinators

`watcher/unified.rs` mixes code events, document batching, config reload, and
collection reload. HTTP and HTTPS are separate servers plus `network.rs`.
Embedding policy (what text is embedded, when, which generation is live) is
spread across pipeline stages, `semantic/simple.rs`, `vector/storage.rs`, and
facade diagnostics.

**Change:** make generation publication one state machine:

```text
plan -> embed -> stage vectors -> commit journal -> publish reader generation
```

Indexer, watcher, and MCP reindex all call that machine. Dry-run calls the plan
stage only. Unify HTTP/HTTPS into one transport module with a TLS option. Auth
and bind policy belong there, not in copied servers.

### 8. Tests are becoming a second codebase

`tests/` has a large integration and regression surface, which is appropriate
for an indexer. It also means production modules are not small enough to test
in isolation.

**Change:** after crate extraction, keep unit invariants next to the crate and
reserve `tests/` for CLI, MCP, workspace, and the upstream contract. Add one
architectural test that both MCP servers expose an identical tool catalog and
JSON schema.

## Priority

### P0 — before more product surface

1. Split `IndexFacade` into reader, writer, and semantic index. Make reader
   methods fallible.
2. One MCP tool kernel used by both servers.
3. Collapse `retrieve` and `retrieval` into one query engine.

### P1 — make the crate evolvable

4. Cargo workspace extraction, starting with parse and semantic.
5. Rename `DocumentIndex` and move embedding modules under `semantic/`.
6. Replace the global language registry with an owned immutable registry.
7. Split `Settings` and shrink `main.rs`.

### P2 — harden the artifact model

8. Single vector-generation publish path for index, watch, and reindex.
9. Language feature flags or optional grammar crates.
10. Unified HTTP/HTTPS transport and bind/auth policy.
11. Formalize dry-run as the shared `IndexPlan` already proposed in
    `ROADMAP.md`. Do not implement dry-run as a second walker.

## What not to do

- Do not add another language, retrieval mode, or MCP tool until the facade is
  split. Each new tool currently accretes into the same type.
- Do not treat this fork as a thin upstream patch stack. Reconstruct generic
  parser and index fixes from extracted crates against current upstream `main`.
- Do not treat `review.md` as current behavior. Re-verify any historical P0
  against `write_lease.rs` and the current vector publish path before acting.
