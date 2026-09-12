# GPU-accelerated local embeddings

Codanna can optionally run local FastEmbed/ONNX embedding inference through a hardware execution provider. CPU inference remains the default and existing installations do not need to change anything.

## Supported providers

| Provider | Target | Cargo feature | Runtime value |
| --- | --- | --- | --- |
| CPU | All platforms | none | `cpu` or unset |
| CoreML | macOS | `gpu-coreml` | `coreml` |
| CUDA | Linux / Windows with NVIDIA runtime support | `gpu-cuda` | `cuda` |

`auto` chooses the accelerated provider compiled for the current platform and falls back to CPU when provider registration is unavailable.

## macOS / CoreML

Build Codanna with CoreML support:

```sh
cargo build --release --features gpu-coreml
```

Run with automatic provider selection:

```sh
CODANNA_EMBED_PROVIDER=auto ./target/release/codanna index
```

Or request CoreML explicitly:

```sh
CODANNA_EMBED_PROVIDER=coreml ./target/release/codanna index
```

CoreML decides which Apple compute units execute supported model operations. Provider selection enables CoreML acceleration but does not guarantee that every operation executes on the GPU or Neural Engine.

## NVIDIA CUDA

Build Codanna with CUDA provider support:

```sh
cargo build --release --features gpu-cuda
```

Then run:

```sh
CODANNA_EMBED_PROVIDER=cuda ./target/release/codanna index
```

The machine must also have the native CUDA/cuDNN libraries required by the ONNX Runtime CUDA execution provider.

## Fallback behavior

Accelerated provider registration allows CPU fallback by default. This keeps Codanna usable when an execution provider is compiled into the binary but unavailable on the current machine.

To make provider registration strict instead:

```sh
CODANNA_EMBED_PROVIDER=coreml \
CODANNA_EMBED_PROVIDER_STRICT=1 \
./target/release/codanna index
```

In strict mode, ONNX Runtime session creation fails rather than silently using CPU when the requested provider cannot register.

## Library use

The `codanna` CLI configures the embedding runtime before creating any FastEmbed session. Applications embedding Codanna as a Rust library should call:

```rust
codanna::embedding_runtime::configure_embedding_runtime();
```

before constructing semantic-search or document-embedding components.

## Notes

- Remote embedding mode is unchanged and does not use these local execution-provider settings.
- The embedding model, tokenizer, dimensions, storage format, and semantic-search API are unchanged.
- CPU remains the default when `CODANNA_EMBED_PROVIDER` is unset.
