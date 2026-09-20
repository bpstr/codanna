# Measured lexical retrieval acceptance

The existing acceptance workspace was executed using binaries built from this
implementation, with embeddings disabled. The unmodified oracle produced
**23 passed and 6 failed out of 29 selected cases**. Every invariant passed;
six improvement targets remain unmet. This is an actual indexing and retrieval
run, separate from the parser and fixed-vector regression suites.

The complete runner output is [lexical-acceptance-results.json](lexical-acceptance-results.json).
[lexical-evaluation-context.json](lexical-evaluation-context.json) ties it to the
tested production/test source hash and records its report checksum. The report's
`inspected_main` field identifies the original acceptance specification's audit
revision; it is not the revision of the evaluated implementation. Evaluated binary
hashes and the actual `codanna_version` are recorded separately in that report.

| Measurement | Result | Acceptance floor |
| --- | ---: | ---: |
| Invariant pass rate | 1.00 | 1.00 |
| Hit@5 on the four positive document queries | 1.00 | 0.90 |
| MRR@5 on the four positive document queries | 1.00 | 0.75 |
| Mean required-evidence recall@5 | 0.875 | 0.90 |
| Forbidden evidence | 0 | 0 |

A relevant first result is insufficient when the question requires evidence from
multiple documents. D06 finds one of its two required sources: five returned
chunks come from only two policy files, and the required operational guide is
missing. Its evidence recall is 0.5. That failure reduces the mean across four
positive queries to 0.875, below the specified floor. There is no semantic-model
quality score in this run.

| Unmet case | Observed gap | Next improvement |
| --- | --- | --- |
| G06 | A declaration-leading rationale comment does not attach to the declaration. | Associate comments using adjacency and syntax boundaries while preserving explicit file-level comments. |
| G14 | Reference-style local Markdown links are not resolved. | Parse reference definitions and uses with a Markdown parser; retain exact link evidence. |
| G15 | A percent-escaped local filename is not resolved. | Decode the local link path before canonical resolution, then apply the existing root-boundary checks. |
| G17 | Referenced symbol-free configuration is absent from knowledge evidence. | Ingest explicitly linked supported configuration independently of code symbol emission. |
| C03 | Intent-only graph context does not recover delivery semantics. | Improve initial candidate selection and evaluate ranked expansion without injecting oracle symbol seeds. |
| D06 | Several chunks from the same files consume the result budget. | Evaluate source/section diversity and lexical/semantic rank fusion before choosing a default policy. |

The other document cases pass: the inclusive boundary, acknowledgement-loss
policy, a late fact with the actual containing chunk, absence of arbitrary results
for a missing term, and unknown-collection isolation. Native code and link controls
also pass, including direct and aliased calls, Unicode and duplicate anchors,
ignored-source exclusions, ambiguous-name handling, and independent peer graphs.

The corpus remains unchanged. No failing assertion was relaxed. The ten manual
acceptance scenarios remain unqualified; related unit tests do not automatically
complete the broader protocol, mutation, performance or multilingual scenarios.
No model or paid provider was used.

Reproduce from the repository after building its binaries:

```bash
cargo build --locked --bin codanna --bin codanna-knowledge
python3 contributing/retrieval/run.py --profile lexical \
  --codanna "$PWD/target/debug/codanna" \
  --knowledge "$PWD/target/debug/codanna-knowledge"
```

For a custom Cargo target directory, supply the actual absolute binary paths.
When using a dynamically linked ONNX runtime, install its shared library in the
system loader's search path. The benchmark intentionally uses a clean environment;
its credential and embedding-override allowlist was not expanded for this run.
