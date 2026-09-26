//! Process-wide ONNX Runtime execution-provider selection for local embeddings.
//!
//! Codanna uses fastembed in several independent code paths. Configuring ORT once
//! before those sessions are created keeps provider selection consistent across
//! code semantic search, document embeddings, and query-time embeddings.
//!
//! Runtime selection is controlled by `CODANNA_EMBED_PROVIDER`:
//! - unset / `cpu`: keep the existing CPU-only behavior
//! - `auto`: prefer CoreML on Apple targets or CUDA on Linux/Windows when the
//!   GPU embedding Cargo feature was compiled in
//! - `coreml`: request CoreML explicitly
//! - `cuda`: request CUDA explicitly
//!
//! By default provider registration is allowed to fall back to CPU. Set
//! `CODANNA_EMBED_PROVIDER_STRICT=1` to fail session creation if the requested
//! execution provider cannot be registered. Strict selection also rejects invalid
//! names, unavailable compiled capabilities, and an already initialized runtime.

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

#[cfg(feature = "gpu-embeddings")]
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

#[cfg(all(feature = "gpu-embeddings", target_vendor = "apple"))]
fn coreml_dispatch(strict: bool) -> Option<ExecutionProviderDispatch> {
    use ort::execution_providers::CoreMLExecutionProvider;
    Some(finalize_dispatch(
        CoreMLExecutionProvider::default().build(),
        strict,
    ))
}

#[cfg(not(all(feature = "gpu-embeddings", target_vendor = "apple")))]
fn coreml_dispatch(_strict: bool) -> Option<ExecutionProviderDispatch> {
    None
}

#[cfg(all(
    feature = "gpu-embeddings",
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
    feature = "gpu-embeddings",
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

#[cfg(feature = "gpu-embeddings")]
fn commit_runtime_provider(dispatch: ExecutionProviderDispatch) -> Result<bool, String> {
    ort::init()
        .with_execution_providers([dispatch])
        .commit()
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "gpu-embeddings"))]
fn commit_runtime_provider(_dispatch: ExecutionProviderDispatch) -> Result<bool, String> {
    Ok(false)
}

/// Providers compiled into this binary. This does not initialize ONNX Runtime
/// or claim that a provider can execute a particular model on an accelerator.
pub fn compiled_embedding_providers() -> Vec<&'static str> {
    let mut providers = vec!["cpu"];
    if cfg!(all(feature = "gpu-embeddings", target_vendor = "apple")) {
        providers.push("coreml");
    }
    if cfg!(all(
        feature = "gpu-embeddings",
        any(target_os = "linux", target_os = "windows")
    )) {
        providers.push("cuda");
    }
    providers
}

fn selection_failure(message: String, strict: bool) -> Result<String, String> {
    if strict {
        Err(message)
    } else {
        Ok(format!(
            "{message}; leaving the existing runtime configuration unchanged"
        ))
    }
}

fn configure_provider(
    raw: &str,
    strict: bool,
    available: &[&str],
    commit: impl FnOnce(EmbeddingExecutionProvider, bool) -> Result<bool, String>,
) -> Result<Option<String>, String> {
    let provider = match EmbeddingExecutionProvider::from_str(raw) {
        Ok(provider) => provider,
        Err(error) => return selection_failure(error, strict).map(Some),
    };
    let name = match provider {
        EmbeddingExecutionProvider::Cpu => return Ok(None),
        EmbeddingExecutionProvider::CoreMl => "coreml",
        EmbeddingExecutionProvider::Cuda => "cuda",
        EmbeddingExecutionProvider::Auto => available
            .iter()
            .copied()
            .find(|name| *name != "cpu")
            .unwrap_or("cpu"),
    };
    if name == "cpu" || !available.contains(&name) {
        return selection_failure(
            format!(
                "embedding provider '{raw}' is not compiled for this target (compiled: {}); \
                 Apple builds require --features gpu-coreml",
                available.join(", ")
            ),
            strict,
        )
        .map(Some);
    }
    let selected = EmbeddingExecutionProvider::from_str(name)?;
    let message = match commit(selected, strict) {
        Ok(true) => format!(
            "local embedding provider {name} configured{}; session registration and actual device execution are not yet verified",
            if strict {
                " (strict registration)"
            } else {
                " with CPU fallback"
            }
        ),
        Ok(false) => selection_failure(
            "ONNX Runtime was already initialized before embedding provider selection".into(),
            strict,
        )?,
        Err(error) => selection_failure(
            format!("failed to configure {name} embedding provider: {error}"),
            strict,
        )?,
    };
    Ok(Some(message))
}

/// Configure the process-wide ONNX Runtime environment before creating models.
///
/// Strict selection errors are returned to the caller. Successful configuration
/// only installs a provider preference; registration occurs during session
/// creation and unsupported graph nodes may still execute on CPU.
/// Library consumers must call this before constructing a fastembed model.
pub fn configure_embedding_runtime() -> Result<(), String> {
    let raw = match std::env::var(PROVIDER_ENV) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    let message = configure_provider(
        &raw,
        strict_provider_registration(),
        &compiled_embedding_providers(),
        |provider, strict| {
            let (_, dispatch) = provider_dispatch(provider, strict)
                .ok_or_else(|| "compiled provider dispatch is unavailable".to_string())?;
            commit_runtime_provider(dispatch)
        },
    )?;
    if let Some(message) = message {
        eprintln!("codanna: {message}");
    }
    Ok(())
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

    #[test]
    fn unavailable_strict_provider_fails_before_runtime_initialization() {
        for requested in ["coreml", "cuda", "auto", "metal"] {
            let result = configure_provider(requested, true, &["cpu"], |_, _| {
                panic!("unavailable providers must not initialize a runtime")
            });
            assert!(result.is_err(), "{requested}");
        }
    }

    #[test]
    fn unavailable_optional_provider_explains_fallback() {
        let message = configure_provider("coreml", false, &["cpu"], |_, _| {
            panic!("unavailable providers must not initialize a runtime")
        })
        .unwrap()
        .unwrap();
        assert!(message.contains("not compiled"));
        assert!(message.contains("--features gpu-coreml"));
        assert!(message.contains("runtime configuration unchanged"));
    }

    #[test]
    fn strict_runtime_failures_are_not_swallowed() {
        for outcome in [Ok(false), Err("prepared initialization failure".into())] {
            assert!(
                configure_provider("coreml", true, &["cpu", "coreml"], |_, strict| {
                    assert!(strict);
                    outcome
                })
                .is_err()
            );
        }
    }

    #[test]
    fn optional_runtime_failures_explain_unchanged_configuration() {
        for outcome in [Ok(false), Err("prepared initialization failure".into())] {
            let message = configure_provider("coreml", false, &["cpu", "coreml"], |_, _| outcome)
                .unwrap()
                .unwrap();
            assert!(message.contains("existing runtime configuration unchanged"));
            assert!(!message.contains("configured with CPU fallback"));
        }
    }

    #[test]
    fn auto_selects_compiled_accelerator_without_claiming_device_execution() {
        for (name, provider) in [
            ("coreml", EmbeddingExecutionProvider::CoreMl),
            ("cuda", EmbeddingExecutionProvider::Cuda),
        ] {
            let message = configure_provider("auto", true, &["cpu", name], |selected, strict| {
                assert_eq!(selected, provider);
                assert!(strict);
                Ok(true)
            })
            .unwrap()
            .unwrap();
            assert!(message.contains(name));
            assert!(message.contains("not yet verified"));
        }
    }

    #[test]
    fn explicit_cpu_does_not_initialize_runtime() {
        assert!(
            configure_provider("cpu", true, &["cpu", "coreml"], |_, _| {
                panic!("CPU selection must preserve default initialization")
            })
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn capabilities_match_compiled_target() {
        let providers = compiled_embedding_providers();
        assert_eq!(providers[0], "cpu");
        assert_eq!(
            providers.contains(&"coreml"),
            cfg!(all(feature = "gpu-embeddings", target_vendor = "apple"))
        );
        assert_eq!(
            providers.contains(&"cuda"),
            cfg!(all(
                feature = "gpu-embeddings",
                any(target_os = "linux", target_os = "windows")
            ))
        );
    }
}
