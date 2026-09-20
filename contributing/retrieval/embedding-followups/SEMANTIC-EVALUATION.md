# Graded semantic relevance follow-up

**Status: unrun.** No real embedding model was loaded to create this follow-up,
and it contains no measured model-quality result. The frozen
[status manifest](semantic-evaluation/status.json) sets `full_qualification` and
`semantic_relevance_measured` to `false`, with `metrics: null`.

The historical [retrieval runner](../run.py) already has an opt-in semantic
profile covering document and code retrieval. Its existing cases and oracles
remain unchanged. Green lexical checks and deterministic fixed-vector tests
establish ranking and storage behavior; they do not establish whether a model
places useful documents near a natural-language query. This isolated follow-up
adds graded source judgments, cross-language intent queries, explicit model
provenance, and source coverage measurements to address that evidence gap.

## Fixed evaluation boundary

[queries.json](semantic-evaluation/queries.json) contains 10 queries and 140
explicit judgments over 14 short English documents: four English queries, three
Spanish queries, and three Hungarian queries. Cases cover paraphrases, several
operational concepts with similar vocabulary, and a query requiring two distinct
sources. The event-batching query has a close distractor about notification
digests after indexing. Every source receives an explained grade for every query.

The grades and translations are **provisional fixture-author judgments**, without
independent human relevance review or native-speaker review. The corpus and
queries were authored together. “Heldout” describes their separation from the
index and their fixed evaluation role; it does not assert independent collection,
representative production sampling, or absence from a model's training data.
Do not tune ranking on this set and then describe it as unseen qualification.

Only `semantic-evaluation/corpus/docs/*.md` is copied into the searchable `docs/`
directory. Query text, titles, tags, grades, rationales, configuration, model
files, and reports remain outside that collection path. Validation rejects a
complete query copied into a source, unexpected corpus files, missing judgments,
duplicate query IDs, and symlinks. Hashes bind exact source bytes, source paths,
and the query manifest. This catches boundary mistakes; it cannot prove semantic
independence of author-created questions and documents.

Judgments apply to a **whole source**, not to exact answer passages. A returned
chunk inherits its source grade, which can over-credit a passage that omits the
useful fact. Raw source byte spans and their UTF-8 boundaries are validated and
retained for manual inspection. The original acceptance runner remains the place
for required-fact-in-chunk assertions.

## Model-free commands

Python 3.11+ and the standard library are sufficient. These commands never invoke
Codanna, provider transports, inference, downloads, or credential-bearing services:

```bash
python3 contributing/retrieval/embedding-followups/semantic-evaluation/evaluate.py check
python3 -m unittest discover \
  -s contributing/retrieval/embedding-followups/semantic-evaluation \
  -p 'test_evaluate.py' -v
```

The tests use invented JSON, dummy cache bytes, and small local Python child
processes to verify bounded output capture and termination. Their apparent scores are
metric arithmetic fixtures, not real model output, and are never committed as
quality evidence. The separate `Semantic evaluation contracts` workflow runs
only these two commands for changes to this harness. It never builds Codanna,
starts the manual collector, or accesses a model. A passing workflow is an
evaluator-contract result, not a semantic-quality gate.

## Opt-in manual local-model run

The collector supports the two local models below using the existing public CLI.
It is intended for macOS and Linux and requires an already built binary from the
revision being evaluated. No Rust build is performed by the harness.

| Model | Cached repository | Weight file in selected snapshot |
| --- | --- | --- |
| `AllMiniLML6V2` | `models--Qdrant--all-MiniLM-L6-v2-onnx` | `model.onnx` |
| `MultilingualE5Small` | `models--intfloat--multilingual-e5-small` | `onnx/model.onnx` |

Use a model that is **already present locally**. Both models also require the
snapshot's `tokenizer.json`, `config.json`, `special_tokens_map.json`, and
`tokenizer_config.json`. The repository's `refs/main` must contain the exact
40-character snapshot identifier without a newline. Missing files or invalid
refs fail before a process is launched. The harness copies and hashes only those
five selected files and the ref, preserving cache layout and dereferencing
internal blob symlinks; it does not copy the rest of a user's model cache.

For a cached MiniLM snapshot, run from the checkout root:

```bash
MODEL_CACHE="$HOME/.codanna/models"
MODEL_REVISION="$(cat "$MODEL_CACHE/models--Qdrant--all-MiniLM-L6-v2-onnx/refs/main")"
EVAL_RUN="$(mktemp -d)/minilm"
CODANNA_BIN="$PWD/target/release/codanna"

python3 contributing/retrieval/embedding-followups/semantic-evaluation/evaluate.py prepare \
  --out "$EVAL_RUN" --model AllMiniLML6V2 --model-revision "$MODEL_REVISION"

# MANUAL INFERENCE: the explicit flag is required even when a cache exists.
python3 contributing/retrieval/embedding-followups/semantic-evaluation/evaluate.py collect \
  --run "$EVAL_RUN" --codanna "$CODANNA_BIN" --cached-models "$MODEL_CACHE" \
  --allow-local-model

# This scoring step reads captured files only; it never loads the model.
python3 contributing/retrieval/embedding-followups/semantic-evaluation/evaluate.py score \
  --run "$EVAL_RUN"
```

For E5, prepare a separate new directory with `--model MultilingualE5Small` and
the revision read from `models--intfloat--multilingual-e5-small/refs/main`.
Keep the corpus, query hash, generated chunk settings, binary, and result budget
equal when comparing models. Examine language-specific rows; do not infer an
English model has multilingual support from a global average. The production
embedding input policy is used as-is, including any model-specific prefix
limitations of the existing backend. This harness does not rewrite queries or
document inputs to improve results.

`prepare` creates only an unrun workspace and local-only configuration.
`collect` has one indexing pass and at most ten top-five document searches, plus
a version request and two diagnostics requests. Indexing is capped at ten minutes;
each other process is capped at two minutes. It stops on the first failed or
malformed query. Each stdout/stderr capture is limited to 16 MiB while the process
runs; reaching the limit stops its process group and retains the bounded prefix.
Imported stdout and stderr have the same bound before hashing or parsing.
The 14-document corpus is deliberately bounded, but the exact
chunk count is measured from diagnostics rather than inferred from file count.
No remote embedding-provider configuration or paid-inference mode is supported.

The subprocess environment is an allowlist. It excludes inherited API keys,
tokens, proxies, Codanna provider overrides, `CI_` settings overrides, and user
configuration. `HOME` and `HF_HOME` point at the private copied cache. It sets
`HF_ENDPOINT=codanna-offline://cache`: cached files work, while a cache miss fails
on an unsupported URL scheme before connecting. This behavior was verified by
source inspection for the pinned `fastembed 5.6.0` → `hf-hub 0.4.3` → `ureq 2.12.1`
path; it was not verified by loading a model. FastEmbed honors `HF_ENDPOINT`,
hf-hub checks its local cache before downloading, and ureq rejects this scheme
before connection. Python-style `HF_HUB_OFFLINE` flags are not relied on.
Recheck this mechanism and the required file map when those dependencies change.

Outputs and models must stay outside the checkout. Existing experiment output
directories are not overwritten, and a collection cannot reuse an existing
`home/`, `raw/`, or `capture.json`. Retain the run in its original absolute
location while scoring: configuration and recorded argument lists deliberately
bind that location. To evaluate another binary, prepare a new run. Timings include
CLI process/model startup and are diagnostic only, not a warm-latency benchmark.

## Metric and failure contracts

Returned CLI order is authoritative, including equal cosine scores. The evaluator
never reranks it. Each result consumes its original top-five slot. Only the first
chunk from a source receives relevance gain; duplicate chunks cannot improve
recall, nDCG, or source coverage and do not give later results a free promotion.

| Measurement | Definition |
| --- | --- |
| Source nDCG@5 | Gain `(2^grade - 1) / log2(rank + 1)` at original chunk rank; repeated sources get zero gain. The ideal ranking contains each judged source once, sorted by grade. |
| Source Hit@5 and reciprocal rank@5 | A relevant source has grade 2 or 3. Use its first original chunk rank; a miss is zero. Aggregate reciprocal rank is MRR@5. |
| Source recall@5 | Distinct grade-2/3 sources found divided by all grade-2/3 sources for that query. |
| Grade-3 source recall@5 | Distinct direct-answer/required-facet sources found divided by all grade-3 sources. This exposes missing facets of `q03`. |
| Source diversity | Distinct sources, relevant sources, grade-zero sources, duplicate slots, duplicate fraction, and returned chunk count within the first five. |

Means and denominators are reported overall, by query language, and by case tag.
Grade 1 earns small graded gain but does not count as binary relevance. The scorer
supports all-zero judgments: their positive-retrieval metrics are null and excluded
from those means, with returned grade-zero sources still counted. This corpus has
no absent-term abstention gate; nearest-neighbor retrieval is allowed to return
results for an unrelated query. Unknown sources and incomplete judgment sets are
errors, never implicit grade-zero assignments.

A valid `NOT_FOUND` response has zero positive metrics. A failed process, protocol
error, wrong query/collection, nonfinite cosine, malformed JSON, invalid byte
span, stale hash, or foreign path is an error. Missing or failed query rows retain
zero positive scores in the expected-query denominator, and the report becomes
`incomplete`. A query failure can leave final diagnostics absent; valid initial
diagnostics still permit those incomplete denominators. A present contradictory
model identity, input policy, generation, or document/vector count invalidates the
whole run. Invalid experiment/corpus/model provenance instead produces an
`invalid_capture` report with no aggregate. Neither can qualify semantic quality.

`capture.json` retains command argument lists, return codes, raw stdout/stderr
paths and hashes, version evidence, the binary SHA-256, host metadata,
and copied model artifact hashes. It binds `experiment.json` through
`experiment_sha256`; the evaluator SHA-256 is recorded in `experiment.json` and
`report.json`. Scoring verifies the raw version/index evidence as well
as both diagnostics requests and every query. The actual
`documents stats semantic_eval --json` identity must specify the expected local
model, revision label, exact tokenizer fingerprint, effective input budget, and
`document-input=2`. Chunk/file counts, live/physical vectors, zero unembedded
chunks, and an unchanged generation are required.

The `model_revision` setting is an identity label; it does not tell FastEmbed to
download a particular revision. Actual copied snapshot/ref paths and file hashes
identify the bytes evaluated. The diagnostic tokenizer hash describes the
normalized counting tokenizer; the raw tokenizer-file SHA-256 is retained
separately and is not assumed to be the same digest. These are reproducibility
records, not an attestation that imported JSON could never be fabricated.

## Raw cosine and selection comparison

The current public `documents search --json` contract returns the selected result
list after production source diversification. It does not expose a raw-cosine
mode or the complete pre-selection candidate set. Every report therefore records
`raw_cosine_comparison.status: unsupported_by_public_cli` and null raw metrics.
Sorting the selected rows by their cosine cannot reconstruct omitted candidates
and must not be called a raw baseline. Increasing `--limit` also changes the
candidate cutoff and selection behavior, so it is not an equivalent control.

No production ranking or CLI change is included. A future independently scoped
comparison needs an explicit public surface that runs both modes over the same
candidate set, query vectors, index generation, and budget, or a separately
justified instrumented experiment with its own provenance.

## Evaluator fixture checklist

The complete [36-fixture catalog](semantic-evaluation/fixtures.json) maps each
entry below to its exact unittest name, source file, scenario, and asserted
oracle. All 36 entries passed the final-source rerun. The catalog includes
source hashes and the [complete contract test log](semantic-evaluation/contract-tests.txt). This records evaluator-contract evidence separately
from the unchanged [unrun semantic status](semantic-evaluation/status.json).

| ID | Contract |
| --- | --- |
| SE01 | Graded nDCG arithmetic |
| SE02 | Repeated source budget |
| SE03 | Original ranks after duplicates |
| SE04 | Partial relevance threshold |
| SE05 | Empty positive retrieval |
| SE06 | All-zero judgments |
| SE07 | Unjudged source rejection |
| SE08 | Failed-query denominator |
| SE09 | Prepared unrun state |
| SE10 | Index and label separation |
| SE11 | Query leakage and extra files |
| SE12 | Judgment completeness and types |
| SE13 | Equal-score order |
| SE14 | Empty result versus error |
| SE15 | Envelope and cosine validation |
| SE16 | Source identity and containment |
| SE17 | Coordinates and response scope |
| SE18 | Required diagnostic identity |
| SE19 | Isolated collection environment |
| SE20 | Manual opt-in boundary |
| SE21 | Live output bounds |
| SE22 | Process timeout |
| SE23 | Imported output bounds |
| SE24 | Cache preflight rejection |
| SE25 | Supported cache layouts |
| SE26 | Mocked collection workflow |
| SE27 | Model-free captured scoring |
| SE28 | Missing captured query |
| SE29 | Query error versus global drift |
| SE30 | Stable generation with count drift |
| SE31 | Invalid model provenance |
| SE32 | Stopped collection denominator |
| SE33 | Recorded command mismatch |
| SE34 | Changed manifest or model bytes |
| SE35 | Malformed capture report |
| SE36 | Strict JSON parsing |

## Qualification checklist

- [x] Keep historical retrieval corpus, oracles, and runner unchanged.
- [x] Define a fixed source corpus, explicit grades/rationales, and query separation.
- [x] Validate deterministic metric arithmetic, input contracts, cache selection,
  credential isolation, and mocked manual collection without a model.
- [x] Commit unrun status with exact corpus/query hashes and null quality metrics.
- [ ] Obtain independent relevance-label and Spanish/Hungarian wording review.
- [ ] Run the manual collector against already cached local model bytes and retain
  the original run directory, raw captures, diagnostics, and report.
- [ ] Review relevant returned passages, misses, distractors, multilingual results,
  and both required sources of `q03` before drawing a relevance conclusion.
- [ ] Run the historical semantic profile and pending manual qualification cases
  separately under their documented scope when explicitly intended.
- [ ] Establish representative production evidence and any future raw-cosine
  comparison before changing or endorsing production ranking behavior.

Even a complete run uses `full_qualification: false`: this small synthetic
source-level experiment cannot complete broader product qualification. A `score`
exit code of 0 means a complete, valid measurement, not that relevance passed a
new quality threshold. No numeric quality gate is invented from unrun data.
