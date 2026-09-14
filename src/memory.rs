//! Adaptive memory budgeting for indexing and local embedding inference.
//!
//! The budget is deliberately based on *available* memory as well as physical
//! memory. This lets a quiet workstation use more RAM while leaving a busy,
//! smaller machine enough headroom to avoid macOS/Linux swap storms.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const MIN_SYSTEM_RESERVE: u64 = 1536 * MIB;
const MIN_WORKING_HEADROOM: u64 = 256 * MIB;
const MAX_TOTAL_FRACTION_PERCENT: u64 = 25;

/// Conservative estimates include native-runtime arenas and temporary tensors,
/// not just the model weights reported by fastembed.
const CPU_SESSION_ESTIMATE: u64 = 384 * MIB;
const ACCELERATED_SESSION_ESTIMATE: u64 = 1024 * MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBudget {
    pub total: u64,
    pub available: u64,
    pub process_rss: u64,
    /// Additional memory Codanna may consume before reaching its soft target.
    pub headroom: u64,
}

/// Reusable host/process sampler for loops that need fresh memory readings at
/// batch boundaries. Keeping the `System` allocation avoids rebuilding the
/// process table for every candidate added to a batch.
pub struct MemorySampler {
    system: System,
    pid: Pid,
}

impl MemorySampler {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            pid: Pid::from_u32(std::process::id()),
        }
    }

    pub fn sample(&mut self) -> MemoryBudget {
        self.system.refresh_memory();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        let process_rss = self
            .system
            .process(self.pid)
            .map_or(0, |process| process.memory());
        MemoryBudget::from_values(
            self.system.total_memory(),
            self.system.available_memory(),
            process_rss,
        )
    }
}

impl Default for MemorySampler {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBudget {
    /// Snapshot the current host and Codanna process.
    pub fn current() -> Self {
        MemorySampler::new().sample()
    }

    /// Deterministic constructor used by tests and by callers with a fresher
    /// platform-specific memory-pressure sample.
    pub fn from_values(total: u64, available: u64, process_rss: u64) -> Self {
        let reserve = MIN_SYSTEM_RESERVE.max(total.saturating_mul(15) / 100);
        let available_after_reserve = available.saturating_sub(reserve);
        let total_cap = total.saturating_mul(MAX_TOTAL_FRACTION_PERCENT) / 100;
        let headroom = available_after_reserve.min(total_cap);
        Self {
            total,
            available,
            process_rss,
            headroom,
        }
    }

    /// Limit eagerly-created embedding sessions. Accelerated runtimes receive a
    /// larger allowance because CoreML/CUDA allocations substantially exceed
    /// the model file size. At least one session is retained so low-memory mode
    /// remains functional instead of silently disabling semantic indexing.
    pub fn embedding_instances(self, requested: usize, accelerated: bool) -> usize {
        let per_session = if accelerated {
            ACCELERATED_SESSION_ESTIMATE
        } else {
            CPU_SESSION_ESTIMATE
        };
        let affordable = (self.headroom / per_session) as usize;
        requested.max(1).min(affordable.max(1))
    }

    /// Scale byte-heavy pipeline queues down when the host has little headroom.
    /// The returned percentage is kept above 10% to preserve useful pipelining.
    pub fn queue_scale_percent(self) -> usize {
        match self.headroom {
            0..=MIN_WORKING_HEADROOM => 10,
            h if h <= 512 * MIB => 25,
            h if h <= GIB => 50,
            _ => 100,
        }
    }

    /// Select an inference batch size from current headroom. Accelerated
    /// providers use smaller batches because native activation buffers can be
    /// much larger than the returned embeddings.
    pub fn embedding_batch_size(self, configured_max: usize, accelerated: bool) -> usize {
        let ceiling = if accelerated { 32 } else { 64 };
        let adaptive = match self.headroom {
            0..=MIN_WORKING_HEADROOM => 4,
            h if h <= 512 * MIB => 8,
            h if h <= GIB => 16,
            h if h <= 2 * GIB => 32,
            _ => ceiling,
        };
        configured_max.max(1).min(adaptive.min(ceiling))
    }

    /// True when starting another native inference batch would consume the
    /// system reserve. Callers should checkpoint or reduce work at this point.
    pub fn under_pressure(self) -> bool {
        self.available <= MIN_SYSTEM_RESERVE || self.headroom == 0
    }
}

/// Whether the selected local embedding provider uses an accelerator runtime.
/// This also works on builds where GPU support is compiled out: the environment
/// still signals that a larger safety allowance is appropriate.
pub fn accelerated_embeddings_requested() -> bool {
    std::env::var("CODANNA_EMBED_PROVIDER")
        .ok()
        .is_some_and(|provider| {
            matches!(
                provider.trim().to_ascii_lowercase().as_str(),
                "auto" | "coreml" | "core-ml" | "cuda"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_eight_gib_host_uses_one_embedding_session() {
        let budget = MemoryBudget::from_values(8 * GIB, 2 * GIB, 100 * MIB);
        assert_eq!(budget.embedding_instances(3, false), 1);
        assert_eq!(budget.embedding_instances(3, true), 1);
        assert_eq!(budget.queue_scale_percent(), 25);
        assert_eq!(budget.embedding_batch_size(64, true), 8);
    }

    #[test]
    fn idle_thirty_two_gib_host_can_use_requested_parallelism() {
        let budget = MemoryBudget::from_values(32 * GIB, 24 * GIB, 100 * MIB);
        assert_eq!(budget.embedding_instances(3, false), 3);
        assert_eq!(budget.embedding_instances(3, true), 3);
        assert_eq!(budget.queue_scale_percent(), 100);
        assert_eq!(budget.embedding_batch_size(64, false), 64);
    }

    #[test]
    fn pressure_never_disables_the_only_embedding_session() {
        let budget = MemoryBudget::from_values(8 * GIB, GIB, 200 * MIB);
        assert!(budget.under_pressure());
        assert_eq!(budget.embedding_instances(8, true), 1);
        assert_eq!(budget.queue_scale_percent(), 10);
    }
}
