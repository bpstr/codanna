# Shared embedding CPU budget (proposed; qualification pending)

This downstream change keeps the index format, v2 source selection, embedding model,
tokenizer policy and vector identity unchanged. It does not repair RC4 crash recovery.
Use disposable staging, never an interrupted incremental retry of the only good index.

## Laptop profile

Merge these keys into the existing settings, preserving the current model and source policy:

```toml
[indexing]
parallelism = 2

[semantic_search]
embedding_threads = 1
```

For the next indexing invocation, set:

```sh
export CODANNA_EMBED_CPU_THREADS=2
export CODANNA_EMBED_ALLOW_SPINNING=0
```

These variables affect the new process only. They require this PR's binary, not unmodified RC4.
The first variable sets the process-wide ORT intra-op pool to 2; global inter-op concurrency
is 1. Setting either control opts in. Spinning defaults to false after opt-in. A spinning-only
override chooses `clamp(logical_cpus - 2, 1, 4)` intra-op threads. Explicit thread counts must
be integers from 1 through 1024. Invalid controls or an already-initialized runtime are errors:
we must not report a resource limit while actually running unconstrained.

This is **not a whole-process CPU quota**. Model callers, tokenizers, parsers, filesystem
workers and Tantivy still contribute concurrency. `embedding_threads` is the model-instance
count; it is independent of the shared native worker count. Multiple Codanna processes also
each have their own ORT environment. Choose one instance for the initial quiet-mode trial.
Unset both CPU variables to retain the original lazy/default threading behavior. Explicit
CPU controls initialize the environment earlier and can affect short-command startup;
include that cost in qualification rather than treating this as an already-proven default.

Provider selection and CPU threading are committed together. CoreML/CUDA fallback workers
use the same shared CPU pool. Existing provider strictness is retained; explicit CPU limits
are fail-closed even when GPU registration is optional. A provider-registration message does
not establish accelerator device execution.

## Why this works with the pinned dependencies

FastEmbed 5.6.0 explicitly sets available parallelism on every new session. ORT rc.10's
`commit_from_file` and `commit_from_memory` call `DisablePerSessionThreads` when their
environment has a global pool. Therefore no FastEmbed fork, replacement inference engine,
model change, unsafe session mutation, or dependency-version upgrade is required.

Primary implementation references inspected for this change:

- https://github.com/Anush008/fastembed-rs/blob/v5.6.0/src/text_embedding/impl.rs
- https://github.com/pykeio/ort/blob/v2.0.0-rc.10/src/session/builder/impl_commit.rs
- https://github.com/pykeio/ort/blob/v2.0.0-rc.10/src/environment.rs
- https://onnxruntime.ai/docs/performance/tune-performance/threading.html

The existing direct ORT dev dependency becomes a normal CPU dependency at the same exact
version. Accelerator feature flags remain opt-in; their platform-gated runtime dispatch is
unchanged. The generic GPU feature enables both ORT provider bindings, while Codanna exposes
only the platform-appropriate provider. ORT's pinned distribution selector falls back to the
platform's non-CUDA distribution on Apple. Verify packaging on native Apple and Linux.

## Required qualification (not performed by this document)

Run `cargo test --lib embedding_runtime::tests` without GPU features, then with all features;
run the repository quick/full checks and a native macOS build. Tests use an injected commit
function and do not download models, contact providers, or consume credentials.

Use identical frozen corpora and cached model weights; alternate baseline/candidate trials.
Separate graph-only indexing, comment embeddings, v2 body embeddings and document indexing.
Measure wall time, process CPU time, peak RSS, native thread count and actual batch occupancy.
Compare one model instance with intra=1/2/3 and two instances with shared intra=2, not an
unbounded Cartesian sweep. Record no-op/first-query startup separately from sustained work.
Require identical structural/source-range coverage and numerical/retrieval tolerances.

CoreML is a separate manual qualification: build `--features gpu-coreml`, select
`CODANNA_EMBED_PROVIDER=coreml` and `CODANNA_EMBED_PROVIDER_STRICT=1`, keep the model and input
policy fixed, and verify actual device execution. Do not label registration a performance
pass. No Mac heat, energy or speedup claim is made before those measurements.

## Memory-aware optional provider selection

Non-strict `CODANNA_EMBED_PROVIDER=auto` uses CPU when the sampled memory budget
has less than 4 GiB of headroom. This is a conservative admission heuristic, not
a measured accelerator performance threshold. Strict `auto` and explicit
`coreml`/`cuda` requests retain their registration behavior. Explicit CPU thread
and spinning controls still apply when optional `auto` falls back to CPU. The
default CPU path does not sample memory for provider selection.
