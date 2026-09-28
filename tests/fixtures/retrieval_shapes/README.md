# Object-method retrieval reproduction

These independently written synthetic sources reproduce two ownership shapes:
methods in a returned adapter object, including an immediately invoked anonymous
async callback, and methods in a bound object initializer. They contain no
application implementation or private evaluation content.

The executable oracle is `tests/retrieval_object_method_regressions.rs`.

| Layer | Expected result | Test status |
| --- | --- | --- |
| Raw calls | Three `mergePage` sites in adapter.ts, owned by listColumn, loadTasks and testHelper; source coordinates point to the call expressions | Active, not run |
| Named control | testHelper resolves to page.ts:mergePage in both directions; reference.ts has no callers | Active, not run |
| Returned-object symbols | One callable endpoint each for listColumn and loadTasks, covering the method body | Active, not run |
| Bound-object symbols | One callable endpoint for listBoundColumn | Active, not run |
| Persisted graph | Exactly four distinct callers of page.ts:mergePage, forward/reverse agreement, call-site metadata, no reference.ts or shadowed local target leakage | Active, not run |
| Scope boundaries | Class ownership restored after nested object traversal; named nested functions keep their own owner | Active, not run |
| Unsupported shapes | No guessed endpoints for computed keys or function-valued properties; getters/setters have separate ranges | Active, not run |
| Persistence and updates | Duplicate method names resolve independently after reopening; an edited call removes its prior target edge | Active, not run |

All tests specify intended behavior, not an observed failure or a passing
qualification. Compilation and execution remain deferred because local resources
are constrained.
Assertions remain outside the indexed corpus. Both index fixtures explicitly
disable semantic search; parser checks instantiate only the TypeScript parser.
No provider transport, model download, credential loading or fixture recording is
needed.

After local execution is authorized and resources are available, start with:

```sh
cargo test --test retrieval_object_method_regressions
```

## Static diagnosis on main

Inspected base: `b4f8a4b80986d45531c6a45c1eb40b6a70fdd8c5`.

In `src/parsing/typescript/parser.rs`, `extract_symbols_from_node` emits function
declarations and delegates class methods to `extract_class_members`, but has no
object `method_definition` emission branch. `process_variable_declaration`
descends into recognized function bindings only; an ordinary object's initializer
is not traversed for nested symbols. These are separate omissions.

Meanwhile, `extract_calls_recursive` enters `method_definition` by name and
inherits the enclosure for anonymous arrows. Raw call ownership can therefore
exist without a corresponding method symbol. In
`src/indexing/pipeline/stages/collect.rs`, `create_unresolved_relationship` looks
up the caller by name/range. `ResolveStage::resolve_one` in
`src/indexing/pipeline/stages/resolve.rs` returns early for a missing `from_id`.
This is a source-grounded mechanism for lost persisted callers; execution is
still needed to check for additional resolution problems.

## Implementation and remaining verification

1. Ordinary variable initializers now traverse for nested declarations. Object
   methods with identifier keys reuse method signature extraction and retain
   full declaration ranges, while receiving lexical rather than class scope.
2. The existing call walker retains anonymous callback ownership. Method body
   traversal emits named nested function declarations independently. Class
   methods and recognized function bindings retain their existing traversal.
3. Tests cover duplicate method names in separate objects, computed keys,
   getters/setters, function-valued properties and class-scope restoration.
   Computed keys and function-valued properties remain unsupported as object
   callable endpoints; they are not guessed from global namesakes.
4. `EMISSION_SEMANTICS_VERSION` in `src/storage/metadata.rs` advances to v5.
   Existing version gates require rebuilding older indexes instead of silently
   retaining stale caller rows. No rebuild has been performed in this session.
5. All witnesses are active. Call-site metadata, persistence and edit scenarios
   are prepared. Run them and the normal repository checks when resources permit.

This PR changes TypeScript symbol emission and the index compatibility stamp.
It makes no measured performance claim. Scope/fidelity and runtime costs remain
unverified until the prepared tests and repository gates can run.

## Other retrieval causes to investigate separately

Source inspection also confirms several independent boundaries on this base:

- `src/indexing/pipeline/stages/collect.rs` supplies only documentation text under
  the legacy DocComment policy. Existing body V2 capture is a separate opt-in
  representation. Compare inputs with deterministic vectors or prepared
  transports before considering a corpus rebuild; do not change the default
  based on a small retrieval sample.
- `src/cli/workspace/mcp/live.rs` exits in semantic-manual mode before automatic
  catch-up/watch when semantic indexing is enabled or semantic metadata exists.
  A semantic-safe refresh proposal needs publication, lease and generation
  checks. Simply relaxing `eligible` risks inconsistent code/vector snapshots.
- `src/mcp/tools/search.rs` routes omitted score floors separately from explicit
  ones. Specify the effective floor and retrieval mode before changing defaults;
  similarity alone does not establish document support or canonical authority.
- `src/main.rs` skips project resolver initialization for languages with empty
  configuration paths. Binding several real project roots requires independent
  isolation and alias-resolution fixtures; this is not proof of a failure for
  any individual import.

Dynamic imports/lazy composition, interface-dispatch candidates, Python protocol
references, document freshness and authority/abstention ranking need their own
fixtures. More embeddings cannot create missing graph edges or repair stale
coordinates. A frozen representative corpus and isolated timing/memory
measurements are needed before claiming scaling improvements.
