//! Process-wide execution providers and opt-in shared CPU threads for embeddings.
//!
//! Configure this before constructing any fastembed model. With no environment
//! overrides, initialization remains lazy and existing behavior is unchanged.
//! `CODANNA_EMBED_CPU_THREADS` bounds the shared ORT intra-op pool, NOT the whole
//! process: callers, tokenizers, parsers and index writers have their own threads.

use crate::memory::MemoryBudget;
use fastembed::ExecutionProviderDispatch;
use std::str::FromStr;

const PROVIDER_ENV: &str = "CODANNA_EMBED_PROVIDER";
const STRICT_ENV: &str = "CODANNA_EMBED_PROVIDER_STRICT";
const THREADS_ENV: &str = "CODANNA_EMBED_CPU_THREADS";
const SPIN_ENV: &str = "CODANNA_EMBED_ALLOW_SPINNING";

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

/// These settings are execution-only; never include them in embedding identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuThreads {
    intra: usize,
    spinning: bool,
}

impl CpuThreads {
    fn parse(
        threads: Option<&str>,
        spinning: Option<&str>,
        logical_cpus: usize,
    ) -> Result<Option<Self>, String> {
        if threads.is_none() && spinning.is_none() {
            return Ok(None);
        }
        let intra = match threads {
            Some(raw) => raw
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|value| (1..=1024).contains(value))
                .ok_or_else(|| format!("{THREADS_ENV} must be an integer between 1 and 1024"))?,
            None => logical_cpus.saturating_sub(2).clamp(1, 4),
        };
        let spinning = match spinning
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("0" | "false" | "no") => false,
            Some("1" | "true" | "yes") => true,
            _ => return Err(format!("{SPIN_ENV} must be 0/1, false/true, or no/yes")),
        };
        Ok(Some(Self { intra, spinning }))
    }
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} must contain UTF-8 text")),
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
) -> Option<ExecutionProviderDispatch> {
    match provider {
        EmbeddingExecutionProvider::CoreMl => coreml_dispatch(strict),
        EmbeddingExecutionProvider::Cuda => cuda_dispatch(strict),
        // Auto is resolved against compiled capabilities before registration.
        EmbeddingExecutionProvider::Cpu | EmbeddingExecutionProvider::Auto => None,
    }
}

/// Compiled capabilities, not proof of accelerator registration or device use.
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

/// Resolve both concerns before the single environment commit. In particular,
/// configuring CPU threads must not initialize ORT ahead of CoreML/CUDA selection.
fn configure_runtime(
    raw: &str,
    strict: bool,
    available: &[&str],
    cpu: Option<CpuThreads>,
    commit: impl FnOnce(
        Option<EmbeddingExecutionProvider>,
        bool,
        Option<CpuThreads>,
    ) -> Result<bool, String>,
) -> Result<Vec<String>, String> {
    let mut messages = Vec::new();
    let requested = EmbeddingExecutionProvider::from_str(raw);
    let selected = match requested {
        Ok(EmbeddingExecutionProvider::Cpu) => None,
        Ok(provider) => {
            let name = match provider {
                EmbeddingExecutionProvider::Auto => {
                    available.iter().copied().find(|name| *name != "cpu")
                }
                EmbeddingExecutionProvider::CoreMl => Some("coreml"),
                EmbeddingExecutionProvider::Cuda => Some("cuda"),
                EmbeddingExecutionProvider::Cpu => None,
            };
            if let Some(name) = name.filter(|name| available.contains(name)) {
                Some(EmbeddingExecutionProvider::from_str(name)?)
            } else {
                let error = format!(
                    "embedding provider '{raw}' is not compiled for this target (compiled: {}); Apple builds require --features gpu-coreml",
                    available.join(", ")
                );
                if strict {
                    return Err(error);
                }
                messages.push(error);
                None
            }
        }
        Err(error) => {
            if strict {
                return Err(error);
            }
            messages.push(error);
            None
        }
    };
    if selected.is_none() && cpu.is_none() {
        if let Some(message) = messages.last_mut() {
            message.push_str("; leaving the existing runtime configuration unchanged");
        }
        return Ok(messages);
    }
    match commit(selected, strict, cpu) {
        Ok(true) => {
            if let Some(provider) = selected {
                messages.push(format!(
                    "local embedding provider {provider:?} configured{}; session registration and actual device execution are not yet verified",
                    if strict { " (strict registration)" } else { " with CPU fallback" }
                ));
            }
            if let Some(cpu) = cpu {
                messages.push(format!(
                    "shared embedding CPU pool configured: intra_threads={}, inter_threads=1, spinning={}; this is not a whole-process CPU cap",
                    cpu.intra, cpu.spinning
                ));
            }
        }
        outcome => {
            let error = match outcome {
                Ok(false) => {
                    "ONNX Runtime was already initialized before embedding runtime selection"
                        .to_string()
                }
                Err(error) => format!("failed to configure embedding runtime: {error}"),
                Ok(true) => unreachable!(),
            };
            // Explicit resource limits must never silently fall back to unbounded
            // per-session pools, even when accelerator registration is optional.
            if strict || cpu.is_some() {
                return Err(error);
            }
            messages.push(format!(
                "{error}; leaving the existing runtime configuration unchanged"
            ));
        }
    }
    Ok(messages)
}

fn commit_runtime(
    provider: Option<EmbeddingExecutionProvider>,
    strict: bool,
    cpu: Option<CpuThreads>,
) -> Result<bool, String> {
    let mut builder = ort::init();
    if let Some(provider) = provider {
        let dispatch = provider_dispatch(provider, strict)
            .ok_or_else(|| "compiled provider dispatch is unavailable".to_string())?;
        builder = builder.with_execution_providers([dispatch]);
    }
    if let Some(cpu) = cpu {
        // ort rc.10 disables per-session threads during session commit whenever
        // the environment has a global pool. This also overrides fastembed 5.6's
        // explicit per-session with_intra_threads(available_parallelism) setting.
        let threads = ort::environment::GlobalThreadPoolOptions::default()
            .with_intra_threads(cpu.intra)
            .and_then(|options| options.with_inter_threads(1))
            .and_then(|options| options.with_spin_control(cpu.spinning))
            .map_err(|error| error.to_string())?;
        builder = builder.with_global_thread_pool(threads);
    }
    builder.commit().map_err(|error| error.to_string())
}

fn optional_auto_needs_cpu(raw: &str, strict: bool, memory: MemoryBudget) -> bool {
    !strict
        && EmbeddingExecutionProvider::from_str(raw) == Ok(EmbeddingExecutionProvider::Auto)
        && memory.headroom < 4 * 1024 * 1024 * 1024
}

/// Configure providers and optional shared CPU workers before creating models.
///
/// Unset CPU controls preserve the original lazy/default threading behavior.
/// Setting either CPU control opts in; spinning then defaults to disabled.
/// Explicit resource-control failures are always returned, never silently ignored.
/// Accelerators can still execute unsupported graph portions on the CPU.
pub fn configure_embedding_runtime() -> Result<(), String> {
    let threads = optional_env(THREADS_ENV)?;
    let spinning = optional_env(SPIN_ENV)?;
    let cpu = CpuThreads::parse(threads.as_deref(), spinning.as_deref(), num_cpus::get())?;
    let provider = optional_env(PROVIDER_ENV)?.unwrap_or_else(|| "cpu".into());
    let strict = strict_provider_registration();
    let provider = if !strict
        && EmbeddingExecutionProvider::from_str(&provider) == Ok(EmbeddingExecutionProvider::Auto)
        && optional_auto_needs_cpu(&provider, strict, MemoryBudget::current())
    {
        eprintln!(
            "codanna: optional auto embedding selection is using CPU because memory headroom is below 4 GiB"
        );
        "cpu"
    } else {
        &provider
    };
    for message in configure_runtime(
        provider,
        strict,
        &compiled_embedding_providers(),
        cpu,
        commit_runtime,
    )? {
        eprintln!("codanna: {message}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_auto_memory_fallback_preserves_explicit_controls() {
        let low = MemoryBudget::from_values(8 << 30, 2 << 30, 0);
        let ample = MemoryBudget::from_values(32 << 30, 24 << 30, 0);
        assert!(optional_auto_needs_cpu(" AUTO ", false, low));
        assert!(!optional_auto_needs_cpu("auto", true, low));
        assert!(!optional_auto_needs_cpu("auto", false, ample));
        for provider in ["cpu", "coreml", "cuda", "invalid"] {
            assert!(!optional_auto_needs_cpu(provider, false, low));
        }
        let selected = if optional_auto_needs_cpu("auto", false, low) {
            "cpu"
        } else {
            "auto"
        };
        configure_runtime(
            selected,
            false,
            &["cpu", "coreml"],
            Some(quiet()),
            |provider, strict, cpu| {
                assert_eq!(provider, None);
                assert!(!strict);
                assert_eq!(cpu, Some(quiet()));
                Ok(true)
            },
        )
        .unwrap();
    }

    fn quiet() -> CpuThreads {
        CpuThreads {
            intra: 2,
            spinning: false,
        }
    }

    #[test]
    fn cpu_thread_controls_are_opt_in_and_checked() {
        assert_eq!(CpuThreads::parse(None, None, 8).unwrap(), None);
        assert_eq!(
            CpuThreads::parse(Some("2"), None, 8).unwrap(),
            Some(quiet())
        );
        assert_eq!(
            CpuThreads::parse(None, Some("false"), 1)
                .unwrap()
                .unwrap()
                .intra,
            1
        );
        assert_eq!(
            CpuThreads::parse(None, Some("0"), 8)
                .unwrap()
                .unwrap()
                .intra,
            4
        );
        assert!(
            CpuThreads::parse(Some("1"), Some("TRUE"), 8)
                .unwrap()
                .unwrap()
                .spinning
        );
        for bad in ["", "0", "-1", "1025", "two", "18446744073709551616"] {
            assert!(CpuThreads::parse(Some(bad), None, 8).is_err(), "{bad}");
        }
        assert!(CpuThreads::parse(None, Some("sometimes"), 8).is_err());
    }

    #[test]
    fn parses_execution_provider_names() {
        for (raw, expected) in [
            ("cpu", EmbeddingExecutionProvider::Cpu),
            ("AUTO", EmbeddingExecutionProvider::Auto),
            ("core-ml", EmbeddingExecutionProvider::CoreMl),
            ("cuda", EmbeddingExecutionProvider::Cuda),
        ] {
            assert_eq!(EmbeddingExecutionProvider::from_str(raw).unwrap(), expected);
        }
        assert!(EmbeddingExecutionProvider::from_str("metal").is_err());
    }

    #[test]
    fn cpu_without_thread_controls_does_not_initialize_runtime() {
        for raw in ["", "cpu"] {
            assert!(
                configure_runtime(raw, true, &["cpu"], None, |_, _, _| panic!(
                    "must remain lazy"
                ))
                .unwrap()
                .is_empty()
            );
        }
    }

    #[test]
    fn cpu_controls_commit_without_a_gpu_feature() {
        let messages =
            configure_runtime("cpu", false, &["cpu"], Some(quiet()), |provider, _, cpu| {
                assert_eq!(provider, None);
                assert_eq!(cpu, Some(quiet()));
                Ok(true)
            })
            .unwrap();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("intra_threads=2"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("not a whole-process CPU cap"))
        );
    }

    #[test]
    fn accelerator_and_threads_share_one_commit() {
        for (name, expected) in [
            ("coreml", EmbeddingExecutionProvider::CoreMl),
            ("cuda", EmbeddingExecutionProvider::Cuda),
        ] {
            let messages = configure_runtime(
                "auto",
                true,
                &["cpu", name],
                Some(quiet()),
                |provider, strict, cpu| {
                    assert_eq!(provider, Some(expected));
                    assert!(strict);
                    assert_eq!(cpu, Some(quiet()));
                    Ok(true)
                },
            )
            .unwrap();
            assert!(
                messages
                    .iter()
                    .any(|message| message.contains("not yet verified"))
            );
        }
    }

    #[test]
    fn unavailable_strict_provider_fails_before_any_initialization() {
        for requested in ["coreml", "cuda", "auto", "metal"] {
            assert!(
                configure_runtime(requested, true, &["cpu"], Some(quiet()), |_, _, _| panic!(
                    "must not initialize"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn optional_unavailable_provider_preserves_requested_cpu_controls() {
        let messages = configure_runtime(
            "coreml",
            false,
            &["cpu"],
            Some(quiet()),
            |provider, _, cpu| {
                assert_eq!(provider, None);
                assert_eq!(cpu, Some(quiet()));
                Ok(true)
            },
        )
        .unwrap();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("not compiled"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("shared embedding CPU pool configured"))
        );
    }

    #[test]
    fn explicit_resource_limits_never_silently_fail() {
        for outcome in [Ok(false), Err("fixture failure".into())] {
            assert!(
                configure_runtime("cpu", false, &["cpu"], Some(quiet()), |_, _, _| outcome)
                    .is_err()
            );
        }
    }

    #[test]
    fn optional_provider_failures_do_not_claim_configuration_success() {
        for outcome in [Ok(false), Err("fixture failure".into())] {
            let messages =
                configure_runtime("coreml", false, &["cpu", "coreml"], None, |_, _, _| outcome)
                    .unwrap();
            assert!(
                messages
                    .iter()
                    .any(|message| message.contains("configuration unchanged"))
            );
            assert!(
                !messages
                    .iter()
                    .any(|message| message.contains("configured with CPU fallback"))
            );
        }
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
