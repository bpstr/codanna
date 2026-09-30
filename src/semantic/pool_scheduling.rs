//! Indexed slice scheduling and bounded automatic usage logging.
//!
//! The caller installs this work on its existing dedicated embedding pool.
//! Never move model checkout or nested tokenizer work to the global Rayon pool.

use rayon::prelude::*;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const USAGE_LOG_INTERVAL: Duration = Duration::from_secs(10);

pub(super) fn map_batches<T, R, E>(
    items: &[T],
    batch_size: usize,
    map: impl Fn(&[T]) -> Result<R, E> + Sync + Send,
) -> Result<Vec<R>, E>
where
    T: Sync,
    R: Send,
    E: Send,
{
    items.par_chunks(batch_size.max(1)).map(map).collect()
}

/// Automatic reports are best-effort; contenders skip instead of waiting.
/// Explicit progress/final reports do not use this gate.
pub(super) struct UsageLogGate {
    last_report: Mutex<Instant>,
}

impl UsageLogGate {
    pub(super) fn new() -> Self {
        Self::new_at(Instant::now())
    }

    fn new_at(now: Instant) -> Self {
        Self {
            last_report: Mutex::new(now),
        }
    }

    pub(super) fn should_log(&self) -> bool {
        self.should_log_at(Instant::now())
    }

    fn should_log_at(&self, now: Instant) -> bool {
        let Ok(mut last) = self.last_report.try_lock() else {
            return false;
        };
        if now.saturating_duration_since(*last) < USAGE_LOG_INTERVAL {
            return false;
        }
        *last = now;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_batches_keep_order_on_the_dedicated_pool_with_nested_work() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        let inputs: Vec<usize> = (0..19).collect();
        let output = pool
            .install(|| {
                map_batches(&inputs, 4, |chunk| {
                    assert_eq!(rayon::current_num_threads(), 2);
                    assert!(rayon::current_thread_index().is_some());
                    Ok::<_, ()>(chunk.par_iter().map(|value| value * 2).collect::<Vec<_>>())
                })
            })
            .unwrap();
        assert_eq!(
            output.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![4, 4, 4, 4, 3]
        );
        assert_eq!(
            output.into_iter().flatten().collect::<Vec<_>>(),
            inputs.iter().map(|value| value * 2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_single_and_zero_batch_inputs_are_bounded() {
        let empty: Vec<u8> = Vec::new();
        assert!(
            map_batches(&empty, 1, |chunk| Ok::<_, ()>(chunk.len()))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            map_batches(&[7u8], 64, |chunk| Ok::<_, ()>(chunk.to_vec())).unwrap(),
            vec![vec![7]]
        );
        assert_eq!(
            map_batches(&[1u8, 2], 0, |chunk| Ok::<_, ()>(chunk.to_vec())).unwrap(),
            vec![vec![1], vec![2]]
        );
    }

    #[test]
    fn batch_errors_propagate_without_a_model() {
        let result = map_batches(&[0u8, 1, 2, 3], 2, |chunk| {
            if chunk.contains(&2) {
                Err("fixture failure")
            } else {
                Ok(chunk.len())
            }
        });
        assert_eq!(result, Err("fixture failure"));
    }

    #[test]
    fn usage_logging_is_time_bounded_without_sleeping() {
        let start = Instant::now();
        let gate = UsageLogGate::new_at(start);
        assert!(!gate.should_log_at(start));
        assert!(!gate.should_log_at(start + Duration::from_secs(9)));
        assert!(gate.should_log_at(start + USAGE_LOG_INTERVAL));
        assert!(!gate.should_log_at(start + USAGE_LOG_INTERVAL));
        assert!(!gate.should_log_at(start));
        assert!(gate.should_log_at(start + USAGE_LOG_INTERVAL * 2));
    }

    #[test]
    fn usage_logging_never_waits_for_another_reporter() {
        let start = Instant::now();
        let gate = UsageLogGate::new_at(start);
        let guard = gate.last_report.lock().unwrap();
        assert!(!gate.should_log_at(start + USAGE_LOG_INTERVAL));
        drop(guard);
        assert!(gate.should_log_at(start + USAGE_LOG_INTERVAL));
    }
}
