# RC3 retrieval development qualification

This packet compares RC2 with RC3 on fixed local fixtures and a bounded copy of
Codanna's own source. It does not authorize an Assign reindex or establish full
product, multilingual, watcher, or large-corpus qualification.

See the [measured results and improvement priorities](RESULTS.md) and
[machine-readable evidence](results.json).

## Reproduce

Use Python 3.11+ and build all three binaries from the revision being measured:

```sh
cargo build --locked --release --bin codanna --bin codanna-knowledge --bin codanna-index-plan
python3 contributing/retrieval/qualify-local.py \
  --codanna "$PWD/target/release/codanna" \
  --knowledge "$PWD/target/release/codanna-knowledge" \
  --cached-models /absolute/path/to/existing/models \
  --out /absolute/path/to/new-fixture-run --allow-local-model
python3 contributing/retrieval/qualify-repository.py \
  --codanna "$PWD/target/release/codanna" \
  --planner "$PWD/target/release/codanna-index-plan" \
  --cached-models /absolute/path/to/existing/models \
  --out /absolute/path/to/new-repository-run --allow-local-model
```

By default, both commands copy only a validated existing MiniLM snapshot, omit ambient
provider credentials and configuration overrides, and disable model downloads
with an unsupported Hugging Face endpoint scheme. All inference runs locally.
Missing cache files fail before indexing. Outputs cannot overwrite an earlier
run or reside inside this checkout. The repository probe copies eight explicit
source files; it does not index or mutate this checkout's active index or Assign.

For the separate real-source model comparison, pass
`--model MultilingualE5Small` to `qualify-repository.py` with that model's cache.
The fixed graded document evaluator's `prepare`, `collect`, and `score` commands
in [its instructions](../embedding-followups/SEMANTIC-EVALUATION.md#opt-in-manual-local-model-run)
support the same E5 snapshot. Downloads are a separate explicit preparation step;
neither qualification script downloads a model on a cache miss.

The fixture command preserves the original query manifest and source oracles.
It measures lexical, default comment-based semantic, and opt-in body-based
semantic profiles separately, then runs the fixed graded document experiment.
Its nonzero exit remains expected while any profile misses a target. A green
body profile does not conceal a failing default profile. The repository probe
uses six implementer-selected source/symbol judgments, records exact returned
ranks and source hashes, checks all six queries after a force rebuild, and
exercises create/update/delete through CLI reindexing. It is a development probe,
not a held-out benchmark or a watcher test.

## Implementation boundary

RC3 semantic document selection prefers distinct sources within the existing
bounded candidate pool and 90%-of-kth-positive-cosine floor. It relaxes source
quotas when needed to fill sparse or single-document results. Returned scores
remain the original cosine values. Lexical coverage ranking and source quotas
retain RC2 behavior. This query-time change requires no document re-embedding
or index-format migration.

The default code representation stays `doc_comment`. The existing opt-in
`symbol_body_v1` representation embeds undocumented implementations too, with
its existing bounded source/segment policy and separate cache identity. Changing
that policy requires an explicit rebuild; equal vector dimensions do not make
the two representations interchangeable. The new fixture flag records the
chosen policy rather than silently changing the original baseline.

The evaluator accepts workspace-parent aliases such as macOS `/var` while
rejecting source-level symlinks, workspace reentry, and parent traversal. Original
RC2 failures remain evidence; no query, relevance grade, or acceptance threshold
was weakened.

## Assign rollout gate

Do not run the full Assign reindex yet. Before a later rollout:

1. Validate the exact release binary and selected model/input policy on a
   disposable, representative Assign subset. Include undocumented code, JSX,
   configuration, multiple document sections, and the actual query languages.
2. Run the source-only `codanna-index-plan` with the intended settings. Inspect
   partial/blocked status, input rejections, body-segment counts, cache identity,
   and unknown values. The planner is not a provider-price quote and does not
   cover document collections or certify all caches will hit at scale.
3. Retain the old index and model/cache identity for rollback. Stage a separate
   index; compare symbol and eligible-vector counts, orphan vectors, provenance,
   source paths, and all required retrieval evidence before changing the active
   index. Do not delete caches as a rebuild prerequisite.
4. Qualify process restart, CLI/MCP parity, workspace routing, watcher mutations,
   generation consistency, and a corpus above 10,000 chunks. Measure peak RSS,
   disk growth, rebuild time, and query latency with the intended production
   model. The bounded candidate lookahead may still hide distinct sources when
   a very long document supplies many high-scoring chunks.
5. Keep automated validation local or mocked. A paid provider run requires the
   user's separate real-content scope, hard cap, and stop condition; this packet
   authorizes none.

Prefer improvements in this order: verify representative source coverage; review
native-language query/answer judgments; compare an appropriate cached multilingual
model with its actual input policy; measure source-aware candidate selection at
scale. Do not infer multilingual support from the aggregate MiniLM score or tune
against these fixtures and then call them unseen qualification.
