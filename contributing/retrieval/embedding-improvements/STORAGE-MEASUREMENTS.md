# Measured document storage across edits

After 100 edits to the same eight one-chunk documents, the updated implementation
stored **18,560 vector-file bytes and 12 physical vector records**, compared with
**166,336 bytes and 108 records** at baseline. Both retained **eight live chunks**,
including after reopening. This is an **88.8% reduction in stored vector-file
bytes** at the final checkpoint.

The unchanged [public-API probe](document_churn.rs) generated both the
[baseline measurements](churn-baseline.json) and [updated measurements](churn-final.json).
It calls `DocumentStore::index_collection`, then performs 100 round-robin
`reindex_file` operations. Each edit changes one source file; each source remains
one chunk. A deterministic local generator produces 384-dimensional vectors.
No model, provider or network inference is involved.

| Completed edits | Live chunks, both runs | Baseline vector records | Updated vector records | Baseline vector-file bytes | Updated vector-file bytes |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 8 | 8 | 8 | 12,336 | 12,336 |
| 1 | 8 | 9 | 9 | 13,876 | 13,892 |
| 10 | 8 | 18 | 10 | 27,736 | 15,448 |
| 25 | 8 | 33 | 9 | 50,836 | 13,892 |
| 50 | 8 | 58 | 10 | 89,336 | 15,448 |
| 75 | 8 | 83 | 11 | 127,836 | 17,004 |
| 100 | 8 | 108 | 12 | 166,336 | 18,560 |

The probe recursively reads actual `.vec` files beneath the store's `vectors/`
directory. It validates each CVEC header and checks the file length against
`16 + records * (4 + dimensions * 4)`. Live chunk and file counts come from the
public `collection_stats` API. These measurements therefore work across the
baseline's single vector file and the updated immutable-segment layout.

The extra 16 bytes after the first edit are the additional segment header. At
the last checkpoint, the baseline has one vector file and the updated store has
five. Linux filesystem allocation for those files is **167,936 bytes** at baseline
and **32,768 bytes** after the change, an **80.5% reduction**. Allocation is measured
as `st_blocks * 512` and depends on the filesystem; file lengths are recorded
separately.

The measurements support the effect of live-record compaction on repeated-edit
storage growth. [Storage and recovery behavior](EMBEDDING-STORAGE.md) describes
the active-generation bounds, bounded copying, temporary publication space and
snapshot retention. This probe has no long-lived query snapshot, so it does not
measure blocks retained by old mapped generations. It also does not measure
Tantivy files, generation JSON, caches, total I/O written or peak temporary space.

The single local runs took **4,246 ms** at baseline and **5,532 ms** after the
change. Other validation ran concurrently, and the updated store performs extra
durability and writer-lifecycle work. The recorded updated run was slower; these
samples establish no latency improvement. Use isolated repeated runs on a
representative corpus before drawing throughput or production-latency conclusions.

## Provenance and reproduction

The baseline library came from
`dd8e247c62dcfd61dd1cdb7feaf23fe3777a469d`. Both libraries used native Rust 1.98.1,
the default HTTP features, a development build with debug symbols and incremental
compilation disabled, and one codegen unit for the `codanna` package. CPU ONNX
Runtime 1.22.0 was linked for the package's runtime dependency. The probe itself
uses only its deterministic generator.

The updated library was rebuilt from the frozen production/test source tree
`a4dff01b27eb17c5f2db9f23b4375994732aa48e791e3bcbf5af4ec68a93498b`
(635 files across `src/`, `tests/`, `Cargo.toml` and `Cargo.lock`). Both runs
compiled the identical probe source directly against a copied library artifact. The probe SHA256 is
`9735a4d1a30388632c4a6f945a2908adf7f8dee566bc5c4141c84fe40a934761`.
The updated library SHA256 is
`a880021c50df1add5525b673e478659bca7dc7847b249660fbb5043dc10cff92`.
The raw updated report SHA256 is
`edd39ece7e3683fff9f22eecb7e85e991b734b72cd05fdd7d32c42da50af6064`.

Build the library for each revision with the same toolchain, profile, features
and runtime setup. Run the probe in a credential-free environment. For the
dynamically linked Linux setup, after setting `ORT_LIB_LOCATION` to the native
library directory:

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=false CARGO_BUILD_JOBS=6 \
  cargo --config 'profile.dev.package.codanna.codegen-units=1' build --locked --lib
churn_target_dir="$(cargo metadata --locked --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
rustc --edition 2024 contributing/retrieval/embedding-improvements/document_churn.rs \
  --extern "codanna=$churn_target_dir/debug/libcodanna.rlib" \
  -L "dependency=$churn_target_dir/debug/deps" \
  -L "native=$ORT_LIB_LOCATION" \
  -C "link-arg=-Wl,-rpath,$ORT_LIB_LOCATION" \
  -o /tmp/codanna-document-churn
/tmp/codanna-document-churn /tmp/codanna-churn-new-run > /tmp/codanna-churn-result.json
```

The output workspace must not already exist. Keep a copy of the same probe when
checking out the baseline, which predates this file. Retain each JSON report and
use a distinct workspace for each run. Match the profile settings listed above
when comparing timings; the stored byte and record measurements are the primary
evidence reported here.
