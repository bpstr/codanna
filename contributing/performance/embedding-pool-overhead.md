# Embedding pool scheduling overhead (proposed; qualification pending)

This is deliberately separate from shared native CPU controls and cross-symbol batching.
It does not change model-instance count, native-thread settings, input filtering, batch-size
selection, tokenizer validation, error policy, storage format or v2 source coverage.

The input batch is already a slice. Schedule its chunks through Rayon's indexed parallel
slice iterator instead of converting a sequential chunks iterator with `par_bridge`.
Keep the existing dedicated embedding pool and its 1:1 instance/worker relationship;
model checkout must not occupy a global Rayon pool needed by nested tokenizer work.

Automatic usage reports are limited to once every ten seconds per embedding pool with a
nonblocking gate. Explicit progress/final `log_usage_stats` calls remain available. When
semantic INFO logging is disabled, return before allocating counter/string summaries.
Do not interpret usage counters as wall time, CPU time, or proof of device execution.

Five model-free fixtures exercise the actual scheduling helper (ordered uneven slices,
nested work on the dedicated pool, empty/single inputs and error propagation) and the
logging gate (time boundaries and contention without sleeps). Existing pool checkout,
unwind safety, whitespace preservation and batch-size tests are retained. Run:

```sh
cargo test --lib semantic::pool
```

Real-model tests remain explicitly ignored; normal qualification must not download a
model or invoke a paid endpoint. Rust tests/Clippy/rustfmt and Mac throughput measurements
were not run in the editing environment. Use existing repository gates and same-corpus
measurements before approval. This is a small overhead change, not a claimed explanation
for all high CPU use or a promised speedup.
