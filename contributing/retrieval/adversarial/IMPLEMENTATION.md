# Edge-case implementation and verification

This change implements the sequence from the code-intelligence investigation:
preserve declaration identity, retain source syntax and positions, make incremental
state converge to fresh state, expose query completeness, and add richer static
relationships only where the source supplies evidence. It also audits and repairs
the document indexing and embedding path.

The investigation baseline is commit
[`1968a6fc`](https://github.com/bpstr/codanna/commit/1968a6fc14c4d5c080ac91b5e74799a4b0aa0cc2).
The implementation also incorporates the subsequent upstream semantic MCP query
initialization fix. This is a correctness suite built from difficult cases; its
pass rate is not a measurement of accuracy on arbitrary repositories.

## Changes and their observable behavior

| Area | Example trigger | Resulting behavior |
| --- | --- | --- |
| Declaration identity | Remove `Alpha.make` while `Beta.make` still exists; import a default export under another name. | Callers retain the original declaration identity or lose the unresolved edge. They do not move to an unrelated same-name declaration. |
| Source evidence | Two physical calls on one line; a source line longer than 65,535 bytes. | Distinct expressions keep distinct positions, duplicate extraction channels merge by expression span, and byte columns survive persistence as `u32`. |
| Rust | A factory owner differs from its return type; a callee has generic arguments. | Explicit type annotations and supported declared factory return evidence identify the receiver; generic call syntax retains its actual callee. |
| Python | Diamond inheritance, `super`, imported bases, field annotations and deferred annotations. | Proven class identities use C3 ordering; `self.field` stays distinct from a local variable; annotation dependencies do not imply immediate runtime calls. |
| Go | Grouped parameters, generic calls, embedded methods, interface edits and pointer receivers. | Calls retain lexical/package evidence. Structural interface edges use supported method sets and signatures, preserve pointer-only provenance, and are rebuilt when declarations change. |
| TypeScript / JavaScript | Barrel chains, renamed/default exports, namespace imports and type-only exports. | Persisted export slots follow actual module identity and reject ambiguous, missing or type-only runtime targets. Simple declared property receiver types support ordinary methods and lexical arrows. |
| PHP | Nullsafe calls, grouped/aliased imports and promoted constructor properties. | The parser retains the relevant syntax and declared receiver evidence, including qualified-name safeguards. |
| Callback registration | `router.get('/health', handle)` inside a named owner. | A source-positioned `References` edge exposes the registration in dependency and impact queries. It does not become an immediate `Calls` edge. |
| Incremental lifecycle | Add, delete, restore or retarget a module while its consumer is unchanged; stop a run at a file limit. | Reverse import dependencies are reparsed, obsolete records are removed, and deferred resolution work survives reopening. A bounded run preserves unvisited inventory. |
| Search and graph queries | More than 100 identical names; punctuation-heavy code; oversized optional context. | Internal exact lookup remains complete, public pages expose totals, literal search uses matching analyzers, and context failure preserves independently valid semantic matches with explicit status. |
| Documents | Force reindex, edit with a preserved timestamp, change chunking or enable embeddings later. | Ownership and content fingerprints drive replacement and backfill. Query surfaces use the configured backend and an explicit indexing lifecycle. |

See [index-lifecycle.md](index-lifecycle.md) for durable pending work and watcher
semantics, and [language-semantics.md](language-semantics.md) for supported type
evidence. The [document audit](document-embedding-review.md) describes chunking,
embedding input, cache identity, ranking, failure recovery, provider parity, and
remaining improvements.

## Fixture inventory and measured results

- [CHECKLIST.md](CHECKLIST.md) lists all **62 original tests**, their source and
  assertions, the original **10 passed / 52 failed / 0 ignored** result, and the
  recorded implementation outcome. Twenty-seven native source fixtures are included.
- [ADDITIONAL-CHECKLIST.md](ADDITIONAL-CHECKLIST.md) lists added implementation,
  boundary, mutation and document fixtures separately. The 16 document baseline
  probes measured **1 passed / 15 failed / 0 ignored** against the same review
  revision. Other added tests have no pre-change result unless explicitly recorded.
- [cases.json](cases.json) and [additional-fixtures.json](additional-fixtures.json)
  are machine-readable inventories. Their result files record individual outcomes,
  tested source hashes and the revision associated with captured logs.
- The separate [retrieval acceptance workspace](../README.md) retains its 32
  executable case definitions and 10 manual scenarios. Passing parser or mocked
  vector tests does not establish retrieval relevance or satisfy those scenarios.

The final fixture result is **163 passing entries**: all **62 original cases**,
**99 newly added tests**, and **two revised existing Python tests**. No
catalogued fixture is ignored. The document baseline probes improved from
**1/16 passing to 16/16 passing**; they are a subset of the implementation cases.
The revised Python entries preserve their former names and explain their changes.
Other legacy unit fixtures now supply actual class, parent or factory declarations
instead of relying on unproven identities. Their positive target assertions remain.

| Validation | Measured result |
| --- | --- |
| `quick-check.sh` | Passed: formatting and strict Clippy for all targets/features. |
| `full-test.sh`, unchanged | Formatting, strict Clippy and no-default-features compilation passed. Library tests reached 1,337 passed / 1 environment setup failure / 27 ignored. |
| Complete default-feature suite with the one exclusion below | 2,239 passed / 0 failed / 62 ignored / 1 filtered. |
| Complete all-feature suite with the same exclusion | 2,241 passed / 0 failed / 62 ignored / 1 filtered. |
| Warning-strict documentation build, all features | Passed. |
| CLI help, fresh scratch indexing and MCP smoke test | All five commands passed. |
| Original and implementation inventory validators | Passed, with complete measured outcomes. |
| Retrieval evaluator tests / plugin boundary tests | 25/25 and 7/7 passed. |
| Actual lexical retrieval acceptance | 23/29 passed; all invariants passed, six targets unmet. |

The single excluded test is
`indexing::pipeline::stages::read::tests::hardening_read_rejects_non_regular_entries_without_blocking`.
This environment denies Unix-domain socket creation with `EPERM`, before the test
can construct its fixture or exercise source reading. Independent Python and
standard-library-only Rust probes confirmed the restriction. The repository test
is unchanged and remains enabled in normal CI. The unmodified full script was
run and its failure retained; the later default/all-feature runs use exactly this
one explicit `--skip`. These local results do not claim an unqualified full-CI pass.
The 62 ignored tests/examples are the repository's existing exclusions, including
model-dependent and explicitly opt-in tests; none was newly ignored to get green.

[validation.json](validation.json) records commands, result counts, runtime
conditions and log checksums. The captured runs used a dirty worktree based on
`bf25b185717fdaac2e36ea55e0edee24a056dbee`; the result artifacts also retain the
production/test source hash, so that snapshot is distinguishable from the base
commit. No further production/test edits followed those recorded runs.

The [lexical acceptance results](LEXICAL-RESULTS.md) preserve the six unmet targets:
leading-comment ownership, reference-style Markdown, escaped link filenames,
symbol-free configuration evidence, intent-only graph context and document
diversity. Document Hit@5 and MRR@5 were both 1.0, but required-evidence recall was
0.875 against a 0.90 floor because one query missed its second required source.
The corpus and its oracles were not changed to improve those scores. Semantic
model quality and the ten broader manual qualification scenarios remain unmeasured.

## Compatibility and limits

Index emission semantics advance to **version 4**. Existing indexes need a full
rebuild because export facts, identity rules and relationship emission changed.
The index command's existing compatibility check supplies that rebuild. Numeric
column fields remain JSON numbers, but the Rust source-column API changes from
`u16` to `u32`; downstream Rust code with explicit column types must adapt.
Additional public context fields can also require downstream Rust struct-literal
or destructuring updates.

New reference fields and search pagination metadata are additive. Call-only APIs
retain their meaning. Document queries read the indexed snapshot; callers should
run document indexing explicitly when they need freshness. A changed embedding
identity requires a fresh compatible index instead of mixing vectors from two
embedding spaces.

This implementation does not infer arbitrary runtime registration, reflection,
dependency injection containers, monkey-patching, dynamic imports or generated
declarations. Unsupported type and module evidence remains unresolved. Go generic
constraints and method-set substitution, unresolved constant array lengths, and
some aliases or non-struct named types still need compiler-grade evidence. Static
Python inheritance requires resolvable class bases. Rust cross-file factory return
inference remains limited. See the fixture checklists for proposed scenarios that
remain open.

Document publication validates embedding batches and protects the prior snapshot
from ordinary generation failures, but vector, text-index and state publication
are not one crash-atomic transaction. An interrupted process between publication
steps still needs a generation manifest or journal with fault-injection tests.
Local mock transports verify protocol and identity behavior; no semantic model
quality, multilingual relevance or production latency claim follows from them.
