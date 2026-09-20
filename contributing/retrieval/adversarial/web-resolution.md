# JavaScript, TypeScript, and PHP resolution regressions

The original investigation reproduced incorrect TypeScript default-import targets, missing calls inside bare-parameter arrow functions, and several PHP syntax omissions. The original 15 regressions are in [the adversarial web module](../../../tests/adversarial/ts_php/regressions.rs). All 15 passed in the first implementation run. The expanded implementation checks are in [web_export_regressions.rs](../../../tests/web_export_regressions.rs); their final execution status is recorded in the shared results and checklist.

## Explicit ES module identities

JavaScript and TypeScript parsers now emit an export surface separately from their declaration symbols. Each indexed file has a persisted surface, including an empty surface when it exposes no declarations. Consequently, a default import resolves the provider's `default` slot regardless of the consumer's local variable name. An ordinary import in a barrel does not automatically become an export.

The resolver follows named aliases, default aliases, imported bindings subsequently exported locally, star re-exports, and namespace exports through relative module paths. Explicit exports take precedence over star exports. Star exports exclude `default`; two paths to the same declaration are one identity, while two distinct declarations with the same public name remain ambiguous. Namespace member calls consult these export facts before ordinary class-member resolution.

Type-only export paths retain their declaration identity for `Uses`, while effective import metadata prevents them from creating runtime `Calls`. This restriction follows re-export chains and applies to namespace members. Unknown external modules remain distinguishable from a known repository module that lacks the requested export. Missing known exports prevent fallback to an unrelated global declaration, including imports whose nonrelative path was resolved by existing tsconfig rules.

Relative lookup respects source-file precedence over a directory index and supports TypeScript source substitution for emitted `.js`, `.jsx`, `.mjs`, and `.cjs` specifiers. Export surfaces survive reopening and partial indexing. They are removed with file import metadata during cleanup; persisted re-export source imports also participate in consumer invalidation.

Export traversal has separate limits of 64 active slots and 4,096 total slot visits per lookup. A cycle can be skipped while another branch supplies a reachable definition. Depth or visit-budget exhaustion instead invalidates the entire lookup, even if an earlier branch found a target. This prevents partial traversal from reporting a falsely unique identity. These bounds can deliberately leave very large valid export graphs unresolved.

## Arrow ownership and declared field types

Bare-parameter arrows such as `const save = value => persist(value)` now attribute the call to `save`. A parameter identifier cannot become the function owner during AST fallback.

TypeScript constructor parameter properties now produce field symbols. Explicit instance field types and constructor parameter properties supply bounded receiver evidence for `this.field.method()` inside the owning class's methods. Lexical arrows retain that evidence; ordinary nested functions and classes establish their own `this` boundary. Field declarations retain their signatures for inspection.

PHP extraction now handles nullsafe calls, import aliases and grouped imports, qualified `extends` and `implements` names, constructor-promoted fields, every field in a comma-separated declaration, and named parameter/property type usages. A class's ordinary or promoted named field type can anchor `$this->field->method()` and nullsafe variants in its own methods. Qualified property type names remain qualified; an unavailable `\External\Gateway` must not capture an unrelated local `Gateway`.

## Coverage and boundaries

| Area | Positive evidence | Negative evidence |
| --- | --- | --- |
| Default imports and arrows | Exact original default definition in a complete graph; actual arrow caller | Same-named named export is not selected |
| Barrels | Three-hop alias chain, imported-then-exported binding, same-definition diamond, cycle with reachable leaf | Ordinary import is not an export; star excludes default; conflicting stars remain unresolved |
| Namespace exports | Direct namespace import and `export * as Namespace` calls reach the original function | Unexported member does not bind a same-named class method elsewhere |
| Type/value separation | Type-only barrel still emits the declared type usage | Type-only class import does not create a runtime static call |
| Persistence and aliases | Reopened partial index uses stored export facts; valid tsconfig alias import resolves | Missing export behind a tsconfig alias cannot select a public decoy |
| Field receivers | PHP named/nullable ordinary and promoted fields; TS named fields and parameter properties; TS lexical arrows | Untyped/union PHP fields, ordinary TS function boundaries, qualified PHP type decoys |
| Traversal limits | Normal same-definition diamond and reachable cyclic graph | Dense DAG and excessive depth cannot publish a partial winner |

The changes do not provide complete JavaScript/TypeScript module execution or PHP framework analysis. Literal CommonJS `require`/`module.exports`, dynamic imports, anonymous default expressions without declaration symbols, package export conditions, and nonrelative alias enhancement at every re-export hop remain limitations. Existing tsconfig enhancement applies at the consumer import boundary; it is not claimed as a complete package resolver.

Field propagation uses declared evidence. It does not infer arbitrary assignments, factories, container bindings, magic properties, structural or union field types, or inherited field declarations. TypeScript propagation currently accepts one simple named annotation. PHP trait conflict adaptations and framework/container registration semantics are separate gaps. Fully qualified PHP hierarchy and type names are now extracted; extraction alone does not establish complete cross-file namespace resolution.

Named callback references in JavaScript and TypeScript use the separate `References` relationship channel. Such a reference does not assert that the caller directly invokes the callback, or establish a dynamic framework/container target.

The new export facts and corrected extraction require the emission-semantics rebuild gate. They reuse existing Tantivy fields with a tagged document type rather than changing the Tantivy schema.

Run the focused checks with semantic search disabled by their fixtures:

```bash
cargo test --test adversarial_regressions web_parser::
cargo test --test web_export_regressions
```
