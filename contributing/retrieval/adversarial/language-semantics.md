# Rust, Go, and Python resolution contracts

These notes describe the implemented evidence rules and their limits. They do
not replace the measured results in `results.json` or the additional-test
checklist. The source fixtures disable semantic search and require no model or
provider.

| Case | Required graph behavior | Evidence used |
| --- | --- | --- |
| Rust `let value: Token = Client::make()` | A call on `value` uses `Token`; an associated function's owner cannot override the annotation. | Explicit local type annotation. |
| Rust `let value = Client::make()` | A declared return type may establish the receiver; an unknown return stays unresolved. | A unique same-file struct or enum owner, matching lexical module, and a declared return type. Duplicate owner names, imported aliases, glob imports, and qualified or cross-file factories do not qualify for this inference. |
| Rust generic call syntax | `identity::<u8>()` and `worker.accept::<u8>()` preserve the original callable and receiver. | The grammar's generic wrapper is removed while retaining the physical call position. |
| Python local and field named `worker` | `self.worker: Second` does not change the type of local `worker: First`. | The complete assignment target distinguishes `self.worker` from `worker`. |
| Python multiple inheritance | Annotated receiver, `self`/`cls`, and lexical `super()` dispatch use the same C3 base order. | Ordered class identities, direct member ownership, and complete parent evidence. |
| Python incremental inheritance | Editing a consumer preserves unchanged imported ancestor identities. | Persisted `Extends` identities ordered by each base expression's source position, checked against the stored class declaration, then overlaid by current contexts. |
| Python unresolved or inconsistent bases | No unrelated global class supplies a missing bare parent; invalid C3 does not pick an arbitrary ancestor. | Scope/import identity and a C3 merge that rejects missing bases, cycles, duplicate bases, and inconsistent order. |
| Python defaults and decorators | Their calls belong to the environment evaluating the definition. | Only the function body changes the active caller; bare decorator application is retained. |
| Python annotations | An annotation such as `compute_type()` produces a `Uses` dependency rather than an eager `Calls` edge. | Annotation syntax is distinct from the body and defaults. This avoids assuming eager evaluation across Python versions and future-import modes. |
| Go generic calls | One or multiple type arguments and zero, one, or multiple value arguments retain callable identity. Actual type conversions are excluded. | The grammar's conversion/index variants plus declaration or type-argument evidence and resolved symbol kind. Indexed function containers are not treated as their elements. |
| Go variable and parameter evidence | Grouped parameter names, `var` initializers, and generic composite literals can identify receivers. A local parameter can shadow a package name. | Repeated grammar name fields, initializer types, and lexical value declarations. |
| Go structural interfaces | A type implements an ordinary interface only when required signatures match its method set. | Named type identities, ordered parameter/result types, variadic distinction, private package identity, pointer/value receivers, and embedded promotion with ambiguity and field shadowing. |
| Go external test packages | A private method in `package p_test` cannot satisfy `package p` merely because both files share a directory. | Persisted package declarations combined with module paths. |
| Go incremental interfaces | Changing a method or interface removes stale implementation edges, including those of unchanged embedded wrappers. | All derived Go `Implements` rows are replaced from the live index; generic incoming-edge rebinding does not restore those rows. |

The `Socket -> Reader` structural edge represents a declaration/interface pair.
When only the pointer method set satisfies the interface, its metadata records
`receiver = "*Socket"` and `context = "Go method set: pointer only"`. A value
implementation records the type name and `Go method set: value and pointer`.
The index does not invent a separate `*Socket` declaration.

Go method-set inference is deliberately bounded by available type evidence.
Generic interface constraints and substitutions, alias expansion, and named
non-struct underlying types are not fully modeled. Unknown parameter types or
array lengths that require constant evaluation leave the affected set
unresolved. Numeric array lengths are compared by literal value, so `16` and
`0x10` agree; unrelated package constants with the same spelling do not. Empty
ordinary interfaces include every complete concrete type handled by this
analysis, without a silent result cap. Large interface/type products can
produce correspondingly large graphs.

Python's C3 evidence describes the indexed class hierarchy. Dynamically computed
bases, runtime mutation, metaclass transformation, and cooperative `super()`
targets that depend on an unknown runtime subclass require further runtime or
type-flow evidence. The C3 helper rejects hierarchies beyond its recursion
bound. Go embedded method expansion likewise fails closed on cycles or excessive
promotion work; neither guard manufactures an alternative target.

The additional integration target is `language_semantics_regressions`; parser
guards are in `parser_semantics_regressions`, Python dispatch negatives are in
`python_dispatch_regressions`, and structural method-set units are in
`parsing::go::method_sets::tests`. The original investigation's 62 assertions
remain a separate baseline cohort.
