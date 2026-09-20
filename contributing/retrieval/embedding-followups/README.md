# Document embedding follow-ups

These review packets continue the embedding and lexical retrieval work in
[PR #38](https://github.com/bpstr/codanna/pull/38). Each behavior has a focused PR,
its own fixture checklist, and explicit verification scope. Earlier reports under
`embedding-improvements/` retain their original source identity and measurements.

| Change | Review | Complete fixtures and results |
| --- | --- | --- |
| Refine document chunks to complete embedding input budgets | [PR #39](https://github.com/bpstr/codanna/pull/39) | [Design and 17-fixture checklist](document-token-splitting.md), [inventory](document-token-splitting-fixtures.json), [local validation](document-token-splitting-validation.json) |
| Reload collection policies with transactional recovery and bounded retries | [PR #41](https://github.com/bpstr/codanna/pull/41) | [Design and 18-fixture checklist](COLLECTION-RELOAD.md), [inventory](COLLECTION-RELOAD.json), [combined validation](COLLECTION-RELOAD-VALIDATION.json) |
| Measure local document relevance with reproducible captures | [PR #40](https://github.com/bpstr/codanna/pull/40) | [Design and 36-fixture checklist](https://github.com/bpstr/codanna/blob/codex/semantic-relevance-evaluation/contributing/retrieval/embedding-followups/SEMANTIC-EVALUATION.md), [verified inventory](https://github.com/bpstr/codanna/blob/codex/semantic-relevance-evaluation/contributing/retrieval/embedding-followups/semantic-evaluation/fixtures.json) |

The fixture counts represent 69 new executable tests and two revised tests across
these follow-ups. Repeated feature-set invocations, retained controls, mock helpers
and parameterized subcases do not increase those counts. The semantic evaluator's
36 Python tests are separate from the Rust suite totals.

PR #41 contains the tested combination of splitting and collection reload, and
targets PR #39's branch. PR #39 and PR #40 depend on PR #38. None of these PRs
changes the historical acceptance corpus or its required-evidence oracles.

The combined local suites pass **2,371 default / 2,373 all-feature tests** and
**29/29 lexical cases**, with **18/18 invariants**. All 35 changed Rust fixtures
pass in both suites; the [combined inventory](COMBINED-RUST-FIXTURES.json) links
each exact source hash and execution witness. The complete results are recorded in [COLLECTION-RELOAD-VALIDATION.json](COLLECTION-RELOAD-VALIDATION.json).
[Complete CI verification](CI-VERIFICATION.json) also passes **2,372 default / 2,374 all-feature tests**, with zero filtered out, including all 35 changed fixtures and the Unix socket fixture. The report archives the CI log, exact tested Git tree, source digest, and successful workflow references.
The separate [semantic status](https://github.com/bpstr/codanna/blob/codex/semantic-relevance-evaluation/contributing/retrieval/embedding-followups/semantic-evaluation/status.json)
remains **unrun**, with null relevance metrics. Passing fixed-vector and evaluator
contracts does not measure a real model's relevance.

All automated fixtures use temporary local sources, fixed tokenizers/vectors,
mocked CLI or loopback responses, or bounded local Python child processes. No model
weights, provider credentials, generated indexes or caches belong in this packet.

The reports document remaining limits: conservative bounded segmentation,
restarting a server that has no attached document store, separate code-directory
catch-up and its existing working-directory assumption, and manual label/model
qualification before drawing semantic quality conclusions.
