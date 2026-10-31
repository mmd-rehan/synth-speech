//! Soak and chaos tests over real processes: the real spawner and JSONL pipes,
//! with a fake worker that needs no model. These are what prove workers do not
//! leak, on every OS CI runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use oas_catalog::Catalog;
use oas_core::Paths;
use oas_hardware::ThermalState;
use oas_jobs::{
    GovernorWarning, Job, JobEvents, JobManager, JobProgress, JobRequest, JobSegment, JobState,
    Machine, ManagerDeps, ParallelMode, PauseOrigin, Priority, ProcessSpawner, Signals,
    SystemClock, WorkerRegistry,
};
use oas_storage::{Database, InstalledModel};

const GIB: u64 = 1024 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

struct Quiet;
impl JobEvents for Quiet {
    fn job_state(&self, _: &Job) {}
    fn job_removed(&self, _: &str) {}
    fn job_progress(&self, _: &JobProgress) {}
    fn job_segments(&self, _: &str, _: &[JobSegment]) {}
    fn governor_warning(&self, _: &GovernorWarning) {}
}

fn machine() -> Machine {
    Machine {
        total_ram: 64 * GIB,
        logical_cores: 16,
        gpu_slots: 0,
    }
}

fn calm() -> Signals {
    Signals {
        available_ram: 48 * GIB,
        swap_used: 0,
        thermal: ThermalState::Nominal,
        cpu_total_pct: 10.0,
        battery_low: false,
    }
}

fn manager(db: &Arc<Database>, root: &Path) -> (JobManager, WorkerRegistry) {
    let spawner = ProcessSpawner::new(PathBuf::from(env!("CARGO_BIN_EXE_oas-fake-worker")));
    let registry = spawner.registry();
    let catalog = Catalog::from_json(include_str!("../../../catalog/models.v1.json")).unwrap();
    let mgr = JobManager::new(ManagerDeps {
        db: Arc::clone(db),
        catalog,
        paths: Paths::new(root),
        clock: Arc::new(SystemClock),
        spawner: Box::new(spawner),
        events: Arc::new(Quiet),
        machine: machine(),
        rtf_estimate: Box::new(|_| 0.1),
    })
    .unwrap();
    mgr.tick(calm());
    (mgr, registry)
}

fn database(root: &Path) -> Arc<Database> {
    let db = Database::open(&root.join("oas test.db")).unwrap();
    db.save_installed_model(&InstalledModel {
        id: "whisper-tiny".into(),
        file_name: "ggml.bin".into(),
        size_bytes: 1,
        sha256: "ab".into(),
        installed_at_ms: 1,
    })
    .unwrap();
    Arc::new(db)
}

fn files(root: &Path, names: &[String]) -> Vec<String> {
    names
        .iter()
        .map(|name| {
            let path = root.join(name);
            std::fs::write(&path, b"x").unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect()
}

fn add(mgr: &JobManager, paths: Vec<String>) -> Vec<String> {
    let count = paths.len();
    mgr.create(
        JobRequest {
            paths,
            model_id: "whisper-tiny".into(),
            language: None,
            translate: false,
            word_timestamps: false,
            priority: Priority::Normal,
        },
        vec![Some(1000); count],
    )
    .unwrap()
    .into_iter()
    .map(|j| j.id)
    .collect()
}

/// A small deterministic random sequence, so a failure reproduces.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn wait_until(mgr: &JobManager, what: &str, done: impl Fn(&JobManager) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done(mgr) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:?}",
            mgr.list().iter().map(|j| j.state).collect::<Vec<_>>()
        );
        mgr.tick(calm());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_reaping(registry: &WorkerRegistry) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while registry.count() > 0 {
        assert!(Instant::now() < deadline, "worker processes leaked");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn all_settled(mgr: &JobManager) -> bool {
    mgr.list().iter().all(|j| j.state.is_terminal())
}

#[test]
fn fifty_jobs_with_random_cancels_leak_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let (mgr, registry) = manager(&db, dir.path());
    mgr.set_parallel(ParallelMode::Manual, 6);

    let names: Vec<String> = (0..50)
        .map(|i| match i % 7 {
            0 => format!("slow {i} ünï.wav"),
            1 => format!("fail {i}.wav"),
            2 => format!("crash {i}.wav"),
            3 => format!("hang slow {i}.wav"),
            _ => format!("plain {i}.wav"),
        })
        .collect();
    let ids = add(&mgr, files(dir.path(), &names));

    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for _ in 0..30 {
        let id = &ids[(rng.next() % ids.len() as u64) as usize];
        let _ = mgr.cancel(id);
        std::thread::sleep(Duration::from_millis(5));
    }

    wait_until(&mgr, "every job to settle", all_settled);
    wait_for_reaping(&registry);
    assert_eq!(mgr.held_resources(), (0, 0, 0));

    for (name, job) in names.iter().zip(ids.iter().map(|id| mgr.get(id).unwrap())) {
        // A cancelled job can be any kind; otherwise the outcome is determined.
        if job.state == JobState::Cancelled {
            continue;
        }
        let want = if name.starts_with("fail") || name.starts_with("crash") {
            JobState::Failed
        } else {
            JobState::Completed
        };
        assert_eq!(job.state, want, "{name}");
    }
}

#[test]
fn twenty_jobs_complete_in_order_and_store_whole_transcripts() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let (mgr, registry) = manager(&db, dir.path());
    mgr.set_parallel(ParallelMode::Manual, 4);
    let names: Vec<String> = (0..20).map(|i| format!("クリップ {i}.wav")).collect();
    let ids = add(&mgr, files(dir.path(), &names));

    wait_until(&mgr, "all jobs to finish", all_settled);
    wait_for_reaping(&registry);
    for id in &ids {
        let job = mgr.get(id).unwrap();
        assert_eq!(job.state, JobState::Completed, "{:?}", job.error_message);
        assert_eq!(job.rtf, Some(0.1));
        let texts: Vec<_> = mgr
            .segments(id)
            .unwrap()
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(texts, ["line 0", "line 1", "line 2"]);
    }
    assert_eq!(mgr.held_resources(), (0, 0, 0));
}

#[test]
fn a_real_pause_and_resume_gives_a_gapless_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let (mgr, registry) = manager(&db, dir.path());
    let id = add(&mgr, files(dir.path(), &["slow one.wav".to_string()])).remove(0);

    wait_until(&mgr, "a few segments", |m| {
        m.segments(&id).unwrap().len() >= 5
    });
    mgr.pause(&id).unwrap();
    wait_until(&mgr, "the pause", |m| {
        m.get(&id).unwrap().state == JobState::Paused
    });
    wait_for_reaping(&registry);
    assert_eq!(mgr.held_resources(), (0, 0, 0));
    assert_eq!(mgr.get(&id).unwrap().pause_origin, Some(PauseOrigin::User));
    let at_pause = mgr.segments(&id).unwrap().len();

    mgr.resume(&id).unwrap();
    wait_until(&mgr, "the job to finish", all_settled);
    assert_eq!(mgr.get(&id).unwrap().state, JobState::Completed);

    let segments = mgr.segments(&id).unwrap();
    assert!(segments.len() > at_pause);
    for (i, segment) in segments.iter().enumerate() {
        assert_eq!(segment.text, format!("line {i}"), "gap or repeat at {i}");
        assert_eq!(segment.idx, i as u32);
    }
}

#[test]
fn killing_workers_at_random_leaves_a_consistent_queue() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let (mgr, registry) = manager(&db, dir.path());
    mgr.set_parallel(ParallelMode::Manual, 5);
    let names: Vec<String> = (0..20).map(|i| format!("slow {i}.wav")).collect();
    add(&mgr, files(dir.path(), &names));

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(30 + rng.next() % 50));
        registry.kill_any();
    }
    mgr.pause_all();
    wait_until(&mgr, "workers to stop", |m| {
        m.list().iter().all(|j| !j.state.is_active())
    });
    wait_for_reaping(&registry);
    assert_eq!(mgr.held_resources(), (0, 0, 0));
    assert!(mgr.list().iter().any(|j| j.state == JobState::Failed));
}

#[test]
fn a_dead_core_takes_its_workers_with_it_and_the_next_start_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let (mgr, registry) = manager(&db, dir.path());
    mgr.set_parallel(ParallelMode::Manual, 4);
    let names: Vec<String> = (0..8).map(|i| format!("slow {i}.wav")).collect();
    let ids = add(&mgr, files(dir.path(), &names));
    wait_until(&mgr, "four running jobs", |m| {
        let running = m
            .list()
            .into_iter()
            .filter(|j| j.state == JobState::Running);
        running.count() == 4
    });
    let part: usize = ids[0..4]
        .iter()
        .map(|id| mgr.segments(id).unwrap().len())
        .sum();

    // Dropping the manager closes every worker's stdin, as a dead process does.
    drop(mgr);
    wait_for_reaping(&registry);

    let (next, next_registry) = manager(&db, dir.path());
    let failed: Vec<Job> = ids[0..4].iter().map(|id| next.get(id).unwrap()).collect();
    assert!(
        failed
            .iter()
            .all(|j| j.state == JobState::Failed
                && j.error_code.as_deref() == Some("JOB_INTERRUPTED"))
    );
    // Waiting jobs are untouched (the helper's first tick may start one).
    let waiting: Vec<JobState> = ids[4..]
        .iter()
        .map(|id| next.get(id).unwrap().state)
        .collect();
    assert!(
        waiting.iter().all(|s| !s.is_terminal()),
        "queued work was lost: {waiting:?}"
    );
    let kept: usize = ids[0..4]
        .iter()
        .map(|id| next.segments(id).unwrap().len())
        .sum();
    assert!(kept >= part, "partial transcripts survive the crash");

    // The queue simply carries on from there.
    next.set_parallel(ParallelMode::Manual, 8);
    next.start();
    wait_until(&next, "the queue to finish", all_settled);
    wait_for_reaping(&next_registry);
    assert_eq!(next.held_resources(), (0, 0, 0));
    assert!(ids[4..]
        .iter()
        .all(|id| next.get(id).unwrap().state == JobState::Completed));
}

#[test]
fn a_forced_gpu_failure_falls_back_to_cpu_and_the_job_completes() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let catalog = Catalog::from_json(include_str!("../../../catalog/models.v1.json")).unwrap();
    let spawner = ProcessSpawner::new(PathBuf::from(env!("CARGO_BIN_EXE_oas-fake-worker")));
    let mgr = JobManager::new(ManagerDeps {
        db,
        catalog,
        paths: Paths::new(dir.path()),
        clock: Arc::new(SystemClock),
        spawner: Box::new(spawner),
        events: Arc::new(Quiet),
        machine: Machine {
            gpu_slots: 1,
            ..machine()
        },
        rtf_estimate: Box::new(|_| 0.1),
    })
    .unwrap();
    mgr.tick(calm());
    let id = add(&mgr, files(dir.path(), &["gpubad clip.wav".to_string()])).remove(0);
    wait_until(&mgr, "the job to finish", all_settled);
    assert_eq!(mgr.get(&id).unwrap().state, JobState::Completed);
    assert_eq!(mgr.segments(&id).unwrap().len(), 3);
}

#[test]
fn a_missing_worker_binary_fails_the_job_with_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path());
    let catalog = Catalog::from_json(include_str!("../../../catalog/models.v1.json")).unwrap();
    let mgr = JobManager::new(ManagerDeps {
        db,
        catalog,
        paths: Paths::new(dir.path()),
        clock: Arc::new(SystemClock),
        spawner: Box::new(ProcessSpawner::new(dir.path().join("no-such-worker"))),
        events: Arc::new(Quiet),
        machine: machine(),
        rtf_estimate: Box::new(|_| 0.1),
    })
    .unwrap();
    let id = add(&mgr, files(dir.path(), &["a.wav".to_string()])).remove(0);
    let job = mgr.get(&id).unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error_code.as_deref(), Some("WORKER_CRASHED"));
    assert_eq!(mgr.held_resources(), (0, 0, 0));
}

#[test]
fn probing_the_worker_reads_its_version() {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_oas-fake-worker"));
    assert_eq!(oas_jobs::probe_worker(exe).unwrap(), "fake");
}
