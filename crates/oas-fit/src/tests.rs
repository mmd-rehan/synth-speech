use super::*;
use oas_catalog::{ModelEntry, RtfReference};
use oas_hardware::{
    CpuFeatures, CpuInfo, DiskInfo, GpuBackend, GpuInfo, MemInfo, OsInfo, PowerInfo,
};

const GB: u64 = 1024 * 1024 * 1024;

fn rank(v: Verdict) -> u8 {
    match v {
        Verdict::NotRecommended => 0,
        Verdict::Risky => 1,
        Verdict::Slow => 2,
        Verdict::Good => 3,
        Verdict::Excellent => 4,
    }
}

fn model(
    id: &str,
    weights: u64,
    overhead: u64,
    per_min: u64,
    tier: u8,
    refs: &[(&str, f32)],
) -> ModelEntry {
    ModelEntry {
        id: id.into(),
        name: id.into(),
        family: "whisper".into(),
        variant: id.into(),
        quant: None,
        english_only: false,
        url: "https://example.com/m.bin".into(),
        mirrors: vec![],
        weights_bytes: weights,
        sha256: "a".repeat(64),
        runtime_overhead_bytes: overhead,
        per_minute_audio_bytes: per_min,
        gpu_capable: true,
        min_vram_bytes: weights + 300 * 1024 * 1024,
        quality_tier: tier,
        license: "MIT".into(),
        rtf_reference: refs
            .iter()
            .map(|(p, r)| RtfReference {
                profile: (*p).into(),
                rtf: *r,
            })
            .collect(),
    }
}

fn tiny() -> ModelEntry {
    model(
        "whisper-tiny",
        80 * 1024 * 1024,
        250 * 1024 * 1024,
        2 * 1024 * 1024,
        1,
        &[
            ("apple_m_metal", 0.03),
            ("ryzen_8c_avx2_cpu", 0.08),
            ("rtx3060_cuda", 0.02),
        ],
    )
}

fn large() -> ModelEntry {
    model(
        "whisper-large-v3",
        3095 * 1024 * 1024,
        1100 * 1024 * 1024,
        8 * 1024 * 1024,
        5,
        &[
            ("apple_m_metal", 0.18),
            ("ryzen_8c_avx2_cpu", 1.4),
            ("rtx3060_cuda", 0.08),
        ],
    )
}

fn base_profile() -> MachineProfile {
    MachineProfile {
        os: OsInfo {
            name: "Test".into(),
            version: "1".into(),
            arch: "x86_64".into(),
        },
        cpu: CpuInfo {
            brand: "Test CPU".into(),
            physical_cores: 8,
            logical_cores: 8,
            base_mhz: 3000,
            features: CpuFeatures {
                avx2: true,
                ..Default::default()
            },
        },
        memory: MemInfo {
            total_bytes: 8 * GB,
            available_bytes: 4 * GB,
            is_unified: false,
        },
        gpus: vec![],
        disk: DiskInfo {
            free_bytes: 100 * GB,
            is_ssd: Some(true),
        },
        power: PowerInfo {
            on_battery: false,
            battery_pct: None,
            low_power_mode: false,
        },
        thermal: ThermalState::Nominal,
        accel: vec![Accelerator::CpuAvx2],
        captured_at_ms: 0,
    }
}

fn laptop_8gb() -> MachineProfile {
    base_profile()
}

fn m1_16gb() -> MachineProfile {
    let mut p = base_profile();
    p.os.arch = "aarch64".into();
    p.cpu = CpuInfo {
        brand: "Apple M1".into(),
        physical_cores: 8,
        logical_cores: 8,
        base_mhz: 3200,
        features: CpuFeatures {
            neon: true,
            ..Default::default()
        },
    };
    p.memory = MemInfo {
        total_bytes: 16 * GB,
        available_bytes: 10 * GB,
        is_unified: true,
    };
    p.gpus = vec![GpuInfo {
        vendor: "Apple".into(),
        name: "Apple M1".into(),
        backend: GpuBackend::Metal,
        dedicated_vram_bytes: None,
        shared_bytes: Some(16 * GB),
        driver: None,
        usable: true,
        not_usable_reason: None,
    }];
    p.accel = vec![
        Accelerator::MetalGpu,
        Accelerator::CoreMl,
        Accelerator::CpuNeon,
    ];
    p
}

fn rtx_32gb() -> MachineProfile {
    let mut p = base_profile();
    p.cpu.logical_cores = 16;
    p.memory = MemInfo {
        total_bytes: 32 * GB,
        available_bytes: 24 * GB,
        is_unified: false,
    };
    p.gpus = vec![GpuInfo {
        vendor: "NVIDIA".into(),
        name: "NVIDIA GeForce RTX 3060".into(),
        backend: GpuBackend::Cuda,
        dedicated_vram_bytes: Some(12 * GB),
        shared_bytes: None,
        driver: Some("555".into()),
        usable: true,
        not_usable_reason: None,
    }];
    p.accel = vec![Accelerator::Cuda {
        compute_cap: String::new(),
    }];
    p
}

fn old_4gb() -> MachineProfile {
    let mut p = base_profile();
    p.cpu = CpuInfo {
        brand: "Old".into(),
        physical_cores: 2,
        logical_cores: 2,
        base_mhz: 2000,
        features: CpuFeatures::default(),
    };
    p.memory = MemInfo {
        total_bytes: 4 * GB,
        available_bytes: 2 * GB,
        is_unified: false,
    };
    p.accel = vec![];
    p
}

fn cfg() -> FitConfig {
    FitConfig::default()
}

#[test]
fn tiny_on_laptop_is_excellent() {
    let f = compute_fit(&laptop_8gb(), None, &tiny(), None, &cfg());
    assert_eq!(f.verdict, Verdict::Excellent, "{}", f.summary);
}

#[test]
fn large_on_m1_is_excellent_or_good() {
    let f = compute_fit(&m1_16gb(), None, &large(), None, &cfg());
    assert!(
        matches!(f.verdict, Verdict::Excellent | Verdict::Good),
        "got {:?}: {}",
        f.verdict,
        f.summary
    );
}

#[test]
fn large_on_rtx_box_is_excellent_or_good() {
    let f = compute_fit(&rtx_32gb(), None, &large(), None, &cfg());
    assert!(
        matches!(f.verdict, Verdict::Excellent | Verdict::Good),
        "got {:?}: {}",
        f.verdict,
        f.summary
    );
}

#[test]
fn large_on_4gb_is_not_recommended_and_hard_blocked() {
    let f = compute_fit(&old_4gb(), None, &large(), None, &cfg());
    assert_eq!(f.verdict, Verdict::NotRecommended);
    assert!(
        f.hard_blocked,
        "needed exceeds total RAM so it must hard block"
    );
}

#[test]
fn large_on_laptop_8gb_is_not_recommended_but_not_hard_blocked() {
    let f = compute_fit(&laptop_8gb(), None, &large(), None, &cfg());
    assert_eq!(f.verdict, Verdict::NotRecommended);
    assert!(!f.hard_blocked, "fits in total RAM, just not right now");
}

#[test]
fn battery_and_thermal_slow_things_down() {
    let mut p = laptop_8gb();
    let base = compute_fit(&p, None, &tiny(), None, &cfg()).rtf_est;
    p.power.on_battery = true;
    p.thermal = ThermalState::Serious;
    let worse = compute_fit(&p, None, &tiny(), None, &cfg());
    assert!(worse.rtf_est > base, "battery + heat must raise rtf");
}

#[test]
fn recommend_prefers_quality_among_fitting_models() {
    let catalog = oas_catalog::Catalog {
        schema_version: 1,
        models: vec![tiny(), large()],
    };
    // On a 32 GB RTX box both fit; the higher-tier large should win.
    let rec = recommend(&rtx_32gb(), None, &catalog, &cfg());
    assert_eq!(rec.as_deref(), Some("whisper-large-v3"));
    // On an 8 GB laptop large is Not recommended, so tiny (Excellent) wins.
    let rec = recommend(&laptop_8gb(), None, &catalog, &cfg());
    assert_eq!(rec.as_deref(), Some("whisper-tiny"));
    // On a weak 4 GB / 2-core machine nothing reaches Good, so None.
    let rec = recommend(&old_4gb(), None, &catalog, &cfg());
    assert_eq!(rec, None);
}

#[test]
fn golden_summary_for_tiny_on_m1() {
    let f = compute_fit(&m1_16gb(), None, &tiny(), None, &cfg());
    assert!(f.summary.starts_with("Excellent."), "{}", f.summary);
    assert!(f.summary.contains("faster than real time"), "{}", f.summary);
    assert!(f.summary.contains("GB"), "{}", f.summary);
}

#[test]
fn golden_summary_for_hard_block() {
    let f = compute_fit(&old_4gb(), None, &large(), None, &cfg());
    assert!(f.summary.starts_with("Not recommended."), "{}", f.summary);
    assert!(f.summary.contains("total memory"), "{}", f.summary);
}

// Property: more free memory never worsens the verdict.
use proptest::prelude::*;

proptest! {
    #[test]
    fn more_free_memory_never_worsens(avail_gb in 1u64..60, bump_gb in 0u64..40) {
        let used = 3u64 * GB; // constant in-use memory
        let mut low = base_profile();
        low.memory = MemInfo {
            total_bytes: avail_gb * GB + used,
            available_bytes: avail_gb * GB,
            is_unified: false,
        };
        let mut high = low.clone();
        let hi_avail = avail_gb + bump_gb;
        high.memory = MemInfo {
            total_bytes: hi_avail * GB + used,
            available_bytes: hi_avail * GB,
            is_unified: false,
        };

        let m = large();
        let low_fit = compute_fit(&low, None, &m, None, &cfg());
        let high_fit = compute_fit(&high, None, &m, None, &cfg());
        prop_assert!(
            rank(high_fit.verdict) >= rank(low_fit.verdict),
            "more memory worsened verdict: {:?} -> {:?}",
            low_fit.verdict,
            high_fit.verdict
        );
    }
}

#[test]
fn a_correction_scales_the_speed_and_marks_it_measured() {
    let plain = compute_fit(&m1_16gb(), None, &tiny(), None, &cfg());
    assert!(!plain.measured);
    let tuned = compute_fit_calibrated(&m1_16gb(), None, &tiny(), None, &cfg(), Some(2.0));
    assert!(tuned.measured);
    assert!((tuned.rtf_est - plain.rtf_est * 2.0).abs() < 0.0001);
}

#[test]
fn a_correction_can_change_the_verdict() {
    let model = tiny();
    let fast = compute_fit_calibrated(&m1_16gb(), None, &model, None, &cfg(), Some(0.1));
    let slow = compute_fit_calibrated(&m1_16gb(), None, &model, None, &cfg(), Some(500.0));
    assert_eq!(fast.verdict, Verdict::Excellent);
    assert_eq!(slow.verdict, Verdict::Risky);
}

#[test]
fn correction_updates_table() {
    // (previous, measured, estimated, expected factor, expected samples)
    let cases = [
        (None, 0.5, 1.0, 0.5, 1),
        (Some((0.5, 1)), 1.5, 1.0, 1.0, 2), // average of the two
        (Some((1.0, 100)), 2.0, 1.0, 1.1, 101), // a long history moves by 1/10
    ];
    for (previous, measured, estimated, factor, samples) in cases {
        let (got, n) = update_correction(previous, measured, estimated).unwrap();
        assert!((got - factor).abs() < 0.0001, "{previous:?}: {got}");
        assert_eq!(n, samples);
    }
}

#[test]
fn impossible_measurements_are_refused() {
    for (measured, estimated) in [
        (0.0, 1.0),
        (-1.0, 1.0),
        (1.0, 0.0),
        (f32::NAN, 1.0),
        (f32::INFINITY, 1.0),
    ] {
        assert_eq!(update_correction(Some((1.0, 3)), measured, estimated), None);
    }
}

proptest! {
    // However many runs arrive, the factor stays within what was measured.
    #[test]
    fn correction_stays_within_the_measured_range(
        samples in proptest::collection::vec(0.01f32..10.0, 1..60)
    ) {
        let mut state = None;
        for s in &samples {
            state = update_correction(state, *s, 1.0);
        }
        let (factor, _) = state.unwrap();
        let lo = samples.iter().copied().fold(f32::MAX, f32::min);
        let hi = samples.iter().copied().fold(f32::MIN, f32::max);
        prop_assert!(factor >= lo - 0.001 && factor <= hi + 0.001);
    }
}
