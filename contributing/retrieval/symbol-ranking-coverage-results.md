# Symbol ranking coverage verification

Runtime implementation selected by the T05/T06 evidence in PR #51.

The current policy reranks only simple multi-concept discovery queries. It uses
16x requested candidates, a **128-candidate floor** for small discovery requests,
and a **200-candidate hard cap**. Explicit Tantivy syntax and single-token
identifier queries retain the pre-existing path.

The 128 floor was selected because the broader evaluation's only 80-candidate
miss was a valid owner at lexical rank 98. No score-weight or schema change was
needed.

## Required current-head checks

The implementation CI must run:

- the crowded seven-query discovery regressions;
- the 21-query tuning/holdout runtime corpus;
- the existing document retrieval-ranking regressions;
- the adversarial search/graph suite;
- formatting and strict all-target/all-feature Clippy.

The repository Quick Check, Hardening, security regressions and Full Test Suite
remain separate gates. Passing results are recorded in the PR body only after
the exact current head executes; previous 64/80-candidate runs are not reused as
proof of the 128-candidate policy.
