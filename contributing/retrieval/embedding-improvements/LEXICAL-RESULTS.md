# Lexical retrieval and ranking evaluation

The rebuilt baseline passed **23/29** lexical-profile acceptance cases. The final
frozen binaries passed **29/29**, recovering all six failed targets while
retaining all **18/18 invariants**. Both runs indexed the original corpus with
embeddings disabled; neither run used a model or provider.

The complete runs are recorded in
[lexical-baseline.json](lexical-baseline.json),
[lexical-final.json](lexical-final.json), and
[evaluation-context.json](evaluation-context.json), which binds the evaluated
source and binary hashes. The final report SHA-256 is
`bba5eb6c160cf9de60e0fccd2879d09631eae7996c5f6ee6ae640f312dc9dbd0`.
The evaluated source-tree SHA-256 is
`a4dff01b27eb17c5f2db9f23b4375994732aa48e791e3bcbf5af4ec68a93498b`.

| Measurement | Baseline | Final frozen run | Required |
| --- | ---: | ---: | ---: |
| Selected cases passed | 23/29 | 29/29 | 29/29 |
| Invariants passed | 18/18 | 18/18 | 18/18 |
| Hit@5, four positive document queries | 1.00 | 1.00 | >= 0.90 |
| MRR@5, four positive document queries | 1.00 | 1.00 | >= 0.75 |
| Mean required-evidence recall@5 | 0.875 | 1.00 | >= 0.90 |
| Forbidden evidence under the existing oracle | 0 | 0 | 0 |

Hit@5 records whether a query retrieves any required evidence in its first five
results. Reciprocal rank uses the first relevant result. Required-evidence recall
counts every required item, then averages across the four positive queries.
D06 requires two sources: finding its policy first already satisfied Hit@5 and
reciprocal rank, but omitting the operational guide reduced that query's evidence
recall to 0.5. Recovering the guide raises the overall mean from 0.875 to 1.00.
Empty-result and unknown-collection controls are excluded from those averages.

| Fixed case | Observed failure | Implemented behavior |
| --- | --- | --- |
| G06 | Declaration-leading rationale attached to the file. | A contiguous comment block at the next unique declaration's indentation attaches to that declaration. Gaps, intervening statements and explicit file comments retain their enclosing/file owner; synthetic module symbols do not absorb those comments. |
| G14 | Reference-style Markdown links remained unresolved. | CommonMark parsing resolves full, collapsed and shortcut references; separate edges retain the link-use span and active definition span. Fenced examples remain excluded. |
| G15 | Percent-escaped local filenames did not resolve. | Strict UTF-8 percent decoding occurs once before lexical and canonical containment checks, including fragments. |
| G17 | Explicitly referenced symbol-free configuration was absent. | Supported configuration files reached through admitted explicit links are loaded with real excerpts, hashes and provenance, without inventing symbols. |
| C03 | Intent-only seeds missed the implementation at depth zero. | Whole-token matching plus conservative long-token trigram similarity recovers related spellings. Exact query coverage precedes approximate evidence; node IDs break ties. No synonym list, oracle symbol seed or extra graph hop is introduced. |
| D06 | Repeated policy chunks filled the budget; the guide ranked eleventh. | Wider candidates, complete-query lexical coverage and source/section diversity recover both required documents. Stemming is a secondary ranking signal over literal-hit candidates. |

D01, D02 and D03 retain their required evidence at rank 1. D04 remains empty for
an absent term; D05 remains empty for an unknown collection. D06 changes from
required ranks **[1, missing]** to **[1, 5]**, with the following returned sources:

| Rank | Final D06 source |
| ---: | --- |
| 1 | `docs/ADR-004.md` |
| 2 | `docs/ADR-004.md` |
| 3 | `docs/reference-style.md` |
| 4 | `docs/archive/prototype.md` |
| 5 | `docs/guides/attachments.md` |

The historical prototype still appears. This evaluation establishes the specified
evidence coverage and diversity, not automatic policy-authority classification.

The two reports contain identical fixture and oracle hashes, also recomputed
against the working tree. The corpus, peer corpus, `cases.json`, runner and runner
tests have no diff from baseline revision `dd8e247`:

| Input | Identical SHA-256 in both runs |
| --- | --- |
| Primary corpus | `3dcca038ed61c665c264b6b4d03ce40df10dc6f85bd0ea0c901dc5bcc7e2b66c` |
| Peer corpus | `876b8e24bf891d4abac188c2bef575cbb1981734b1fe4f865025eeacdc4f4253` |
| Acceptance cases | `a5ecff723a277da35c185d110be3566e54f98d416638681510af29430c1b6a3e` |

No assertion was relaxed, and benchmark answers remain outside the indexed roots.
Both binaries can report the same Git revision while local changes differ; use
the binary and source hashes in the evaluation context, not the version string,
to identify the evaluated implementation. The reports' `inspected_main` value
identifies the acceptance specification's earlier audit revision.

Reproduce from a checkout with the Rust toolchain and runtime dependencies
installed. Each acceptance invocation creates a fresh disposable workspace:

```bash
cargo build --locked --bin codanna --bin codanna-knowledge
python3 contributing/retrieval/run.py --check
python3 -m unittest discover -s contributing/retrieval -p 'test_run.py'
python3 contributing/retrieval/run.py --profile lexical \
  --codanna "$PWD/target/debug/codanna" \
  --knowledge "$PWD/target/debug/codanna-knowledge"

git diff --exit-code dd8e247 -- \
  contributing/retrieval/workspace \
  contributing/retrieval/peer-workspace \
  contributing/retrieval/cases.json \
  contributing/retrieval/run.py \
  contributing/retrieval/test_run.py
```

Supply the actual binary paths for a custom Cargo target directory. A dynamically
linked ONNX runtime must be available to the system loader; the harness uses a
clean environment and does not inherit arbitrary embedding overrides or provider
credentials. See [FIXTURES.md](FIXTURES.md) for the separate regression coverage.

The ranking policy has explicit practical limits:

- Lexical search retains at most `6 * limit` candidates: `4 * limit` initial
  BM25 hits plus bounded probes excluding already represented sources. Every
  candidate still matches an analyzed literal query term and all requested
  source/collection filters. English stemming changes rank, not candidate
  eligibility. A relevant source outside the bounded pool may still be missed.
- Both search modes prefer distinct sections and cap each source at
  `max(1, floor(limit / 2))` during the first passes. Sparse candidate sets relax
  these preferences to preserve useful results. The final ordering can differ
  from raw BM25 or cosine order; returned score values remain unchanged.
- Semantic diversity uses at most `4 * limit` highest-cosine candidates. An
  additional candidate must score at least 90% of the original kth positive
  cosine; a nonpositive cutoff is retained unchanged. This **10% cosine margin**
  is a ranking heuristic, not a relevance probability or quality guarantee.
  Fixed vectors verify non-tied complementary sources, weak/zero exclusions,
  filters and sparse-source fallback. **No real semantic-model quality was
  evaluated**, and no lexical/semantic score fusion was introduced.
- Context approximate matching requires tokens of 6–64 characters, at least
  three shared trigrams and Dice overlap >= 0.70. It recognizes surface forms,
  not arbitrary paraphrases. Tokenization scans stored graph excerpts per query.
  C03 subprocess time rose from **8.18 ms to 16.89 ms** in these two runs; D06
  changed from **35.16 ms to 33.78 ms**. These are individual observations, not
  repeated latency benchmarks; larger-graph performance remains unqualified.
- Passing this profile does not complete the ten manual acceptance scenarios.
  The reports retain `full_qualification: false`; semantic-model quality,
  multilingual retrieval and broader protocol/performance qualification require
  their own evidence.
