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
| Returned-object symbols | One callable endpoint each for listColumn and loadTasks, covering the method body | Ignored pending parser fix, not run |
| Bound-object symbols | One callable endpoint for listBoundColumn | Ignored pending traversal fix, not run |
| Persisted graph | Exactly four distinct callers of page.ts:mergePage, forward/reverse agreement, no reference.ts or shadowed local target leakage | Ignored pending fixes, not run |

Ignored tests specify intended behavior, not an observed failure or a passing
qualification. Remove each ignore when its fix is implemented and verified.
Assertions remain outside the indexed corpus. Both index fixtures explicitly
disable semantic search; parser checks instantiate only the TypeScript parser.
No provider transport, model download, credential loading or fixture recording is
needed.

After local execution is authorized and resources are available, start with:

```sh
cargo test --test retrieval_object_method_regressions
# After implementing the proposed fixes, run the intended-behavior witnesses:
cargo test --test retrieval_object_method_regressions -- --ignored
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

## Proposed implementation

1. Traverse ordinary object initializers and emit callable symbols for supported
   object method definitions. Preserve declaration ranges and lexical ownership;
   reuse method signature extraction without assuming every object is a class.
2. Keep anonymous callback calls attributed to their enclosing method. Retain
   independent ownership for named nested functions. Avoid duplicate traversal
   of class methods and already handled function bindings.
3. Add follow-up cases for duplicate method names in separate objects, computed
   keys, getters/setters and function-valued properties before broadening support.
   Never resolve a missing endpoint by choosing a global namesake.
4. Review `EMISSION_SEMANTICS_VERSION` in `src/storage/metadata.rs` and existing
   index migration behavior when emission changes. A source fix alone does not
   repair existing persisted graphs.
5. Enable the ignored witnesses, verify persisted source metadata and reopen/edit
   behavior, then run the normal repository checks when resources permit.

This PR prepares reproduction evidence; it changes no parser or runtime behavior
and makes no measured performance claim.

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
