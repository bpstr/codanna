<div align="center">

<h1 align="center">Codanna</h1>

[![Claude](https://img.shields.io/badge/Claude-✓%20Compatible-grey?logo=claude&logoColor=fff&labelColor=D97757)](#)
[![Google Gemini](https://img.shields.io/badge/Gemini-✓%20Compatible-grey?logo=googlegemini&logoColor=fff&labelColor=8E75B2)](#)
[![OpenAI Codex](https://img.shields.io/badge/Codex-✓%20Compatible-grey?logo=openai&logoColor=fff&labelColor=10A37F)](#)
[![Rust](https://img.shields.io/badge/Rust-CE412B?logo=rust&logoColor=white)](#)
[![Crates.io Total Downloads](https://img.shields.io/crates/d/codanna?logo=rust&labelColor=CE412B&color=grey)](#)

<p align="center">
  <a href="https://docs.codanna.sh/" target="_blank">Documentation</a>
  ·
  <a href="https://github.com/bartolli/codanna/issues">Report Bug</a>
  ·
  <a href="https://github.com/bartolli/codanna/discussions">Discussions</a>
</p>

<h2></h2>

**Local code intelligence MCP server and CLI for AI coding agents.**

*X-ray vision for your agent.*

</div>

<p align="center">
  <img src="https://raw.githubusercontent.com/bartolli/codanna/main/assets/readme/disc-tour.webp" alt="The codanna index as an interactive disc: module wedges, call webs, symbol search, commit timeline" width="100%">
</p>

Codanna is a local code intelligence and semantic code search MCP server for AI coding agents. One MCP call returns symbol context, call graph, and impact analysis, pre-correlated — the grep-and-read loop your agent runs today, collapsed into a single response it can act on.

It indexes your repository on disk and serves symbol search, semantic search, call graphs, dependency tracking, document RAG, and impact analysis to Claude Code, Cursor, Windsurf, Codex, Gemini, and any MCP-compatible client — as a persistent MCP server for session-long work, or a one-shot CLI for instant answers when LSP is too slow. Written in Rust. 15 languages. No source code leaves your machine.

## Two operational modes

Codanna's MCP toolset is reachable two ways. Same tools, same surface — pick the mode that fits the workflow.

**Persistent MCP server** — for session-long agent workflows.

```bash
codanna serve                # stdio
codanna serve --http         # HTTP
codanna serve --https        # HTTPS
```

**One-shot CLI** — for slash-commands, bash hooks, scripts, CI. No daemon required.

```bash
codanna mcp find_symbol name:"my_function"
codanna mcp analyze_impact symbol_name:"my_function"
codanna mcp semantic_search_with_context query:"recursively extract function calls"
```

`codanna mcp semantic_search_with_context` is the headline command: it returns N semantic matches, and for each match the symbol identity, signature, docstring, callees, callers, and recursive impact analysis — five MCP tools fused into one query.

Exact symbol listings return up to 100 matches by default. Use `limit:1..1000` and `offset:0..` to page through common names. Language filtering and `Type.member` qualification apply before pagination, so a member stays reachable even when more than 100 other types use its name. `lang` also applies to direct `symbol_id` lookup.

```bash
codanna mcp find_symbol name:save limit:100 offset:0 --json
codanna mcp find_symbol name:save limit:100 offset:100 --json
codanna mcp find_symbol name:Invoice.save lang:python
```

CLI JSON keeps the existing result array and adds `meta.total`, `meta.offset`, `meta.limit`, `meta.next_offset` (when another page exists), and `meta.truncated`. MCP returns the same page information under `structuredContent.pagination`, with `returned` indicating the page size. Results use source-location order; restart pagination after the index changes. An empty page beyond the end still reports the complete match count.

Contextual semantic search retains its matches when a popular symbol exceeds the impact budget. Per-function `impact.status` is `complete`, `budget_exceeded`, or `unavailable`; only a completed traversal has a `count`. MCP exposes these records under `structuredContent.impact`, and CLI JSON includes `impact` on each function result. Standalone `analyze_impact` continues to report a budget error explicitly. Lexical search treats rejected query syntax as an analyzed literal phrase, so pasted qualified names such as `std::collections::HashMap` can match signatures. Invalid search limits and backend query failures are errors, not “no results.”

Ticket retrieval supports scoped semantic candidates, observed facets, persistent documentation links, owner/impact profiles, and paged coverage of bounded candidates. See the [evidence retrieval guide](contributing/retrieval/evidence-v1/README.md) for request examples, limits, and the opt-in `symbol_body_v2` representation.


In JavaScript and TypeScript, passing a resolved function as an argument, such as `router.get('/health', handle)`, records a `References` relationship with the argument's source location. Find registrations and their dependents with `codanna mcp analyze_impact symbol_name:handle`; inspect source evidence with `codanna retrieve describe handle --json` under `relationships.referenced_by`. Describing the registering function exposes `relationships.references`. `get_calls` and `find_callers` continue to report explicit invocations, while dependency and impact queries also traverse references.

The one-shot CLI is also what makes codanna skill-friendly: an Agent Skill can wrap `codanna mcp` commands directly in Claude Code, Cursor, Windsurf, Codex, Gemini, or any harness that runs shell commands — no MCP plumbing required.

## What one call returns

The headline command, run against codanna's own source tree. One call, and the agent has the symbol, its documentation, signature, callees with exact call sites, callers, and blast radius:

```text
$ codanna mcp semantic_search_with_context query:"recursively extract function calls" limit:1

Found 1 results for query: 'recursively extract function calls'

1. extract_calls_recursive - Method at src/parsing/cpp/parser.rs:1002-1047 [symbol_id:1477]
   Similarity Score: 0.833
   Documentation:
     Recursively extract function calls with context tracking
   Signature: fn extract_calls_recursive<'a>(
        node: Node,
        code: &'a str,
        current_function: Option<&'a str>,
        calls: &mut Vec<(&'a str, &'a str, Range)>,
    )

   extract_calls_recursive calls 3 function(s):
     -> Method extract_calls_recursive at src/parsing/cpp/parser.rs:1002 [symbol_id:1477] (called at src/parsing/cpp/parser.rs:1045)
     -> Method function_name_at_def at src/parsing/cpp/parser.rs:377 [symbol_id:1451] (called at src/parsing/cpp/parser.rs:1008)
     -> Method new at src/types/mod.rs:112 [symbol_id:7388] (called at src/parsing/cpp/parser.rs:1028)

   2 function(s) call extract_calls_recursive:
     <- Method extract_calls_recursive at src/parsing/cpp/parser.rs:1045 [symbol_id:1477]
     <- Method find_calls at src/parsing/cpp/parser.rs:885 [symbol_id:1467]

   Changing extract_calls_recursive would impact 1 symbol(s) (max depth: 2):

     methods (1):
       - find_calls [symbol_id:1467]

Guidance: Found one match with full context. Review the relationships to understand how this fits into the codebase.
```

Every result carries `file:line` coordinates the agent can open directly, `symbol_id`s it can reuse to disambiguate name collisions, and a guidance hint it can chain on. Add `--json` for the structured envelope.

## Local-first

Codanna runs on your machine. The repository index lives on disk under `.codanna/`. The embedding model downloads once on first use, then runs locally. No source code, symbols, or queries are sent to a remote API by default. Remote OpenAI-compatible embeddings are opt-in: `remote_url` under `[semantic_search]` in `settings.toml`, or `CODANNA_EMBED_*` environment variables; the API key is environment-only.

Embedding inputs are validated in full, including document headings, and oversized inputs fail without truncation. Configure the provider's token budget, optional local tokenizer file, and explicit model revision as described in [Embedding input limits and revisions](contributing/embedding-inputs.md).

<h3 align="left"></h3>

## Quick Start

### Install (macOS, Linux, WSL)

```bash
curl -fsSL --proto '=https' --tlsv1.2 https://install.codanna.sh | sh
```

### Or via Homebrew

```bash
brew install codanna
```

### Or via Nix

```bash
nix run github:bartolli/codanna
```

### Windows (PowerShell)
```powershell
irm https://raw.githubusercontent.com/bartolli/codanna/main/scripts/install.ps1 | iex
```

See [Installation Guide](https://docs.codanna.sh/installation) for Cargo and other options.

### Initialize and index

```bash
codanna init
codanna index src
```

### Search code

```bash
codanna mcp semantic_search_with_context query:"where do we handle errors" limit:3
```

### Search documentation (RAG)

```bash
codanna documents add-collection docs ./docs
codanna documents index
codanna mcp search_documents query:"authentication flow"
```

Document indexing and every document query use the configured `semantic_search`
backend, model and dimensions. With `semantic_search.enabled = false`, documents
use lexical search over their text and headings without loading an embedding
model. Queries read the indexed snapshot; run `codanna documents index` to refresh
it, or use the persistent server's document watcher. Collection overrides also
apply to watcher updates, including new files and recreated directories. With an
attached document store, settings edits also reload collection roots, globs and
chunking, and reconcile additions or removals immediately. Removing a collection
or disabling documents removes its derived indexed data while preserving source
files. See [Live document collection reload](contributing/retrieval/embedding-followups/COLLECTION-RELOAD.md)
for retry behavior and restart boundaries.

`documents index --force` replaces the selected collections while preserving other
collections' source tracking. A source can belong to one collection; overlapping
collections fail explicitly. Changed content and chunking settings invalidate the
stored chunks even when modification timestamps are unchanged. Persisted vectors
record their backend/model identity; changing that identity requires a fresh
document index. Document updates publish recoverable generations and compact
obsolete vectors while existing queries retain their original snapshots. See the
[embedding and lexical retrieval results](contributing/retrieval/embedding-improvements/README.md)
for the fixture checklist, measured storage growth, migration limits and remaining
improvements.

Inspect indexing from another terminal without loading an embedding model:

```bash
codanna documents status
codanna documents status --json
codanna documents stats docs
codanna documents stats docs --json
```

Document indexing records its phase, collection, current file, completed embedding
count, elapsed time, and time since progress last advanced. A heartbeat is written
every 10 seconds, including during embedding batches and with `--no-progress`.
The latest ten run records are available through `documents status`; records live
under the configured index directory in `documents/runs/`. Failed runs retain their
error. A heartbeat older than 30 seconds is marked stale, with completion unconfirmed.
This indicates a missing heartbeat, not proof of a deadlock. A fresh heartbeat also
does not prove the embedding batch is advancing; check the progress age and count.
Completed runs describe only the collections selected for that run and whether
embeddings were enabled. Older indexes have no run record. Collection statistics
report stored metadata counts; the additive `embedding_index` diagnostics describe
the shared document index across all collections, including live/stored vectors,
chunks without embeddings, active generation and backend/input identity. These
counts describe committed evidence and do not establish freshness against current
source files. Run-progress diagnostics currently cover CLI document indexing.

## What It Does

Your AI assistant gains structured knowledge of your code:

- **"Where's this function called?"** - Instant call graph, not grep results
- **"Find authentication logic"** - Semantic search matches intent, not just keywords
- **"What breaks if I change this?"** - Full dependency analysis across files

The difference: Codanna understands code structure. It knows `parseConfig` is a function that calls `validateSchema`, not just a string match.

## Features

| Feature | Description |
|---------|-------------|
| **[Semantic Search](https://docs.codanna.sh/features/semantic-search)** | Natural language queries against code and documentation. Finds functions by what they do, not just their names. |
| **[Relationship Tracking](https://docs.codanna.sh/features/relationships)** | Call graphs, implementations, and dependencies. Trace how code connects across files. |
| **[Document Search](https://docs.codanna.sh/features/document-search)** | Index markdown and text files for RAG workflows. Query project docs alongside code. |
| **[MCP Protocol](https://docs.codanna.sh/reference/mcp-quick)** | Native integration with Claude, Gemini, Codex, and other AI assistants. |
| **[Profiles](https://docs.codanna.sh/features/collaboration)** | Package hooks, commands, and agents for different project types. |

**Performance:** parser throughput 76,000-249,000 symbols/second depending on language; warm-server lookups under 10 ms (0.3 ms exact, ~3 ms semantic). Parsing is the fast layer; embedding generation during indexing takes longer and scales with your cores. Measured numbers, machine spec, and reproduction commands: [Benchmarks](https://docs.codanna.sh/reference/benchmarks).

**Languages:** Rust, Python, JavaScript, TypeScript, Java, Kotlin, Go, PHP, C, C++, C#, Clojure, Lua, Swift, GDScript.

## Claude Code plugin

Two skills over the index, driven by the agent: it picks the view that fits the question. **x-ray** covers scoped questions — a symbol's blast radius as a call DAG, module structure as a collapsible tree or radial poster, cross-module calls as an edge-bundle page. **graph** covers codebase shape: the module disc with hubs at the centre, a commit timeline, and a heatmap. Symbol search, detail panels with highlighted signatures, call-graph navigation. The disc renderer is adapted from [vault-graph](https://github.com/luke321/vault-graph) by Lukas Proprentner (MIT).

```
/plugin marketplace add bartolli/codanna
/plugin install codanna-toolset@codanna
```

<p align="center">
  <img src="https://raw.githubusercontent.com/bartolli/codanna/main/assets/readme/xray-tree.webp" alt="The x-ray collapsible tree: module structure with symbol detail panels and call-graph navigation" width="100%">
</p>

## Integration

Add codanna to any MCP-compatible client. Project-scoped `.mcp.json` (Claude Code, Cursor, Windsurf):

```json
{
  "mcpServers": {
    "codanna": {
      "command": "codanna",
      "args": ["--config", ".codanna/settings.toml", "serve", "--watch"]
    }
  }
}
```

`--watch` keeps the index hot as files change. Transports: stdio, HTTP, HTTPS. See [Integration Guides](https://docs.codanna.sh/reference/mcp-quick) for Claude Desktop, Gemini, Codex, and network setups.

## Requirements

- ~150MB for the default AllMiniLML6V2 embedding model (downloaded on first use)
- **Build from source:** Rust 1.85+, Linux needs `pkg-config libssl-dev`
- Windows support is experimental

## Contributing

Contributions welcome. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache License 2.0 - See [LICENSE](LICENSE).

Attribution required. See [NOTICE](NOTICE).

---

Built with Rust.
