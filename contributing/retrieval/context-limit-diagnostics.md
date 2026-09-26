# Search-context limit diagnostics

Implements the request-validation slice of T09 in
[PR #43](https://github.com/bpstr/codanna/pull/43). This does not change retrieval,
conversation opt-in, resource budgets, or result sections.

`code_limit`, `document_limit`, and `conversation_limit` each accept an integer
from **1 through 10**, inclusive. Omitting a field keeps its default of **5**.
Zero is not a request to disable a section; strings, booleans, nulls, fractions,
negative values, overflow, and unknown keys remain invalid.

Range and type errors name the actual offending field and the accepted range.
For example, the originally reported `conversation_limit: 0` request now yields
a diagnostic containing:

```text
conversation_limit must be an integer between 1 and 10; received 0
```

The deserializer and direct `search_context` handler use one validator. Invalid
direct Rust requests still return an MCP tool error before code, document, or
conversation retrieval. JSON deserialization still rejects invalid arguments
before dispatch. No error becomes a successful empty result.

## Fixtures

`tests/context_limit_diagnostics.rs` covers the original three-limit request,
all three fields, both serde JSON entry points, lower/upper accepted boundaries,
omitted defaults, serialization roundtrips, strict unknown keys, advertised
schema type/bounds/defaults, and direct-handler budget checks. It also forbids
ANSI escapes in the diagnostics. Tests use no provider or private transcript.

```bash
cargo test --locked --test context_limit_diagnostics
```

The checkpoint has not yet been verified by a completed Rust test run. Full T09
work on structured/text result parity and unavailable-versus-empty recall
fixtures is separate; this change does not claim to complete those parts.
