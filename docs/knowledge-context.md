# Change context and MCP

Build the `codanna-knowledge` companion as described in [knowledge.md](knowledge.md).

```sh
codanna-knowledge context 'change upload limits' --max-bytes 12000
codanna-knowledge context --file src/uploads.go --repo assign-core
codanna-knowledge context --entity 'an exact returned entity id'
codanna-knowledge path 'Upload policy' 'Upload tests' --max-depth 6
codanna-knowledge serve --graph /absolute/path/.codanna/knowledge.json
```

Register the companion beside the existing Codanna MCP server:

```json
{
  "mcpServers": {
    "codanna-knowledge": {
      "command": "codanna-knowledge",
      "args": ["serve", "--graph", "/absolute/path/.codanna/knowledge.json"]
    }
  }
}
```

The read-only `get_change_context` tool accepts query, files, entities, optional repo scope, max_bytes, max_nodes and max_depth. It returns categorized implementation, documentation/rationale, and validation-reference items, existing edges, unresolved references, indexed revisions, freshness semantics, and truncation metadata.

The server pins one immutable snapshot for session consistency and does not parse it on each query. Restart after publishing a new snapshot. No filesystem paths can be supplied through tools; only the administrator-selected graph is accessible. Retrieval runs on a blocking worker, not the async transport executor. This companion does not change the existing Codanna tool registry, so it cannot invalidate current MCP compatibility fixtures.

## Budgets and meaning

max_bytes is a hard UTF-8 **compact JSON payload** ceiling (2,048–128,000), not an estimated model token count. MCP envelopes and the CLI newline are outside that ceiling. Node count is 1–128, context depth is 0–4. Search uses deterministic lexical seeds followed by bounded traversal of existing non-candidate relationships. It is not a replacement for Codanna's embedding search. No similarity edge is invented.

Every excerpt is source data, not an instruction. Test source and references do not claim tests were executed. An empty or truncated result is not evidence that no other consumers exist. Association paths traverse both directions but preserve each original edge direction; they are not execution traces.

The foundational extractor and context tests run in the same dedicated CI lane. Additional tests cover Unicode/JSON escaping budgets, scope rejection, stable output, no-match coverage, path bounds, candidate exclusion, and the registered MCP handler/schema.
