# Local retrieval acceptance workspace

A small, deliberately awkward application that **Codanna itself indexes**. This is
an acceptance specification and local evaluation harness, not a claim that current
retrieval meets the proposed minimums. Production retrieval code is unchanged.

The corpus includes Python, TypeScript and PHP, decisions, runbooks, ambiguous
names, aliases, misleading old policies, unsupported link forms, ignored files,
and an independent peer workspace with conflicting facts. Names and content are
synthetic. The miniature application has no external dependencies or services.

## Run locally

Python 3.11+ is required for the harness. Build the two binaries from this checkout;
using an unrelated globally installed release would evaluate that release instead.

```bash
cargo build --locked --release --bin codanna --bin codanna-knowledge
python3 contributing/retrieval/run.py --check
python3 -m unittest discover -s contributing/retrieval -p 'test_run.py'

python3 contributing/retrieval/run.py --profile structural \
  --codanna "$PWD/target/release/codanna" \
  --knowledge "$PWD/target/release/codanna-knowledge"
```

Each run creates and retains a new temporary directory outside the checkout.
Alternatively, supply `--out /absolute/path/to/a-new-directory`. Existing output
directories are never overwritten. Its `report.json` identifies both binaries,
versions, fixture hashes, profile, commands, durations, results, errors and pending
manual scenarios. `raw/` retains stdout and stderr for every command. Generated
indexes, dumps, configuration and models remain outside Git.

| Profile | Cases | What is measured |
| --- | ---: | --- |
| `structural` | 23 | Real parser/dump/knowledge graph, evidence spans, links, bounded context, independent snapshots. Embeddings disabled. |
| `lexical` | 29 | Structural cases plus document lookup with embeddings disabled. Expected to expose the missing lexical fallback. |
| `semantic` | 31 | Structural cases plus local document/code embeddings. Explicit opt-in; first use may download the local model. |

Run `lexical` and `semantic` by replacing the profile in the command above. They
use separate clean indexes. The absent-literal case D04 is lexical-only: nearest
neighbor search returning a related result is not inherently a defect. In total,
there are **32 distinct executable cases and 10 manual scenarios**.

The semantic profile explicitly configures `AllMiniLML6V2`; it does not test arbitrary
embedding providers. It uses a fresh private HOME/cache and an environment
allowlist, omitting inherited provider credentials, Codanna embedding overrides
and `CI_` configuration overrides. No paid inference is authorized or needed.
Downloads and local inference are distinct: this profile is not guaranteed to work
without network access on its first run. Peer indexing remains embedding-free.

Exit codes: `0` means every selected case and measured numeric minimum passed;
`1` means acceptance failures; `2` means setup/evaluation could not complete.
A missing model, invalid JSON, failed process or partial/error response is never
converted to an empty successful result. An unrun profile has no quality score.
Manual qualification remains pending even after a profile passes.

## Corpus and oracle separation

Only `workspace/` and `peer-workspace/` are copied into the disposable indexing
roots. **Do not index this entire benchmark directory**: `cases.json`, the harness,
requirements and test answers intentionally remain outside the searchable corpus.
The harness writes explicit scratch configuration and passes an actual `codanna
dump` to `codanna-knowledge index`; it does not manufacture symbol or call graphs.

`cases.json` uses source paths, names, heading locations and required evidence,
not frozen numeric symbol IDs or hashes from a previous generation. IDs used by
context queries are resolved from the newly indexed graph. Explicit-seed context
cases test graph traversal; intent-only cases do not receive those hints.

Scoring validates the returned source range, not a path appearing somewhere in
formatted output. A correct filename with the wrong document chunk fails its
required-fact check. Source hashes, UTF-8 byte boundaries, line ranges and graph
endpoints are checked. Duplicate chunks still consume the top-five budget;
retrieval is not silently reranked or deduplicated by the evaluator.

## Minimums and known red cases

The initial quality contract is intentionally stricter than the current code.
`invariant` means a hard correctness requirement, **not a pre-recorded pass**.
`target` means a retrieval improvement target, **not an expected failure to skip**.
Both tiers fail visibly. No baseline results have been fabricated or blessed.

All invariants must pass. Ranked positive queries target Hit@5 >= 0.90,
MRR@5 >= 0.75 and mean required-evidence recall@5 >= 0.90; forbidden evidence is
zero. Each case also specifies its own required evidence and maximum rank, and
those assertions must all pass for exit 0. See [REQUIREMENTS.md](REQUIREMENTS.md)
for the rationale, implementation findings, formulas and staged improvement plan.

Known source-level gaps include embedding-free document ranking, reference-style
Markdown, percent-encoded links, symbol-free configuration ingestion and
pre-declaration rationale ownership. Initial local runs may therefore be red.
Keep these failures as actionable evidence; do not weaken the oracle to get green.

## Manual scenarios

M01-M10 in `cases.json` specify freshness after mutations, preserved timestamps,
CLI/MCP parity, actual workspace routing, precise line-link targets, cross-language
policy changes, multilingual queries, >10,000-chunk saturation, root escapes and
provider/protocol errors. Always mutate disposable copies, never a real project.

G18 proves isolation of separately built snapshots only. It does **not** prove
PR #34's client-root negotiation, automatic initialization or long-lived MCP
routing. M04 is explicitly reserved for that end-to-end protocol qualification.
Static test references are not executed coverage; this fixture does not infer that
an application test ran merely because its node is connected.

## Optional CI without embeddings

The `Retrieval fixture contracts` workflow is `workflow_dispatch` only. It runs
`--check` and the Python evaluator tests, not Rust builds, indexing or models.
It is not a retrieval-quality gate. Once this workflow is available on the default
branch, it can be dispatched against a selected branch in GitHub Actions.

The same two commands are useful in any existing CI job. Real semantic qualification
should stay local or on an explicitly provisioned runner with a known cached model.
No automatic embedding workflow or new required status check is introduced.
