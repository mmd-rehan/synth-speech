//! The resource governor: decides how many jobs may run, reserves resources on
//! admission, and releases them on any exit path (no leaks). All logic here is
//! pure given its inputs, so it is exhaustively testable.

use oas_catalog::ModelEntry;
use oas_hardware::ThermalState;

const GIB: u64 = 1024 * 1024 * 1024;

/// Resource budgets for the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budgets {
    /// Memory the app may plan to use (pool minus safety reserve).
    pub mem_budget: u64,
    /// CPU job slots (cores minus one for UI/OS, min 1).
    pub cpu_budget: u32,
    /// Concurrent GPU jobs allowed.
    pub gpu_slots: u32,
}

impl Budgets {
    /// Build budgets from a memory pool, total RAM, cores, and GPU count.
    pub fn new(pool_bytes: u64, total_bytes: u64, logical_cores: u32, gpu_count: u32) -> Self {
        let reserve = (3 * GIB / 2).max((total_bytes as f64 * 0.15) as u64);
        Budgets {
            mem_budget: pool_bytes.saturating_sub(reserve),
            cpu_budget: logical_cores.saturating_sub(1).max(1),
            gpu_slots: gpu_count, // 1 per GPU
        }
    }
}

/// The cost of running one job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobCost {
    pub mem_bytes: u64,
    pub gpu: bool,
}

impl JobCost {
    /// Memory cost from a catalog model (weights + overhead + audio working set).
    pub fn for_model(model: &ModelEntry, gpu: bool) -> Self {
        JobCost {
            mem_bytes: model
                .weights_bytes
                .saturating_add(model.runtime_overhead_bytes)
                .saturating_add(model.per_minute_audio_bytes),
            gpu,
        }
    }
}

/// What the governor granted a job. Returned to the job and surrendered on exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reservation {
    pub mem_bytes: u64,
    pub cpu_slots: u32,
    pub gpu_slot: bool,
    /// Threads to hand the worker (informational; derived from concurrency).
    pub threads: u32,
}

/// Why a job could not start right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    Memory,
    Cpu,
    NoGpuSlot,
    Thermal,
    Battery,
}

impl DenyReason {
    /// A short user-facing explanation ("Waiting: ...").
    pub fn message(self) -> &'static str {
        match self {
            DenyReason::Memory => "Waiting: not enough free memory",
            DenyReason::Cpu => "Waiting: all CPU slots are busy",
            DenyReason::NoGpuSlot => "Waiting: the GPU is busy",
            DenyReason::Thermal => "Waiting: the machine is too hot",
            DenyReason::Battery => "Waiting: paused on battery",
        }
    }
}

/// Tracks live reservations against the budgets.
#[derive(Debug, Clone)]
pub struct Governor {
    budgets: Budgets,
    reserved_mem: u64,
    cpu_slots_used: u32,
    gpu_slots_used: u32,
}

impl Governor {
    pub fn new(budgets: Budgets) -> Self {
        Governor {
            budgets,
            reserved_mem: 0,
            cpu_slots_used: 0,
            gpu_slots_used: 0,
        }
    }

    pub fn budgets(&self) -> Budgets {
        self.budgets
    }

    pub fn reserved_mem(&self) -> u64 {
        self.reserved_mem
    }

    /// Threads to give a CPU job, balancing the CPU budget across jobs and
    /// capping at 8 (whisper CPU scaling flattens past ~8).
    fn threads_for(&self, concurrent_cpu_jobs: u32) -> u32 {
        let n = concurrent_cpu_jobs.max(1);
        (self.budgets.cpu_budget / n).clamp(1, 8)
    }

    /// Try to admit a job, reserving resources on success.
    pub fn admit(
        &mut self,
        cost: JobCost,
        thermal: ThermalState,
        battery_blocked: bool,
    ) -> Result<Reservation, DenyReason> {
        if matches!(thermal, ThermalState::Serious | ThermalState::Critical) {
            return Err(DenyReason::Thermal);
        }
        if battery_blocked {
            return Err(DenyReason::Battery);
        }
        if self.reserved_mem.saturating_add(cost.mem_bytes) > self.budgets.mem_budget {
            return Err(DenyReason::Memory);
        }

        let reservation = if cost.gpu {
            if self.gpu_slots_used >= self.budgets.gpu_slots {
                return Err(DenyReason::NoGpuSlot);
            }
            self.gpu_slots_used += 1;
            Reservation {
                mem_bytes: cost.mem_bytes,
                cpu_slots: 0,
                gpu_slot: true,
                threads: self.budgets.cpu_budget.clamp(1, 8),
            }
        } else {
            if self.cpu_slots_used >= self.budgets.cpu_budget {
                return Err(DenyReason::Cpu);
            }
            let threads = self.threads_for(self.cpu_slots_used + 1);
            self.cpu_slots_used += 1;
            Reservation {
                mem_bytes: cost.mem_bytes,
                cpu_slots: 1,
                gpu_slot: false,
                threads,
            }
        };
        self.reserved_mem += cost.mem_bytes;
        Ok(reservation)
    }

    /// Release a reservation (idempotent-safe against underflow).
    pub fn release(&mut self, r: &Reservation) {
        self.reserved_mem = self.reserved_mem.saturating_sub(r.mem_bytes);
        self.cpu_slots_used = self.cpu_slots_used.saturating_sub(r.cpu_slots);
        if r.gpu_slot {
            self.gpu_slots_used = self.gpu_slots_used.saturating_sub(1);
        }
    }

    /// Maximum CPU jobs of this per-job memory cost that fit from empty.
    pub fn max_concurrency(&self, per_job_mem: u64) -> u32 {
        let mem_limited = self
            .budgets
            .mem_budget
            .checked_div(per_job_mem)
            .map(|v| v as u32)
            .unwrap_or(self.budgets.cpu_budget);
        mem_limited.min(self.budgets.cpu_budget)
    }
}

/// Gauge level for the "Parallel jobs" control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaugeLevel {
    Safe,
    Tight,
    TooMany,
}

/// Gauge result: level + the maximum safe concurrency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gauge {
    pub level: GaugeLevel,
    pub max_safe: u32,
}

/// Evaluate running `desired` jobs of `per_job_mem` each against `budgets`.
pub fn gauge(desired: u32, budgets: Budgets, per_job_mem: u64) -> Gauge {
    let gov = Governor::new(budgets);
    let max_safe = gov.max_concurrency(per_job_mem);
    let level = if desired > max_safe {
        GaugeLevel::TooMany
    } else {
        // Memory headroom after running `desired` jobs.
        let used = per_job_mem.saturating_mul(desired as u64);
        let headroom = budgets.mem_budget.saturating_sub(used);
        let frac = if budgets.mem_budget > 0 {
            headroom as f64 / budgets.mem_budget as f64
        } else {
            0.0
        };
        if frac >= 0.30 {
            GaugeLevel::Safe
        } else {
            GaugeLevel::Tight
        }
    };
    Gauge { level, max_safe }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn budgets(mem_gb: u64, cores: u32, gpus: u32) -> Budgets {
        // pool == total for the test; reserve is applied inside.
        Budgets::new(mem_gb * GIB, mem_gb * GIB, cores, gpus)
    }

    #[test]
    fn budgets_apply_reserve_and_core_reserve() {
        let b = budgets(16, 8, 1);
        // reserve = max(1.5, 15% of 16) = 2.4 GB → budget ~13.6 GB.
        assert!(b.mem_budget < 16 * GIB && b.mem_budget > 13 * GIB);
        assert_eq!(b.cpu_budget, 7);
        assert_eq!(b.gpu_slots, 1);
    }

    #[test]
    fn admits_until_memory_runs_out() {
        let mut g = Governor::new(budgets(16, 8, 0));
        let cost = JobCost {
            mem_bytes: 4 * GIB,
            gpu: false,
        };
        // ~13.6 GB budget / 4 GB ≈ 3 jobs.
        let mut admitted = 0;
        for _ in 0..10 {
            if g.admit(cost, ThermalState::Nominal, false).is_ok() {
                admitted += 1;
            }
        }
        assert_eq!(admitted, 3);
    }

    #[test]
    fn cpu_budget_caps_job_count() {
        let mut g = Governor::new(budgets(256, 4, 0)); // huge RAM, 3 cpu slots
        let cost = JobCost {
            mem_bytes: GIB,
            gpu: false,
        };
        let mut admitted = 0;
        for _ in 0..10 {
            if g.admit(cost, ThermalState::Nominal, false).is_ok() {
                admitted += 1;
            }
        }
        assert_eq!(admitted, 3); // cpu_budget = 4-1 = 3
    }

    #[test]
    fn thermal_and_battery_block_admission() {
        let mut g = Governor::new(budgets(16, 8, 0));
        let cost = JobCost {
            mem_bytes: GIB,
            gpu: false,
        };
        assert_eq!(
            g.admit(cost, ThermalState::Serious, false),
            Err(DenyReason::Thermal)
        );
        assert_eq!(
            g.admit(cost, ThermalState::Nominal, true),
            Err(DenyReason::Battery)
        );
    }

    #[test]
    fn gpu_slots_limit_gpu_jobs() {
        let mut g = Governor::new(budgets(32, 16, 1));
        let gpu = JobCost {
            mem_bytes: 2 * GIB,
            gpu: true,
        };
        assert!(g.admit(gpu, ThermalState::Nominal, false).is_ok());
        assert_eq!(
            g.admit(gpu, ThermalState::Nominal, false),
            Err(DenyReason::NoGpuSlot)
        );
    }

    #[test]
    fn release_frees_resources_no_leak() {
        let mut g = Governor::new(budgets(16, 8, 0));
        let cost = JobCost {
            mem_bytes: 4 * GIB,
            gpu: false,
        };
        let r1 = g.admit(cost, ThermalState::Nominal, false).unwrap();
        let _r2 = g.admit(cost, ThermalState::Nominal, false).unwrap();
        g.admit(cost, ThermalState::Nominal, false).unwrap();
        assert!(g.admit(cost, ThermalState::Nominal, false).is_err());
        g.release(&r1);
        // After releasing one, another fits again.
        assert!(g.admit(cost, ThermalState::Nominal, false).is_ok());
    }

    #[test]
    fn threads_are_balanced_and_capped() {
        let mut g = Governor::new(budgets(64, 32, 0)); // cpu_budget 31
        let cost = JobCost {
            mem_bytes: GIB,
            gpu: false,
        };
        let r1 = g.admit(cost, ThermalState::Nominal, false).unwrap();
        assert_eq!(r1.threads, 8); // first job capped at 8, not 31
    }

    #[test]
    fn gauge_levels() {
        let b = budgets(16, 8, 0); // ~13.6 GB budget
        assert_eq!(gauge(1, b, GIB).level, GaugeLevel::Safe);
        // near the limit → Tight
        let g = gauge(13, b, GIB);
        assert!(matches!(g.level, GaugeLevel::Tight | GaugeLevel::TooMany));
        // way over → TooMany
        assert_eq!(gauge(100, b, GIB).level, GaugeLevel::TooMany);
    }

    proptest! {
        // More memory budget never reduces the max safe concurrency.
        #[test]
        fn more_memory_never_reduces_concurrency(
            lo in 2u64..32, extra in 0u64..64, per in 1u64..4
        ) {
            let g_lo = Governor::new(budgets(lo, 16, 0));
            let g_hi = Governor::new(budgets(lo + extra, 16, 0));
            let m_lo = g_lo.max_concurrency(per * GIB);
            let m_hi = g_hi.max_concurrency(per * GIB);
            prop_assert!(m_hi >= m_lo, "more RAM lowered concurrency: {m_lo} -> {m_hi}");
        }

        // Reservations never exceed the memory budget.
        #[test]
        fn never_overcommits_memory(per in 1u64..6, attempts in 0u32..30) {
            let b = budgets(16, 32, 0);
            let mut g = Governor::new(b);
            for _ in 0..attempts {
                let _ = g.admit(JobCost { mem_bytes: per * GIB, gpu: false }, ThermalState::Nominal, false);
            }
            prop_assert!(g.reserved_mem() <= b.mem_budget);
        }
    }
}
