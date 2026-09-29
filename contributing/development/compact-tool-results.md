# Compact, addressable tool results

The six search tools (`search_symbols`, `semantic_search_docs`,
`semantic_search_with_context`, `search_documents`, `search_context`, and
`search_ticket_context`) share a compact output contract. `get_document_chunk`
uses the same budget for exact indexed document reads. Other tools, the general
`retrieve` CLI, and the separate `documents` CLI retain their existing contracts.

## Output controls

`view: "compact"` is the search default. It returns identifiers, source locations,
short evidence previews and essential availability/coverage facts. `view: "detail"`
adds longer previews and ranking diagnostics. In `semantic_search_with_context`,
only `detail` performs automatic graph expansion; compact discovery does not spend
context or traversal work on every candidate's neighbourhood. Explicit ticket
profiles and `include_related_code` still control ticket graph traversal.

`max_output_bytes` defaults to **8192**, with accepted integer values **1024–65536**.
This is a UTF-8 byte ceiling, not a tokenizer estimate. The measured object is the
entire serialized MCP `CallToolResult`, including text, `structuredContent`, JSON
escaping, and omission metadata. JSON-RPC framing, host-added wrappers, and other
tools in a parallel batch are outside this per-result contract. The direct CLI also
checks its final JSON export (including the final newline).

Search candidate collection and ranking limits are unchanged. The output layer
selects complete ordered result rows, with all primary lists considered before
optional details and round-robin admission between equally important sources.
The text rendering also places primary rows before supplemental lists. Identities
are not shortened to make rows fit; only explicitly labelled previews are lossy.

`output.partial` means that rows in the retrieved result were omitted by the
output budget. `output.lists` gives returned and omitted counts for each list.
This is not full-corpus coverage. `retrieved_chunks` / `retrieved_symbols` report
backend candidate counts where available; `returned_chunks` / `returned_symbols`
count emitted rows. Search result limits, incomplete graph traversal, and unknown
source freshness remain separate facts. A fully displayed page proves none of
those are complete or current.

A budget too small for essential metadata returns `output_budget_too_small`, not
an apparently successful empty search. An oversized first result is not skipped
in favour of a lower-ranked result. Its omission is explicit. Increase the budget
or narrow the source/target. Do not repeatedly issue the same unsuccessful query.

Previews are evidence excerpts, not full source. `preview_truncated` describes
additional preview clipping; keyword-in-context extraction can already contain
ellipsis markers. Exact text is available through the chunk reader below.

## Search within a document, then read its indexed chunk

```bash
codanna mcp search_documents \
  'query:reconnect snapshot' document:docs/realtime.md \
  max_output_bytes:4096 --json
```

The `document` selector is relative to the selected workspace, not the shell's
working directory. It is also supported by `search_context` and
`search_ticket_context`. Traversal and symlink escapes are rejected. No source is
indexed or rebuilt to satisfy a query.

Document hits retain `chunk_id`, `source_path` and the source `byte_range`.
Ranges are zero-based UTF-8 bytes with an exclusive end; they address the indexed
source revision, not a possibly edited current file. A document generation is
returned in `retrieval.document_generation` (or `documents.document_generation`
for ticket context). Handles are scoped to the selected document store/workspace.

Pass the **actual returned values**, not the placeholders below:

```bash
codanna mcp get_document_chunk --args \
  '{"chunk_id":123,"document_generation":"<returned-generation>","line_limit":40,"max_output_bytes":8192}' \
  --json
```

This reader uses a pinned read-only index snapshot, never embeddings or current
file contents. `chunk.lines` contains exact indexed text with retained newline
characters. Line numbers are **1-based relative to the indexed chunk**, not source
file line numbers. `line_offset` is zero-based; `line_limit` defaults to 80 and is
limited to 200. Use the returned `chunk.next_offset` with the same generation to
continue. The next offset accounts for the lines actually emitted, not the
requested page size. Coverage pages use the same actual-row accounting.

An index generation change requires a fresh search. An unindexed source edit does
not change the stored evidence, and the response continues to say source freshness
is unchecked. This does not implement verified live-source revision tracking.
A single huge source line is never sliced into misleading code; if it cannot fit,
it is omitted with no self-looping cursor. Raise the budget or use a source-aware
reader. A `null` next offset with `output.partial: true` is **not** end-of-document.

## Context and host integration

`search_context` now excludes conversation recall unless
`include_conversations: true`. Ticket recall already defaulted off. Historical
text remains evidence, not instructions or verified current policy.

The CLI and MCP execute the same search handler once and use its structured data.
For the seven tools above, direct `codanna mcp ... --json` now returns that object
in `data`; for example, symbol hits are in **`data.results[]`**, not the old
`data[].symbol` array. The CLI envelope advertises `meta.schema_version: "2.0.0"`;
the MCP payload advertises `schema_version: 2`. Other tools keep their existing
schema. Update machine consumers and `--fields` projections accordingly. A true
empty symbol search retains exit code 1; invalid requests retain exit code 2.

A compact text response is accompanied by the same bounded structured evidence
for MCP compatibility. Whether a host sends one or both to a model is host-specific;
no duplicate-token saving is assumed. In parallel calls the host must allocate
its aggregate response budget across calls, leaving framing headroom. A child
request larger than its parent allowance cannot be repaired by this server.

Start with one focused discovery call. Select a returned ID or document section,
read only the needed evidence, and stop expanding when it answers the question.
Retain a bounded working note with the task, chosen references, generation and next
step. Do not append whole result dumps to it. After compaction, reread needed
references explicitly; the server does not suppress evidence merely because an
earlier session received it. Use literal search or `rg` for gaps rather than
forcing repeated semantic discovery.

## Validation

`cargo test --locked --lib mcp::output` exercises whole-result budgeting, JSON
escaping, Unicode, primary ordering, oversized identities and advancing offsets.
`cargo test --locked --test compact_tool_results` exercises real temporary document
indexes, scoped discovery, search-to-read handles, stale generations, exact text,
and combined context. Embeddings use an in-process deterministic fixture with
recorded calls; no external provider is contacted.

Replay representative agent tasks before claiming token or compaction savings.
This change does not add a document-outline tool, a cross-session evidence cache,
new ranking algorithms, or verified current-source provenance. Those are distinct
changes, not implied by compact output.
