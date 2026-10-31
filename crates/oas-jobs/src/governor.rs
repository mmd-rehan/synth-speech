//! The resource governor: decides how many jobs may run, reserves resources on
//! admission, and releases them on any exit path (no leaks). All logic here is
//! pure given its inputs, so it is exhaustively testable.

use oas_catalog::ModelEntry;
use oas_hardware::ThermalState;
use serde::{Deserialize, Serialize};
use specta::Type;

const GIB: u64 = 1024 * 1024 * 1024;

/// Whisper CPU scaling flattens past about 8 threads.
pub const MAX_THREADS_PER_JOB: u32 = 8;

/// Memory always left for the OS and other apps: `max(1.5 GB, 15% of total)`.
pub fn safety_reserve(total_bytes: u64) -> u64 {
    (3 * GIB / 2).max((total_bytes as f64 * 0.15) as u64)
}

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
        Budgets {
            mem_budget: pool_bytes.saturating_sub(safety_reserve(total_bytes)),
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
    /// Memory cost from a catalog model: weights + overhead + audio working
    /// set. An unknown length counts as one minute until the worker reports it.
    pub fn for_model(model: &ModelEntry, audio_duration_ms: Option<u64>, gpu: bool) -> Self {
        let minutes = audio_duration_ms.map_or(1, |ms| ms.div_ceil(60_000).max(1));
        JobCost {
            mem_bytes: model
                .weights_bytes
                .saturating_add(model.runtime_overhead_bytes)
                .saturating_add(model.per_minute_audio_bytes.saturating_mul(minutes)),
            gpu,
        }
    }
}

/// What the governor granted a job. Returned to the job and surrendered on exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reservation {
    pub mem_bytes: u64,
    /// CPU budget this job holds (its threads; 0 for a GPU job).
    pub cpu_slots: u32,
    pub gpu_slot: bool,
    /// Threads to hand the worker.
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
    /// A job with this model ran out of memory at this concurrency before.
    LearnedLimit,
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
            DenyReason::LearnedLimit => {
                "Waiting: this model ran out of memory with more jobs at once"
            }
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

    /// Replace the budgets (live memory changes); reservations are kept.
    pub fn set_budgets(&mut self, budgets: Budgets) {
        self.budgets = budgets;
    }

    pub fn reserved_mem(&self) -> u64 {
        self.reserved_mem
    }

    /// CPU budget currently held by running jobs.
    pub fn reserved_cpu(&self) -> u32 {
        self.cpu_slots_used
    }

    /// `threads_per_job = max(1, floor(cpu_budget / concurrent_jobs_target))`,
    /// capped at [`MAX_THREADS_PER_JOB`].
    pub fn threads_for(&self, concurrent_jobs_target: u32) -> u32 {
        (self.budgets.cpu_budget / concurrent_jobs_target.max(1)).clamp(1, MAX_THREADS_PER_JOB)
    }

    /// Whether a job of this memory cost fits right now.
    pub fn mem_fits(&self, mem_bytes: u64) -> bool {
        self.reserved_mem.saturating_add(mem_bytes) <= self.budgets.mem_budget
    }

    /// Try to admit a job, reserving resources on success. `concurrent_jobs_target`
    /// is how many jobs the CPU budget should be split between.
    pub fn admit(
        &mut self,
        cost: JobCost,
        concurrent_jobs_target: u32,
        thermal: ThermalState,
        battery_blocked: bool,
    ) -> Result<Reservation, DenyReason> {
        if matches!(thermal, ThermalState::Serious | ThermalState::Critical) {
            return Err(DenyReason::Thermal);
        }
        if battery_blocked {
            return Err(DenyReason::Battery);
        }
        if !self.mem_fits(cost.mem_bytes) {
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
                threads: self.budgets.cpu_budget.clamp(1, MAX_THREADS_PER_JOB),
            }
        } else {
            let threads = self.threads_for(concurrent_jobs_target);
            if self.cpu_slots_used.saturating_add(threads) > self.budgets.cpu_budget {
                return Err(DenyReason::Cpu);
            }
            self.cpu_slots_used += threads;
            Reservation {
                mem_bytes: cost.mem_bytes,
                cpu_slots: threads,
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

/// The most jobs a user may force in Manual mode: never more than total RAM can
/// hold, however the warning is dismissed.
pub fn hard_cap(total_ram: u64, per_job_mem: u64) -> u32 {
    total_ram
        .checked_div(per_job_mem)
        .map_or(u32::MAX, |n| n.min(u32::MAX as u64) as u32)
        .max(1)
}

/// Auto mode: start at one job, add one while there is headroom and idle CPU,
/// drop one at once under pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoConcurrency {
    target: u32,
    last_raise_ms: u64,
}

impl AutoConcurrency {
    /// Time between raises, so each new job's load shows up before the next.
    pub const RAISE_INTERVAL_MS: u64 = 5_000;
    /// CPU use (percent) above which the CPU no longer counts as idle.
    pub const CPU_BUSY_PCT: f32 = 75.0;

    pub fn new() -> Self {
        AutoConcurrency {
            target: 1,
            last_raise_ms: 0,
        }
    }

    pub fn target(&self) -> u32 {
        self.target
    }

    /// Pressure seen: shed one job of concurrency, never below one.
    pub fn on_pressure(&mut self, now_ms: u64) {
        self.target = self.target.saturating_sub(1).max(1);
        self.last_raise_ms = now_ms;
    }

    /// Raise by one if the queue is saturating the target, the next job fits,
    /// and the CPU has room. Lowers to `max_safe` if the machine got tighter.
    pub fn on_tick(
        &mut self,
        now_ms: u64,
        saturated: bool,
        next_job_fits: bool,
        cpu_total_pct: f32,
        max_safe: u32,
    ) {
        let max_safe = max_safe.max(1);
        self.target = self.target.min(max_safe);
        let waited = now_ms.saturating_sub(self.last_raise_ms) >= Self::RAISE_INTERVAL_MS;
        if saturated
            && next_job_fits
            && waited
            && cpu_total_pct < Self::CPU_BUSY_PCT
            && self.target < max_safe
        {
            self.target += 1;
            self.last_raise_ms = now_ms;
        }
    }
}

impl Default for AutoConcurrency {
    fn default() -> Self {
        Self::new()
    }
}

/// Gauge level for the "Parallel jobs" control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GaugeLevel {
    Safe,
    Tight,
    TooMany,
}

/// Gauge result: level + the maximum safe concurrency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
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
            if g.admit(cost, 100, ThermalState::Nominal, false).is_ok() {
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
            if g.admit(cost, 100, ThermalState::Nominal, false).is_ok() {
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
            g.admit(cost, 100, ThermalState::Serious, false),
            Err(DenyReason::Thermal)
        );
        assert_eq!(
            g.admit(cost, 100, ThermalState::Nominal, true),
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
        assert!(g.admit(gpu, 100, ThermalState::Nominal, false).is_ok());
        assert_eq!(
            g.admit(gpu, 100, ThermalState::Nominal, false),
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
        let r1 = g.admit(cost, 100, ThermalState::Nominal, false).unwrap();
        let _r2 = g.admit(cost, 100, ThermalState::Nominal, false).unwrap();
        g.admit(cost, 100, ThermalState::Nominal, false).unwrap();
        assert!(g.admit(cost, 100, ThermalState::Nominal, false).is_err());
        g.release(&r1);
        // After releasing one, another fits again.
        assert!(g.admit(cost, 100, ThermalState::Nominal, false).is_ok());
    }

    #[test]
    fn threads_are_balanced_and_capped() {
        let mut g = Governor::new(budgets(64, 32, 0)); // cpu_budget 31
        let cost = JobCost {
            mem_bytes: GIB,
            gpu: false,
        };
        let alone = g.admit(cost, 1, ThermalState::Nominal, false).unwrap();
        assert_eq!(alone.threads, 8); // capped at 8, not 31
        let shared = g.admit(cost, 4, ThermalState::Nominal, false).unwrap();
        assert_eq!(shared.threads, 7); // 31 / 4
    }

    #[test]
    fn cpu_reservation_is_the_threads_handed_out() {
        // cpu_budget 7, split between two jobs: 3 threads each.
        let mut g = Governor::new(budgets(64, 8, 0));
        let cost = JobCost {
            mem_bytes: GIB,
            gpu: false,
        };
        let a = g.admit(cost, 2, ThermalState::Nominal, false).unwrap();
        let _b = g.admit(cost, 2, ThermalState::Nominal, false).unwrap();
        assert_eq!(g.reserved_cpu(), 6);
        // A third would need 3 more, with only 1 left.
        assert_eq!(
            g.admit(cost, 2, ThermalState::Nominal, false),
            Err(DenyReason::Cpu)
        );
        g.release(&a);
        assert_eq!(g.reserved_cpu(), 3);
    }

    #[test]
    fn long_audio_costs_more_memory() {
        let catalog =
            oas_catalog::Catalog::from_json(include_str!("../../../catalog/models.v1.json"))
                .unwrap();
        let model = catalog.get("whisper-tiny").unwrap();
        let unknown = JobCost::for_model(model, None, false).mem_bytes;
        let minute = JobCost::for_model(model, Some(1), false).mem_bytes;
        let hour = JobCost::for_model(model, Some(3_600_000), false).mem_bytes;
        assert_eq!(unknown, minute);
        assert_eq!(hour - minute, 59 * model.per_minute_audio_bytes);
    }

    #[test]
    fn hard_cap_follows_total_ram() {
        assert_eq!(hard_cap(16 * GIB, 4 * GIB), 4);
        assert_eq!(hard_cap(GIB, 4 * GIB), 1);
    }

    #[test]
    fn auto_mode_table() {
        const MS: u64 = AutoConcurrency::RAISE_INTERVAL_MS;
        // (saturated, fits, cpu%, max_safe, expected target from 1, at t=MS)
        let cases = [
            (true, true, 20.0, 4, 2),  // room everywhere: raise
            (false, true, 20.0, 4, 1), // queue not saturating: hold
            (true, false, 20.0, 4, 1), // next job does not fit: hold
            (true, true, 90.0, 4, 1),  // CPU busy: hold
            (true, true, 20.0, 1, 1),  // already at max safe: hold
        ];
        for (sat, fits, cpu, max, want) in cases {
            let mut a = AutoConcurrency::new();
            a.on_tick(MS, sat, fits, cpu, max);
            assert_eq!(a.target(), want, "{sat} {fits} {cpu} {max}");
        }
    }

    #[test]
    fn auto_mode_raises_slowly_and_sheds_fast() {
        const MS: u64 = AutoConcurrency::RAISE_INTERVAL_MS;
        let mut a = AutoConcurrency::new();
        a.on_tick(MS, true, true, 10.0, 8);
        a.on_tick(MS + 1, true, true, 10.0, 8); // too soon
        assert_eq!(a.target(), 2);
        a.on_tick(2 * MS, true, true, 10.0, 8);
        assert_eq!(a.target(), 3);
        a.on_pressure(2 * MS);
        assert_eq!(a.target(), 2);
        a.on_tick(2 * MS + 1, true, true, 10.0, 1); // machine got tighter
        assert_eq!(a.target(), 1);
        a.on_pressure(3 * MS);
        assert_eq!(a.target(), 1); // never below one
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
                let _ = g.admit(JobCost { mem_bytes: per * GIB, gpu: false }, 8, ThermalState::Nominal, false);
            }
            prop_assert!(g.reserved_mem() <= b.mem_budget);
        }

        // Any admit/release order holds the budgets and ends with nothing held.
        #[test]
        fn reservations_balance_on_every_path(
            ops in proptest::collection::vec((any::<bool>(), 1u32..8), 0..60)
        ) {
            let b = budgets(32, 16, 1);
            let mut g = Governor::new(b);
            let mut held = Vec::new();
            for (gpu, target) in ops {
                let cost = JobCost { mem_bytes: 2 * GIB, gpu };
                if let Ok(r) = g.admit(cost, target, ThermalState::Nominal, false) {
                    held.push(r);
                } else if let Some(r) = held.pop() {
                    g.release(&r);
                }
                prop_assert!(g.reserved_mem() <= b.mem_budget);
                prop_assert!(g.reserved_cpu() <= b.cpu_budget);
            }
            for r in held {
                g.release(&r);
            }
            prop_assert_eq!(g.reserved_mem(), 0);
            prop_assert_eq!(g.reserved_cpu(), 0);
        }
    }
}
