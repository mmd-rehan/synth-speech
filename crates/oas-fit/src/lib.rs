//! The fit engine predicts, for a given model on this machine, whether it will
//! fit in memory, how fast it will run, and a plain-language verdict. It is the
//! product's signature feature, so the rules here mirror doc 03 exactly and are
//! covered by table-driven, property, and golden tests.

#![forbid(unsafe_code)]

use oas_catalog::ModelEntry;
use oas_hardware::{Accelerator, MachineProfile, ThermalState, UsageSample};
use serde::{Deserialize, Serialize};
use specta::Type;

const GIB: u64 = 1024 * 1024 * 1024;

/// Tunable constants for the fit engine, all in one place.
#[derive(Debug, Clone, Copy)]
pub struct FitConfig {
    /// Absolute floor for the system safety reserve.
    pub min_safety_reserve_bytes: u64,
    /// Safety reserve as a fraction of total RAM (the larger of the two wins).
    pub safety_reserve_fraction: f64,
    /// Verdict thresholds.
    pub excellent_headroom: f64,
    pub excellent_rtf: f32,
    pub good_headroom: f64,
    pub good_rtf: f32,
    pub slow_rtf_max: f32,
}

impl Default for FitConfig {
    fn default() -> Self {
        FitConfig {
            min_safety_reserve_bytes: 3 * GIB / 2, // 1.5 GB
            safety_reserve_fraction: 0.15,
            excellent_headroom: 0.40,
            excellent_rtf: 0.25,
            good_headroom: 0.20,
            good_rtf: 0.70,
            slow_rtf_max: 2.0,
        }
    }
}

/// The overall fit judgement, worst to best is the opposite of this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Excellent,
    Good,
    Slow,
    Risky,
    NotRecommended,
}

/// A structured reason behind a verdict, so the UI can render icons and tests
/// can assert on specifics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FitReason {
    Memory { detail: String },
    Speed { detail: String },
    Thermal { detail: String },
    Gpu { detail: String },
    Vram { detail: String },
}

/// The full result of a fit computation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FitResult {
    pub model_id: String,
    pub verdict: Verdict,
    /// Estimated real-time factor (processing time / audio duration).
    pub rtf_est: f32,
    #[specta(type = f64)]
    pub needed_bytes: u64,
    #[specta(type = f64)]
    pub pool_bytes: u64,
    #[specta(type = f64)]
    pub headroom_bytes: u64,
    /// The accelerator this prediction assumes.
    pub accelerator: Accelerator,
    /// Whether these numbers come from a real calibration run (future work).
    pub measured: bool,
    /// Whether download/run must be hard-blocked (needed exceeds total RAM).
    pub hard_blocked: bool,
    /// One-sentence plain-language summary.
    pub summary: String,
    /// Structured reasons for the detail view.
    pub reasons: Vec<FitReason>,
}

fn is_gpu(accel: &Accelerator) -> bool {
    matches!(
        accel,
        Accelerator::MetalGpu
            | Accelerator::CoreMl
            | Accelerator::Cuda { .. }
            | Accelerator::Vulkan
            | Accelerator::DirectMl
    )
}

/// Pick the accelerator to assume: the caller's override, else the machine's
/// best (profile.accel is ordered best-first), else a scalar CPU plan.
fn choose_accelerator(
    profile: &MachineProfile,
    override_accel: Option<Accelerator>,
) -> Accelerator {
    override_accel
        .or_else(|| profile.accel.first().cloned())
        .unwrap_or(Accelerator::CpuNeon)
}

fn reference_score(profile_key: &str) -> f32 {
    match profile_key {
        "apple_m_metal" => 100.0,
        "rtx3060_cuda" => 200.0,
        "ryzen_8c_avx2_cpu" => 40.0,
        _ => 50.0,
    }
}

fn preferred_profile_key(accel: &Accelerator) -> &'static str {
    match accel {
        Accelerator::MetalGpu | Accelerator::CoreMl => "apple_m_metal",
        Accelerator::Cuda { .. } => "rtx3060_cuda",
        Accelerator::Vulkan | Accelerator::DirectMl => "rtx3060_cuda",
        Accelerator::CpuAvx2 | Accelerator::CpuAvx512 | Accelerator::CpuNeon => "ryzen_8c_avx2_cpu",
    }
}

/// Choose the reference RTF entry for this accelerator, returning its rtf and
/// the reference machine's score.
fn choose_reference(model: &ModelEntry, accel: &Accelerator) -> (f32, f32) {
    let key = preferred_profile_key(accel);
    if let Some(r) = model.rtf_reference.iter().find(|r| r.profile == key) {
        return (r.rtf, reference_score(&r.profile));
    }
    match model.rtf_reference.first() {
        Some(r) => (r.rtf, reference_score(&r.profile)),
        None => (1.0, 50.0), // no data: assume roughly real time on a mid machine
    }
}

fn cpu_score(profile: &MachineProfile) -> f32 {
    let cores = profile.cpu.logical_cores.clamp(1, 16) as f32;
    let ghz = ((profile.cpu.base_mhz as f32) / 1000.0).clamp(1.5, 5.0);
    let f = &profile.cpu.features;
    let simd = if f.avx512 {
        1.5
    } else if f.avx2 {
        1.0
    } else if f.neon {
        1.1
    } else {
        0.6
    };
    cores * ghz * simd * 1.5
}

fn gpu_name_score(profile: &MachineProfile) -> f32 {
    let name = profile
        .gpus
        .iter()
        .find(|g| g.usable)
        .map(|g| g.name.to_uppercase())
        .unwrap_or_default();
    if name.contains("RTX 40") || name.contains("RTX 50") {
        300.0
    } else if name.contains("RTX 30") {
        200.0
    } else if name.contains("RTX 20") || name.contains("GTX 16") {
        130.0
    } else if name.contains("RX ") || name.contains("RADEON") {
        160.0
    } else {
        150.0
    }
}

fn machine_score(profile: &MachineProfile, accel: &Accelerator) -> f32 {
    match accel {
        Accelerator::MetalGpu | Accelerator::CoreMl => 100.0,
        Accelerator::Cuda { .. } | Accelerator::Vulkan | Accelerator::DirectMl => {
            gpu_name_score(profile)
        }
        _ => cpu_score(profile),
    }
}

fn thermal_factor(t: ThermalState) -> f32 {
    match t {
        ThermalState::Nominal | ThermalState::Unknown => 1.0,
        ThermalState::Fair => 1.3,
        ThermalState::Serious => 1.8,
        ThermalState::Critical => 2.5,
    }
}

fn battery_factor(profile: &MachineProfile) -> f32 {
    if profile.power.on_battery || profile.power.low_power_mode {
        1.25
    } else {
        1.0
    }
}

/// Live available RAM, preferring the current usage sample over the profile.
fn available_ram(profile: &MachineProfile, usage: Option<&UsageSample>) -> u64 {
    usage
        .map(|u| u.ram_available_bytes)
        .filter(|&v| v > 0)
        .unwrap_or(profile.memory.available_bytes)
}

/// Compute the full fit for `model` on `profile`, optionally overriding the
/// accelerator and factoring in the current `usage`.
pub fn compute_fit(
    profile: &MachineProfile,
    usage: Option<&UsageSample>,
    model: &ModelEntry,
    override_accel: Option<Accelerator>,
    config: &FitConfig,
) -> FitResult {
    let accel = choose_accelerator(profile, override_accel);
    let gpu_used = is_gpu(&accel) && model.gpu_capable;

    let total_ram = profile.memory.total_bytes;
    let avail_ram = available_ram(profile, usage);

    // Step 1: memory feasibility.
    let audio_working_set = model.per_minute_audio_bytes;
    let needed = model
        .weights_bytes
        .saturating_add(model.runtime_overhead_bytes)
        .saturating_add(audio_working_set);

    let mut reasons = Vec::new();

    // Determine the memory pool for the chosen plan.
    let dedicated_vram = profile
        .gpus
        .iter()
        .find(|g| g.usable && g.dedicated_vram_bytes.is_some())
        .and_then(|g| g.dedicated_vram_bytes);

    let (pool, pool_kind) = if gpu_used {
        if profile.memory.is_unified {
            (avail_ram, "shared memory")
        } else if let Some(vram) = dedicated_vram {
            (vram, "GPU memory")
        } else {
            reasons.push(FitReason::Vram {
                detail: "GPU memory could not be measured; this estimate uses system RAM and may be optimistic.".into(),
            });
            (avail_ram, "system memory")
        }
    } else {
        (avail_ram, "system memory")
    };

    let reserve = config
        .min_safety_reserve_bytes
        .max((total_ram as f64 * config.safety_reserve_fraction) as u64);

    let usable_pool = pool.saturating_sub(reserve);
    let fits = needed <= usable_pool;
    let headroom = pool.saturating_sub(needed);
    let headroom_frac = if pool > 0 {
        headroom as f64 / pool as f64
    } else {
        0.0
    };
    let hard_blocked = needed > total_ram;

    // Step 2: speed estimate.
    let (rtf_ref, ref_score) = choose_reference(model, &accel);
    let m_score = machine_score(profile, &accel).max(1.0);
    let rtf_est =
        rtf_ref * (ref_score / m_score) * thermal_factor(profile.thermal) * battery_factor(profile);

    // Step 4: verdict.
    let verdict = if !fits {
        Verdict::NotRecommended
    } else if headroom_frac >= config.excellent_headroom && rtf_est <= config.excellent_rtf {
        Verdict::Excellent
    } else if headroom_frac >= config.good_headroom && rtf_est <= config.good_rtf {
        Verdict::Good
    } else if headroom_frac < config.good_headroom
        || profile.thermal == ThermalState::Serious
        || profile.thermal == ThermalState::Critical
        || rtf_est > config.slow_rtf_max
    {
        Verdict::Risky
    } else {
        Verdict::Slow
    };

    // Step 5: reasons + summary.
    reasons.push(FitReason::Memory {
        detail: format!(
            "Needs about {} of {} available {}.",
            gb(needed),
            gb(pool),
            pool_kind
        ),
    });
    reasons.push(FitReason::Speed {
        detail: speed_phrase(rtf_est),
    });
    if gpu_used {
        reasons.push(FitReason::Gpu {
            detail: format!("Runs on {}.", accel_label(&accel)),
        });
    }
    if matches!(
        profile.thermal,
        ThermalState::Serious | ThermalState::Critical
    ) {
        reasons.push(FitReason::Thermal {
            detail: "The machine is hot, so it will run slower than usual.".into(),
        });
    }

    let summary = build_summary(
        verdict,
        &accel,
        gpu_used,
        rtf_est,
        needed,
        pool,
        hard_blocked,
    );

    FitResult {
        model_id: model.id.clone(),
        verdict,
        rtf_est,
        needed_bytes: needed,
        pool_bytes: pool,
        headroom_bytes: headroom,
        accelerator: accel,
        measured: false,
        hard_blocked,
        summary,
        reasons,
    }
}

/// Recommend the best model: highest quality tier whose verdict is Good or
/// Excellent, breaking ties by the faster (lower) estimated RTF.
pub fn recommend(
    profile: &MachineProfile,
    usage: Option<&UsageSample>,
    catalog: &oas_catalog::Catalog,
    config: &FitConfig,
) -> Option<String> {
    let mut best: Option<(u8, f32, String)> = None;
    for model in &catalog.models {
        let fit = compute_fit(profile, usage, model, None, config);
        if !matches!(fit.verdict, Verdict::Good | Verdict::Excellent) {
            continue;
        }
        let candidate = (model.quality_tier, fit.rtf_est, model.id.clone());
        match &best {
            Some((tier, rtf, _))
                if *tier > candidate.0 || (*tier == candidate.0 && *rtf <= candidate.1) => {}
            _ => best = Some(candidate),
        }
    }
    best.map(|(_, _, id)| id)
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GIB as f64)
}

fn speed_phrase(rtf: f32) -> String {
    if rtf <= 0.0 {
        return "Speed unknown.".into();
    }
    let factor = 1.0 / rtf;
    if factor >= 1.5 {
        format!("About {:.0}x faster than real time.", factor)
    } else if factor >= 0.9 {
        "Roughly real time.".into()
    } else {
        format!("About {:.1}x slower than real time.", rtf)
    }
}

fn accel_label(accel: &Accelerator) -> &'static str {
    match accel {
        Accelerator::MetalGpu => "the GPU (Metal)",
        Accelerator::CoreMl => "Core ML",
        Accelerator::Cuda { .. } => "the GPU (CUDA)",
        Accelerator::Vulkan => "the GPU (Vulkan)",
        Accelerator::DirectMl => "the GPU (DirectML)",
        Accelerator::CpuAvx512 => "the CPU (AVX-512)",
        Accelerator::CpuAvx2 => "the CPU (AVX2)",
        Accelerator::CpuNeon => "the CPU (NEON)",
    }
}

fn build_summary(
    verdict: Verdict,
    accel: &Accelerator,
    _gpu_used: bool,
    rtf: f32,
    needed: u64,
    pool: u64,
    hard_blocked: bool,
) -> String {
    let where_ = format!("Runs on {}. ", accel_label(accel));
    let speed = speed_phrase(rtf);
    let mem = format!("Uses about {} of {}.", gb(needed), gb(pool));
    match verdict {
        Verdict::Excellent => format!("Excellent. {where_}{speed} {mem}"),
        Verdict::Good => format!("Good. {where_}{speed} {mem}"),
        Verdict::Slow => format!("Slow. {where_}{speed} {mem}"),
        Verdict::Risky => format!(
            "Risky. {where_}{speed} It may use swap and slow your whole computer. {mem}"
        ),
        Verdict::NotRecommended if hard_blocked => format!(
            "Not recommended. Needs about {}, more than this computer's total memory. It may freeze your machine.",
            gb(needed)
        ),
        Verdict::NotRecommended => format!(
            "Not recommended. Needs about {} but there is not enough free memory right now. Close other apps or pick a smaller model.",
            gb(needed)
        ),
    }
}

#[cfg(test)]
mod tests;
