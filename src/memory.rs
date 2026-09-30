//! Adaptive memory budgeting for indexing and local embedding inference.
//!
//! Budgets use currently available memory as well as physical memory, allowing
//! a quiet workstation to use more RAM while protecting a busy smaller host
//! from an avoidable swap storm.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const MIN_SYSTEM_RESERVE: u64 = 1536 * MIB;
const MIN_WORKING_HEADROOM: u64 = 256 * MIB;
const MAX_TOTAL_FRACTION_PERCENT: u64 = 25;
const CPU_SESSION_ESTIMATE: u64 = 384 * MIB;
const ACCELERATED_SESSION_ESTIMATE: u64 = 1024 * MIB;
// Native accelerator footprints can substantially exceed model weights. A
// four-GiB gate means `auto` never selects CoreML on an 8-GiB host (the total
// budget itself is capped at 25%), while quiet 16/32-GiB machines can still use
// it. Explicit `coreml`/`cuda` selections remain operator-controlled.
const ACCELERATOR_MIN_HEADROOM: u64 = 4 * GIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBudget {
    pub total: u64,
    pub available: u64,
    pub process_rss: u64,
    /// Additional memory Codanna may consume before reaching its soft target.
    pub headroom: u64,
}

impl MemoryBudget {
    pub fn current() -> Self {
        let mut system = System::new();
        system.refresh_memory();
        let pid = Pid::from_u32(std::process::id());
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        let process_rss = system.process(pid).map_or(0, |process| process.memory());
        Self::from_values(
            system.total_memory(),
            system.available_memory(),
            process_rss,
        )
    }

    pub fn from_values(total: u64, available: u64, process_rss: u64) -> Self {
        let reserve = MIN_SYSTEM_RESERVE.max(total.saturating_mul(15) / 100);
        let available_after_reserve = available.saturating_sub(reserve);
        let total_cap = total.saturating_mul(MAX_TOTAL_FRACTION_PERCENT) / 100;
        Self {
            total,
            available,
            process_rss,
            headroom: available_after_reserve.min(total_cap),
        }
    }

    pub fn embedding_instances(self, requested: usize, accelerated: bool) -> usize {
        let per_session = if accelerated {
            ACCELERATED_SESSION_ESTIMATE
        } else {
            CPU_SESSION_ESTIMATE
        };
        let affordable = (self.headroom / per_session) as usize;
        requested.max(1).min(affordable.max(1))
    }

    pub fn queue_scale_percent(self) -> usize {
        match self.headroom {
            0..=MIN_WORKING_HEADROOM => 10,
            h if h <= 512 * MIB => 25,
            h if h <= GIB => 50,
            _ => 100,
        }
    }

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

    pub fn under_pressure(self) -> bool {
        self.available <= MIN_SYSTEM_RESERVE || self.headroom == 0
    }

    /// Auto-selected CoreML/CUDA is worthwhile only when the host has enough
    /// free headroom for native weights, activation arenas, and transient copies.
    pub fn accelerator_suitable(self) -> bool {
        self.headroom >= ACCELERATOR_MIN_HEADROOM
    }
}

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
    fn busy_eight_gib_host_uses_constrained_settings() {
        let budget = MemoryBudget::from_values(8 * GIB, 2 * GIB, 100 * MIB);
        assert_eq!(budget.embedding_instances(3, false), 1);
        assert_eq!(budget.embedding_instances(3, true), 1);
        assert_eq!(budget.queue_scale_percent(), 25);
        assert_eq!(budget.embedding_batch_size(64, true), 8);
        assert!(!budget.accelerator_suitable());
    }

    #[test]
    fn idle_thirty_two_gib_host_uses_requested_parallelism() {
        let budget = MemoryBudget::from_values(32 * GIB, 24 * GIB, 100 * MIB);
        assert_eq!(budget.embedding_instances(3, false), 3);
        assert_eq!(budget.embedding_instances(3, true), 3);
        assert_eq!(budget.queue_scale_percent(), 100);
        assert_eq!(budget.embedding_batch_size(64, false), 64);
        assert!(budget.accelerator_suitable());
    }

    #[test]
    fn pressure_never_disables_cpu_indexing() {
        let budget = MemoryBudget::from_values(8 * GIB, GIB, 200 * MIB);
        assert!(budget.under_pressure());
        assert_eq!(budget.embedding_instances(8, false), 1);
        assert_eq!(budget.queue_scale_percent(), 10);
    }
}
