//! Process-wide ONNX Runtime execution-provider selection for local embeddings.
//!
//! Codanna uses fastembed in several independent code paths. Configuring ORT once
//! before those sessions are created keeps provider selection consistent across
//! code semantic search, document embeddings, and query-time embeddings.
//!
//! Runtime selection is controlled by `CODANNA_EMBED_PROVIDER`:
//! - unset / `cpu`: keep the existing CPU-only behavior
//! - `auto`: prefer CoreML on Apple targets or CUDA on Linux/Windows when the
//!   matching Cargo feature was compiled in
//! - `coreml`: request CoreML explicitly
//! - `cuda`: request CUDA explicitly
//!
//! By default provider registration is allowed to fall back to CPU. Set
//! `CODANNA_EMBED_PROVIDER_STRICT=1` to fail session creation if the requested
//! execution provider cannot be registered.

use fastembed::ExecutionProviderDispatch;
use std::str::FromStr;

const PROVIDER_ENV: &str = "CODANNA_EMBED_PROVIDER";
const STRICT_ENV: &str = "CODANNA_EMBED_PROVIDER_STRICT";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingExecutionProvider {
    Cpu,
    Auto,
    CoreMl,
    Cuda,
}

impl FromStr for EmbeddingExecutionProvider {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "cpu" => Ok(Self::Cpu),
            "auto" => Ok(Self::Auto),
            "coreml" | "core-ml" => Ok(Self::CoreMl),
            "cuda" => Ok(Self::Cuda),
            other => Err(format!(
                "unsupported embedding execution provider '{other}'; expected cpu, auto, coreml, or cuda"
            )),
        }
    }
}

fn strict_provider_registration() -> bool {
    std::env::var(STRICT_ENV)
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn finalize_dispatch(
    dispatch: ExecutionProviderDispatch,
    strict: bool,
) -> ExecutionProviderDispatch {
    if strict {
        dispatch.error_on_failure()
    } else {
        dispatch.fail_silently()
    }
}

#[cfg(all(feature = "gpu-coreml", target_vendor = "apple"))]
fn coreml_dispatch(strict: bool) -> Option<ExecutionProviderDispatch> {
    use ort::execution_providers::CoreMLExecutionProvider;
    Some(finalize_dispatch(
        CoreMLExecutionProvider::default().build(),
        strict,
    ))
}

#[cfg(not(all(feature = "gpu-coreml", target_vendor = "apple")))]
fn coreml_dispatch(_strict: bool) -> Option<ExecutionProviderDispatch> {
    None
}

#[cfg(all(
    feature = "gpu-cuda",
    any(target_os = "linux", target_os = "windows")
))]
fn cuda_dispatch(strict: bool) -> Option<ExecutionProviderDispatch> {
    use ort::execution_providers::CUDAExecutionProvider;
    Some(finalize_dispatch(
        CUDAExecutionProvider::default().build(),
        strict,
    ))
}

#[cfg(not(all(
    feature = "gpu-cuda",
    any(target_os = "linux", target_os = "windows")
)))]
fn cuda_dispatch(_strict: bool) -> Option<ExecutionProviderDispatch> {
    None
}

fn provider_dispatch(
    provider: EmbeddingExecutionProvider,
    strict: bool,
) -> Option<(&'static str, ExecutionProviderDispatch)> {
    match provider {
        EmbeddingExecutionProvider::Cpu => None,
        EmbeddingExecutionProvider::CoreMl => {
            coreml_dispatch(strict).map(|dispatch| ("coreml", dispatch))
        }
        EmbeddingExecutionProvider::Cuda => {
            cuda_dispatch(strict).map(|dispatch| ("cuda", dispatch))
        }
        EmbeddingExecutionProvider::Auto => {
            #[cfg(target_vendor = "apple")]
            if let Some(dispatch) = coreml_dispatch(strict) {
                return Some(("coreml", dispatch));
            }

            #[cfg(any(target_os = "linux", target_os = "windows"))]
            if let Some(dispatch) = cuda_dispatch(strict) {
                return Some(("cuda", dispatch));
            }

            None
        }
    }
}

/// Configure the process-wide ONNX Runtime environment for local embeddings.
///
/// Call this before any fastembed model is constructed. The Codanna CLI invokes
/// it at the start of `main`; library consumers can call it explicitly when they
/// want the same optional GPU acceleration.
pub fn configure_embedding_runtime() {
    let raw = match std::env::var(PROVIDER_ENV) {
        Ok(value) => value,
        Err(_) => return,
    };

    let provider = match EmbeddingExecutionProvider::from_str(&raw) {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!("codanna: {error}; using CPU embeddings");
            return;
        }
    };

    if provider == EmbeddingExecutionProvider::Cpu {
        return;
    }

    let strict = strict_provider_registration();
    let Some((provider_name, dispatch)) = provider_dispatch(provider, strict) else {
        eprintln!(
            "codanna: embedding provider '{raw}' is not compiled for this target; using CPU embeddings"
        );
        return;
    };

    let committed = ort::init()
        .with_execution_providers([dispatch])
        .commit();

    if committed {
        eprintln!(
            "codanna: local embeddings configured for {provider_name}{}",
            if strict { " (strict)" } else { " with CPU fallback" }
        );
    } else {
        eprintln!(
            "codanna: ONNX Runtime was already initialized before embedding provider selection; existing runtime configuration is unchanged"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_execution_provider_names() {
        assert_eq!(
            EmbeddingExecutionProvider::from_str("cpu").unwrap(),
            EmbeddingExecutionProvider::Cpu
        );
        assert_eq!(
            EmbeddingExecutionProvider::from_str("AUTO").unwrap(),
            EmbeddingExecutionProvider::Auto
        );
        assert_eq!(
            EmbeddingExecutionProvider::from_str("core-ml").unwrap(),
            EmbeddingExecutionProvider::CoreMl
        );
        assert_eq!(
            EmbeddingExecutionProvider::from_str("cuda").unwrap(),
            EmbeddingExecutionProvider::Cuda
        );
    }

    #[test]
    fn rejects_unknown_execution_provider() {
        let error = EmbeddingExecutionProvider::from_str("metal").unwrap_err();
        assert!(error.contains("expected cpu, auto, coreml, or cuda"));
    }
}
