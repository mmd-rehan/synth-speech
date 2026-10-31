//! The real manager running the real worker and a real model. Gated on
//! OAS_TEST_MODEL (a ggml-tiny file) and OAS_TEST_AUDIO, so CI does not need a
//! model. Prints how long a batch takes at each concurrency, which is the
//! measurement behind the "Parallel jobs" gauge.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oas_catalog::Catalog;
use oas_core::Paths;
use oas_hardware::{RealProbe, SystemProbe, ThermalState};
use oas_jobs::{
    GovernorWarning, Job, JobEvents, JobManager, JobProgress, JobRequest, JobSegment, JobState,
    Machine, ManagerDeps, ParallelMode, Priority, ProcessSpawner, Signals, SystemClock,
};
use oas_storage::{Database, InstalledModel};

#[derive(Default)]
struct Log {
    live_segments: Mutex<usize>,
    progress: Mutex<usize>,
}

impl JobEvents for Log {
    fn job_state(&self, _: &Job) {}
    fn job_removed(&self, _: &str) {}
    fn job_progress(&self, _: &JobProgress) {
        *self.progress.lock().unwrap() += 1;
    }
    fn job_segments(&self, _: &str, segments: &[JobSegment]) {
        *self.live_segments.lock().unwrap() += segments.len();
    }
    fn governor_warning(&self, warning: &GovernorWarning) {
        println!("warning: {}", warning.message);
    }
}

fn real_signals(probe: &mut RealProbe, models: &Path) -> Signals {
    let usage = probe.sample_usage(models);
    Signals {
        available_ram: usage.ram_available_bytes,
        swap_used: usage.swap_used_bytes,
        thermal: ThermalState::Nominal,
        cpu_total_pct: usage.cpu_total_pct,
        battery_low: false,
    }
}

fn run_batch(
    model: &Path,
    audio: &Path,
    jobs: usize,
    parallel: u32,
    machine: Machine,
) -> (f64, u64, u32) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let model_dir = paths.models_dir().join("whisper-tiny");
    std::fs::create_dir_all(&model_dir).unwrap();
    // The app's real layout, on a path with a space and non-ASCII letters.
    std::fs::copy(model, model_dir.join("ggml tiny é.bin")).unwrap();

    let db = Arc::new(Database::open(&dir.path().join("oas.db")).unwrap());
    db.save_installed_model(&InstalledModel {
        id: "whisper-tiny".into(),
        file_name: "ggml tiny é.bin".into(),
        size_bytes: 1,
        sha256: "unchecked".into(),
        installed_at_ms: 1,
    })
    .unwrap();

    let spawner = ProcessSpawner::new(PathBuf::from(env!("CARGO_BIN_EXE_oas-worker")));
    let registry = spawner.registry();
    let log = Arc::new(Log::default());
    let catalog = Catalog::from_json(include_str!("../../../catalog/models.v1.json")).unwrap();
    let mgr = JobManager::new(ManagerDeps {
        db,
        catalog,
        paths: paths.clone(),
        clock: Arc::new(SystemClock),
        spawner: Box::new(spawner),
        events: log.clone(),
        machine,
        rtf_estimate: Box::new(|_| 0.1),
    })
    .unwrap();

    let mut probe = RealProbe::new();
    mgr.set_parallel(ParallelMode::Manual, parallel);
    mgr.tick(real_signals(&mut probe, dir.path()));

    let started = Instant::now();
    let inputs: Vec<String> = (0..jobs)
        .map(|i| {
            let copy = dir.path().join(format!("clip {i} é.wav"));
            std::fs::copy(audio, &copy).unwrap();
            copy.to_string_lossy().into_owned()
        })
        .collect();
    let ids: Vec<String> = mgr
        .create(
            JobRequest {
                paths: inputs,
                model_id: "whisper-tiny".into(),
                language: None,
                translate: false,
                word_timestamps: false,
                priority: Priority::Normal,
            },
            vec![None; jobs],
        )
        .unwrap()
        .into_iter()
        .map(|j| j.id)
        .collect();

    let mut lowest_available = u64::MAX;
    let mut busiest = 0;
    let deadline = Instant::now() + Duration::from_secs(300);
    while !mgr.list().iter().all(|j| j.state.is_terminal()) {
        assert!(
            Instant::now() < deadline,
            "the batch timed out: {:#?} {:#?}",
            mgr.status(),
            mgr.list()
                .iter()
                .map(|j| (j.state, j.wait_reason.clone(), j.error_message.clone()))
                .collect::<Vec<_>>()
        );
        let signals = real_signals(&mut probe, dir.path());
        lowest_available = lowest_available.min(signals.available_ram);
        busiest = busiest.max(registry.count());
        mgr.tick(signals);
        std::thread::sleep(Duration::from_millis(200));
    }
    let elapsed = started.elapsed().as_secs_f64();

    for id in &ids {
        let job = mgr.get(id).unwrap();
        assert_eq!(job.state, JobState::Completed, "{:?}", job.error_message);
        assert!(!mgr.segments(id).unwrap().is_empty());
    }
    assert_eq!(mgr.held_resources(), (0, 0, 0));
    assert!(busiest <= parallel as usize, "ran {busiest} at once");
    assert!(*log.live_segments.lock().unwrap() > 0);

    let transcript = mgr.render_transcript(&ids[0], "txt").unwrap();
    assert!(!transcript.trim().is_empty());
    let gauge = mgr.gauge_for("whisper-tiny", parallel).unwrap();
    println!(
        "gauge at {parallel}: {:?}, max safe {}, ran {busiest} at once",
        gauge.level, gauge.max_safe
    );
    (elapsed, lowest_available, mgr.status().cpu_budget)
}

#[test]
fn queue_runs_real_inference_at_several_concurrencies() {
    let (Ok(model), Ok(audio)) = (
        std::env::var("OAS_TEST_MODEL"),
        std::env::var("OAS_TEST_AUDIO"),
    ) else {
        eprintln!("skipping: set OAS_TEST_MODEL and OAS_TEST_AUDIO to run");
        return;
    };
    let (model, audio) = (PathBuf::from(model), PathBuf::from(audio));
    let jobs: usize = std::env::var("OAS_TEST_JOBS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);

    let probe = RealProbe::new();
    let profile = oas_hardware::build_profile(&probe, Path::new("."));
    let machine = Machine {
        total_ram: profile.memory.total_bytes,
        logical_cores: profile.cpu.logical_cores,
        gpu_slots: 0,
    };

    let mut one = 0.0;
    for parallel in [1u32, 2, 4, 8] {
        let (secs, low, cpu) = run_batch(&model, &audio, jobs, parallel, machine);
        if parallel == 1 {
            one = secs;
        }
        println!(
            "{jobs} jobs, {parallel} at once: {secs:.2} s ({:.2}x), lowest available RAM {:.1} GB, cpu budget {cpu}",
            one / secs,
            low as f64 / (1u64 << 30) as f64,
        );
    }
}
