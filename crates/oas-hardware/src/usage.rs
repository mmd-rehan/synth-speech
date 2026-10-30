//! A fixed-capacity ring buffer of recent [`UsageSample`]s that backs the live
//! chart on the Machine screen.

use std::collections::VecDeque;

use crate::types::UsageSample;

/// Number of samples retained (120 = two minutes at 1 Hz).
pub const HISTORY_CAPACITY: usize = 120;

/// A bounded history of usage samples. Oldest samples are dropped once the
/// capacity is reached.
#[derive(Debug, Default)]
pub struct UsageHistory {
    samples: VecDeque<UsageSample>,
}

impl UsageHistory {
    /// Create an empty history.
    pub fn new() -> Self {
        UsageHistory {
            samples: VecDeque::with_capacity(HISTORY_CAPACITY),
        }
    }

    /// Append a sample, evicting the oldest if at capacity.
    pub fn push(&mut self, sample: UsageSample) {
        if self.samples.len() == HISTORY_CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Number of samples currently held.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether the history is empty.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The most recent sample, if any.
    pub fn latest(&self) -> Option<&UsageSample> {
        self.samples.back()
    }

    /// All samples, oldest first.
    pub fn as_vec(&self) -> Vec<UsageSample> {
        self.samples.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ThermalState;

    fn sample(cpu: f32, at: u64) -> UsageSample {
        UsageSample {
            cpu_total_pct: cpu,
            cpu_per_core: vec![cpu],
            ram_used_bytes: 0,
            ram_available_bytes: 0,
            swap_used_bytes: 0,
            gpu_util_pct: None,
            vram_used_bytes: None,
            app_rss_bytes: None,
            worker_rss_bytes: 0,
            disk_free_bytes: 0,
            thermal: ThermalState::Nominal,
            battery_pct: None,
            captured_at_ms: at,
        }
    }

    #[test]
    fn keeps_latest_and_reports_len() {
        let mut h = UsageHistory::new();
        assert!(h.is_empty());
        h.push(sample(10.0, 1));
        h.push(sample(20.0, 2));
        assert_eq!(h.len(), 2);
        assert_eq!(h.latest().unwrap().cpu_total_pct, 20.0);
    }

    #[test]
    fn evicts_oldest_beyond_capacity() {
        let mut h = UsageHistory::new();
        for i in 0..(HISTORY_CAPACITY as u64 + 10) {
            h.push(sample(i as f32, i));
        }
        assert_eq!(h.len(), HISTORY_CAPACITY);
        // The first 10 samples were evicted; the oldest retained is #10.
        assert_eq!(h.as_vec().first().unwrap().captured_at_ms, 10);
        assert_eq!(
            h.latest().unwrap().captured_at_ms,
            HISTORY_CAPACITY as u64 + 9
        );
    }
}
