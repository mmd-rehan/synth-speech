//! Deterministic manager tests: a fake clock and fake workers, driven one event
//! at a time.

use std::sync::atomic::{AtomicU64, Ordering};

use oas_storage::InstalledModel;
use proptest::prelude::*;

use super::*;

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Default)]
struct FakeClock(AtomicU64);

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct WorkerLog {
    /// (job id, attempt) of every spawn.
    spawned: Vec<(String, u64)>,
    /// (attempt, message) of everything sent to workers.
    sent: Vec<(u64, CoreToWorker)>,
    killed: Vec<u64>,
    fail_next_spawn: bool,
}

#[derive(Clone, Default)]
struct FakeSpawner(Arc<Mutex<WorkerLog>>);

struct FakeHandle {
    attempt: u64,
    log: Arc<Mutex<WorkerLog>>,
}

impl WorkerHandle for FakeHandle {
    fn send(&mut self, msg: &CoreToWorker) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .sent
            .push((self.attempt, msg.clone()));
        Ok(())
    }
    fn kill(&mut self) {
        self.log.lock().unwrap().killed.push(self.attempt);
    }
}

impl WorkerSpawner for FakeSpawner {
    fn spawn(
        &self,
        job_id: &str,
        attempt: u64,
        _sink: WorkerSink,
    ) -> Result<Box<dyn WorkerHandle>> {
        let mut log = self.0.lock().unwrap();
        if std::mem::take(&mut log.fail_next_spawn) {
            return Err(OasError::WorkerCrashed {
                reason: "no binary".into(),
            });
        }
        log.spawned.push((job_id.to_string(), attempt));
        Ok(Box::new(FakeHandle {
            attempt,
            log: Arc::clone(&self.0),
        }))
    }
}

#[derive(Default)]
struct Recorder {
    states: Mutex<Vec<(String, JobState)>>,
    progress: Mutex<Vec<JobProgress>>,
    segments: Mutex<Vec<JobSegment>>,
    warnings: Mutex<Vec<GovernorWarning>>,
}

impl JobEvents for Recorder {
    fn job_state(&self, job: &Job) {
        self.states
            .lock()
            .unwrap()
            .push((job.id.clone(), job.state));
    }
    fn job_progress(&self, progress: &JobProgress) {
        self.progress.lock().unwrap().push(progress.clone());
    }
    fn job_segments(&self, _job_id: &str, segments: &[JobSegment]) {
        self.segments.lock().unwrap().extend_from_slice(segments);
    }
    fn governor_warning(&self, warning: &GovernorWarning) {
        self.warnings.lock().unwrap().push(warning.clone());
    }
}

struct Harness {
    mgr: JobManager,
    clock: Arc<FakeClock>,
    workers: FakeSpawner,
    events: Arc<Recorder>,
    db: Arc<Database>,
    dir: tempfile::TempDir,
    machine: Machine,
}

fn catalog() -> Catalog {
    Catalog::from_json(include_str!("../../../../catalog/models.v1.json")).unwrap()
}

fn manager_on(
    db: Arc<Database>,
    root: &Path,
    clock: Arc<FakeClock>,
    workers: FakeSpawner,
    events: Arc<Recorder>,
    machine: Machine,
) -> JobManager {
    JobManager::new(ManagerDeps {
        db,
        catalog: catalog(),
        paths: Paths::new(root),
        clock,
        spawner: Box::new(workers),
        events,
        machine,
        rtf_estimate: Box::new(|_| 0.5),
    })
    .unwrap()
}

fn harness_with(machine: Machine) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::open_in_memory().unwrap());
    for id in ["whisper-tiny", "whisper-base"] {
        db.save_installed_model(&InstalledModel {
            id: id.into(),
            file_name: "ggml.bin".into(),
            size_bytes: 1,
            sha256: "ab".into(),
            installed_at_ms: 1,
        })
        .unwrap();
    }
    let clock = Arc::new(FakeClock(AtomicU64::new(1_000_000)));
    let workers = FakeSpawner::default();
    let events = Arc::new(Recorder::default());
    let mgr = manager_on(
        Arc::clone(&db),
        dir.path(),
        Arc::clone(&clock),
        workers.clone(),
        Arc::clone(&events),
        machine,
    );
    Harness {
        mgr,
        clock,
        workers,
        events,
        db,
        dir,
        machine,
    }
}

/// 32 GB, 16 cores, no GPU: plenty of room for tiny jobs.
fn harness() -> Harness {
    harness_with(Machine {
        total_ram: 32 * GIB,
        logical_cores: 16,
        gpu_slots: 0,
    })
}

impl Harness {
    fn calm(&self) -> Signals {
        Signals {
            available_ram: 24 * GIB,
            swap_used: 0,
            thermal: ThermalState::Nominal,
            cpu_total_pct: 10.0,
            battery_low: false,
        }
    }

    fn advance(&self, ms: u64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    /// Add `n` jobs for `model`, each with its own real file.
    fn add(&self, n: usize, model: &str) -> Vec<String> {
        let paths: Vec<String> = (0..n)
            .map(|i| {
                let unique = JobId::new();
                let path = self.dir.path().join(format!("clip é {unique} {i}.wav"));
                std::fs::write(&path, b"x").unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        let durations = vec![Some(60_000); n];
        self.mgr
            .create(
                JobRequest {
                    paths,
                    model_id: model.into(),
                    language: None,
                    translate: false,
                    word_timestamps: false,
                    priority: Priority::Normal,
                },
                durations,
            )
            .unwrap()
            .into_iter()
            .map(|j| j.id)
            .collect()
    }

    fn state(&self, id: &str) -> JobState {
        self.mgr.get(id).unwrap().state
    }

    fn attempt(&self, id: &str) -> u64 {
        let log = self.workers.0.lock().unwrap();
        let found = log.spawned.iter().rev().find(|(job, _)| job == id);
        found.map(|(_, a)| *a).expect("job was spawned")
    }

    fn spawned(&self) -> Vec<String> {
        let log = self.workers.0.lock().unwrap();
        log.spawned.iter().map(|(id, _)| id.clone()).collect()
    }

    fn killed(&self, id: &str) -> bool {
        let attempt = self.attempt(id);
        self.workers.0.lock().unwrap().killed.contains(&attempt)
    }

    fn send(&self, id: &str, message: WorkerToCore) {
        self.mgr.on_worker_event(WorkerEvent {
            job_id: id.to_string(),
            attempt: self.attempt(id),
            kind: WorkerEventKind::Message(message),
        });
    }

    fn ready(&self, id: &str) {
        self.send(
            id,
            WorkerToCore::Ready {
                worker_version: "t".into(),
            },
        );
    }

    fn segment(&self, id: &str, start: f64, end: f64, text: &str) {
        self.send(
            id,
            WorkerToCore::Segment {
                start,
                end,
                text: text.into(),
                words: vec![],
            },
        );
    }

    fn exit(&self, id: &str, success: bool, oom_suspected: bool) {
        self.mgr.on_worker_event(WorkerEvent {
            job_id: id.to_string(),
            attempt: self.attempt(id),
            kind: WorkerEventKind::Exited(WorkerExit {
                success,
                oom_suspected,
                detail: "test exit".into(),
            }),
        });
    }

    /// A clean run of a job that has already been spawned.
    fn complete(&self, id: &str) {
        self.ready(id);
        self.send(id, WorkerToCore::Done);
        self.exit(id, true, false);
    }

    fn start_of(&self, id: &str) -> CoreToWorker {
        let attempt = self.attempt(id);
        let log = self.workers.0.lock().unwrap();
        log.sent
            .iter()
            .rev()
            .find(|(a, m)| *a == attempt && matches!(m, CoreToWorker::Start { .. }))
            .map(|(_, m)| m.clone())
            .expect("a start message")
    }

    fn cancels_sent(&self, id: &str) -> usize {
        let attempt = self.attempt(id);
        let log = self.workers.0.lock().unwrap();
        log.sent
            .iter()
            .filter(|(a, m)| *a == attempt && matches!(m, CoreToWorker::Cancel))
            .count()
    }

    fn assert_nothing_held(&self) {
        assert_eq!(self.mgr.held_resources(), (0, 0, 0), "resources leaked");
    }

    fn warnings(&self) -> Vec<WarningReason> {
        let warnings = self.events.warnings.lock().unwrap();
        warnings.iter().map(|w| w.reason).collect()
    }
}

#[test]
fn job_runs_through_every_state_and_stores_its_transcript() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    assert_eq!(h.state(&id), JobState::Starting);

    h.ready(&id);
    assert_eq!(h.state(&id), JobState::Running);
    let CoreToWorker::Start {
        audio_path,
        model_path,
        limits,
        resume_from_sec,
        ..
    } = h.start_of(&id)
    else {
        panic!("not a start");
    };
    assert!(audio_path.contains("clip é"));
    assert!(model_path.ends_with("ggml.bin"));
    assert!(model_path.contains("whisper-tiny"));
    assert!(limits.max_threads >= 1);
    assert_eq!(resume_from_sec, None);

    h.send(&id, WorkerToCore::Audio { duration_sec: 90.0 });
    h.segment(&id, 0.0, 1.5, "hello");
    h.segment(&id, 1.5, 3.0, "world");
    h.send(
        &id,
        WorkerToCore::Metrics {
            rtf: 0.25,
            peak_rss_mib: 100,
        },
    );
    h.send(&id, WorkerToCore::Done);
    // Done alone does not free resources: that happens when the process exits.
    assert_eq!(h.mgr.held_resources().0, 1);
    h.exit(&id, true, false);

    let job = h.mgr.get(&id).unwrap();
    assert_eq!(job.state, JobState::Completed);
    assert_eq!(job.progress, 1.0);
    assert_eq!(job.duration_ms, Some(90_000));
    assert_eq!(job.rtf, Some(0.25));
    assert!(job.finished_at_ms.is_some());
    let texts: Vec<_> = h
        .mgr
        .segments(&id)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(texts, ["hello", "world"]);
    assert_eq!(h.events.segments.lock().unwrap().len(), 2);
    h.assert_nothing_held();
}

#[test]
fn twenty_queued_jobs_complete_with_correct_states_and_no_leaks() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 3);
    let ids = h.add(20, "whisper-tiny");

    let mut finished = 0;
    while finished < 20 {
        let running: Vec<String> = ids
            .iter()
            .filter(|id| h.state(id) == JobState::Starting)
            .cloned()
            .collect();
        assert!(!running.is_empty() && running.len() <= 3);
        for id in running {
            h.complete(&id);
            finished += 1;
        }
    }
    assert!(ids.iter().all(|id| h.state(id) == JobState::Completed));
    // Started first-in, first-out.
    assert_eq!(h.spawned(), ids);
    h.assert_nothing_held();
}

#[test]
fn creating_jobs_rejects_bad_input_and_adds_nothing() {
    let h = harness();
    let good = h.dir.path().join("good.wav");
    std::fs::write(&good, b"x").unwrap();
    let good = good.to_string_lossy().into_owned();
    let request = |paths: Vec<String>, model: &str| JobRequest {
        paths,
        model_id: model.into(),
        language: None,
        translate: false,
        word_timestamps: false,
        priority: Priority::Normal,
    };

    let cases = [
        (vec![], "whisper-tiny", "INVALID_INPUT"),
        (vec![good.clone()], "whisper-small", "MODEL_NOT_INSTALLED"),
        (vec![good.clone()], "no-such-model", "MODEL_NOT_INSTALLED"),
        (
            vec![good.clone(), "/no/such/file.wav".into()],
            "whisper-tiny",
            "FILE_NOT_FOUND",
        ),
    ];
    for (paths, model, code) in cases {
        let err = h.mgr.create(request(paths, model), vec![]).unwrap_err();
        assert_eq!(err.code(), code);
    }
    assert!(h.mgr.list().is_empty());
}

#[test]
fn priority_then_fifo_decides_the_next_job() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 1);
    let ids = h.add(4, "whisper-tiny");
    h.mgr.set_priority(&ids[3], Priority::High).unwrap();
    h.mgr.set_priority(&ids[1], Priority::Low).unwrap();

    h.complete(&ids[0]);
    assert_eq!(h.state(&ids[3]), JobState::Starting, "high goes first");
    h.complete(&ids[3]);
    assert_eq!(h.state(&ids[2]), JobState::Starting, "normal before low");
    h.complete(&ids[2]);
    assert_eq!(h.state(&ids[1]), JobState::Starting);
}

#[test]
fn reorder_changes_the_queue_without_touching_other_jobs() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 1);
    let ids = h.add(4, "whisper-tiny"); // ids[0] is running
    h.mgr.reorder(&[ids[3].clone(), ids[1].clone()]).unwrap();
    h.complete(&ids[0]);
    assert_eq!(h.state(&ids[3]), JobState::Starting);
    h.complete(&ids[3]);
    assert_eq!(h.state(&ids[2]), JobState::Starting, "ids[2] kept its slot");
}

#[test]
fn cancelling_a_running_job_asks_first_then_kills_after_the_grace_period() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.mgr.cancel(&id).unwrap();
    assert_eq!(h.cancels_sent(&id), 1);
    assert!(!h.killed(&id));

    h.advance(STOP_GRACE_MS - 1);
    h.mgr.tick(h.calm());
    assert!(!h.killed(&id));
    h.advance(1);
    h.mgr.tick(h.calm());
    assert!(h.killed(&id));

    h.exit(&id, false, true); // our own kill must not read as out-of-memory
    assert_eq!(h.state(&id), JobState::Cancelled);
    assert!(h.warnings().is_empty());
    h.assert_nothing_held();
}

#[test]
fn stopping_a_worker_that_never_got_its_job_kills_it_at_once() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.mgr.cancel(&id).unwrap();
    assert!(h.killed(&id));
    h.ready(&id); // already on its way: must not start the job now
    assert_eq!(h.workers.0.lock().unwrap().sent.len(), 0);
    h.exit(&id, false, false);
    assert_eq!(h.state(&id), JobState::Cancelled);
    h.assert_nothing_held();
}

#[test]
fn queued_and_paused_jobs_cancel_immediately() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 1);
    let ids = h.add(3, "whisper-tiny");
    h.mgr.cancel(&ids[1]).unwrap();
    assert_eq!(h.state(&ids[1]), JobState::Cancelled);
    h.mgr.pause(&ids[2]).unwrap();
    assert_eq!(h.state(&ids[2]), JobState::Paused);
    h.mgr.cancel(&ids[2]).unwrap();
    assert_eq!(h.state(&ids[2]), JobState::Cancelled);
}

#[test]
fn pause_stops_the_worker_frees_resources_and_resume_continues_the_transcript() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.segment(&id, 0.0, 2.5, "first");
    h.mgr.pause(&id).unwrap();
    assert_eq!(h.state(&id), JobState::Running, "until the worker has gone");
    h.exit(&id, true, false);

    let job = h.mgr.get(&id).unwrap();
    assert_eq!(job.state, JobState::Paused);
    assert_eq!(job.pause_origin, Some(PauseOrigin::User));
    h.assert_nothing_held();

    h.mgr.resume(&id).unwrap();
    assert_eq!(h.state(&id), JobState::Starting);
    h.ready(&id);
    let CoreToWorker::Start {
        resume_from_sec, ..
    } = h.start_of(&id)
    else {
        panic!("not a start");
    };
    assert_eq!(resume_from_sec, Some(2.5));
    h.segment(&id, 2.5, 4.0, "second");
    h.send(&id, WorkerToCore::Done);
    h.exit(&id, true, false);

    let segments = h.mgr.segments(&id).unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].idx, 1);
    assert_eq!(segments[1].text, "second");
}

#[test]
fn a_job_finished_as_a_stop_was_requested_counts_as_finished() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.mgr.cancel(&id).unwrap();
    h.send(&id, WorkerToCore::Done);
    h.exit(&id, true, false);
    assert_eq!(h.state(&id), JobState::Completed);
}

#[test]
fn failures_are_classified_from_the_worker_and_its_exit() {
    // (error reported, oom-looking exit, expected code)
    let info = ErrorInfo::from(OasError::DecodeFailed { reason: "x".into() });
    let cases = [
        (Some(info), false, "DECODE_FAILED"),
        (None, true, "OUT_OF_MEMORY"),
        (None, false, "WORKER_CRASHED"),
    ];
    for (error, oom, code) in cases {
        let h = harness();
        let id = h.add(1, "whisper-tiny").remove(0);
        h.ready(&id);
        if let Some(error) = error {
            h.send(&id, WorkerToCore::Error { error });
        }
        h.exit(&id, false, oom);
        let job = h.mgr.get(&id).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.error_code.as_deref(), Some(code));
        h.assert_nothing_held();
    }
}

#[test]
fn a_spawn_failure_fails_the_job_and_the_queue_moves_on() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 1);
    h.workers.0.lock().unwrap().fail_next_spawn = true;
    let ids = h.add(2, "whisper-tiny");
    assert_eq!(h.state(&ids[0]), JobState::Failed);
    assert_eq!(h.state(&ids[1]), JobState::Starting);
    assert_eq!(h.mgr.held_resources().0, 1);
}

#[test]
fn a_worker_that_never_becomes_ready_is_killed_and_reported() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.advance(START_TIMEOUT_MS);
    h.mgr.tick(h.calm());
    assert!(h.killed(&id));
    h.exit(&id, false, true);
    let job = h.mgr.get(&id).unwrap();
    assert_eq!(job.error_code.as_deref(), Some("WORKER_CRASHED"));
}

#[test]
fn events_from_an_old_worker_are_ignored() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    let old = h.attempt(&id);
    h.ready(&id);
    h.mgr.pause(&id).unwrap();
    h.exit(&id, true, false);
    h.mgr.resume(&id).unwrap();
    assert_ne!(old, h.attempt(&id));

    h.mgr.on_worker_event(WorkerEvent {
        job_id: id.clone(),
        attempt: old,
        kind: WorkerEventKind::Exited(WorkerExit {
            success: false,
            oom_suspected: true,
            detail: "stale".into(),
        }),
    });
    assert_eq!(h.state(&id), JobState::Starting);
    assert_eq!(h.mgr.held_resources().0, 1);
}

#[test]
fn retry_starts_clean() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.segment(&id, 0.0, 1.0, "partial");
    h.exit(&id, false, false);
    assert_eq!(h.state(&id), JobState::Failed);

    h.mgr.retry(&id).unwrap();
    let job = h.mgr.get(&id).unwrap();
    assert_eq!(job.state, JobState::Starting);
    assert_eq!(job.retries, 1);
    assert_eq!(job.error_code, None);
    assert!(h.mgr.segments(&id).unwrap().is_empty());
    h.ready(&id);
    let CoreToWorker::Start {
        resume_from_sec, ..
    } = h.start_of(&id)
    else {
        panic!("not a start");
    };
    assert_eq!(resume_from_sec, None);
}

#[test]
fn illegal_commands_are_rejected_not_applied() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.complete(&id);
    assert_eq!(h.mgr.pause(&id).unwrap_err().code(), "INVALID_INPUT");
    assert_eq!(h.mgr.resume(&id).unwrap_err().code(), "INVALID_INPUT");
    assert_eq!(h.mgr.cancel(&id).unwrap_err().code(), "INVALID_INPUT");
    assert_eq!(h.mgr.retry(&id).unwrap_err().code(), "INVALID_INPUT");
    assert_eq!(h.mgr.get("nope").unwrap_err().code(), "INVALID_INPUT");
    assert_eq!(h.state(&id), JobState::Completed);
}

#[test]
fn deleting_a_running_job_waits_for_the_worker() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.segment(&id, 0.0, 1.0, "x");
    h.mgr.delete(&id).unwrap();
    assert_eq!(h.mgr.held_resources().0, 1);
    h.exit(&id, true, false);
    assert!(h.mgr.get(&id).is_err());
    assert!(h.db.list_jobs().unwrap().is_empty());
    assert!(h.db.list_segments(&id).unwrap().is_empty());
    h.assert_nothing_held();
}

#[test]
fn clear_completed_leaves_everything_else() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 1);
    let ids = h.add(2, "whisper-tiny");
    h.complete(&ids[0]);
    h.mgr.clear_completed().unwrap();
    assert!(h.mgr.get(&ids[0]).is_err());
    assert!(h.mgr.get(&ids[1]).is_ok());
}

#[test]
fn pause_all_and_resume_all() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 1);
    let ids = h.add(3, "whisper-tiny");
    h.ready(&ids[0]);
    h.mgr.pause_all();
    h.exit(&ids[0], true, false);
    assert!(ids.iter().all(|id| h.state(id) == JobState::Paused));
    h.assert_nothing_held();
    h.mgr.resume_all();
    let running = ids.iter().filter(|id| h.state(id) == JobState::Starting);
    assert_eq!(running.count(), 1);
}

#[test]
fn progress_events_are_throttled_and_never_go_backwards() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    let progress = |pct: f32| WorkerToCore::Progress {
        pct,
        stage: "transcribing".into(),
    };
    h.advance(1000);
    h.send(&id, progress(10.0));
    for pct in 11..50 {
        h.send(&id, progress(pct as f32)); // same instant: all throttled
    }
    assert_eq!(h.events.progress.lock().unwrap().len(), 1);
    h.advance(EMIT_INTERVAL_MS);
    h.send(&id, progress(5.0));
    let events = h.events.progress.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert!((events[1].progress - 0.49).abs() < 0.001);
    assert!(events[1].eta_secs.is_some());
}

#[test]
fn segments_are_stored_at_once_but_sent_to_the_ui_in_batches() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.advance(1000);
    h.segment(&id, 0.0, 1.0, "a"); // sent, and starts the interval
    h.segment(&id, 1.0, 2.0, "b");
    h.segment(&id, 2.0, 3.0, "c");
    assert_eq!(h.db.list_segments(&id).unwrap().len(), 3);
    assert_eq!(h.events.segments.lock().unwrap().len(), 1);
    h.mgr.tick(h.calm());
    assert_eq!(h.events.segments.lock().unwrap().len(), 3);
}

// --- Governor behaviour under synthetic usage traces -----------------------

fn four_running(h: &Harness) -> Vec<String> {
    h.mgr.set_parallel(ParallelMode::Manual, 4);
    let ids = h.add(4, "whisper-tiny");
    for id in &ids {
        h.ready(id);
    }
    ids
}

#[test]
fn memory_pressure_pauses_the_newest_job_and_resumes_it_after_recovery() {
    let h = harness();
    let ids = four_running(&h);
    h.advance(WATCHDOG_COOLDOWN_MS);
    let low = Signals {
        available_ram: 100 * 1024 * 1024,
        ..h.calm()
    };

    h.mgr.tick(low);
    assert_eq!(h.cancels_sent(&ids[3]), 1, "newest is stopped");
    assert_eq!(h.cancels_sent(&ids[2]), 0);
    h.exit(&ids[3], true, false);
    let paused = h.mgr.get(&ids[3]).unwrap();
    assert_eq!(paused.state, JobState::Paused);
    assert_eq!(paused.pause_origin, Some(PauseOrigin::Governor));
    assert_eq!(h.warnings(), [WarningReason::LowMemory]);

    // Still low, but the first pause has had no time to work: one at a time.
    h.advance(1000);
    h.mgr.tick(low);
    assert_eq!(h.cancels_sent(&ids[2]), 0);

    // Recovered, but give it the cooldown before bringing work back.
    h.advance(1000);
    h.mgr.tick(h.calm());
    assert_eq!(h.state(&ids[3]), JobState::Paused);
    h.advance(WATCHDOG_COOLDOWN_MS);
    h.mgr.tick(h.calm());
    assert_eq!(h.state(&ids[3]), JobState::Starting);
    assert_eq!(
        h.warnings(),
        [WarningReason::LowMemory, WarningReason::Recovered]
    );
}

#[test]
fn a_memory_spike_keeps_shedding_until_it_stops() {
    let h = harness();
    let ids = four_running(&h);
    let low = Signals {
        available_ram: 100 * 1024 * 1024,
        ..h.calm()
    };
    for newest in ids.iter().rev() {
        h.advance(WATCHDOG_COOLDOWN_MS);
        h.mgr.tick(low);
        assert_eq!(h.cancels_sent(newest), 1);
        h.exit(newest, true, false);
    }
    h.assert_nothing_held();
}

#[test]
fn a_user_pause_is_never_undone_by_recovery() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.mgr.pause(&id).unwrap();
    h.exit(&id, true, false);
    h.advance(10 * WATCHDOG_COOLDOWN_MS);
    h.mgr.tick(h.calm());
    assert_eq!(h.state(&id), JobState::Paused);
}

#[test]
fn a_swap_storm_pauses_the_newest_job() {
    let h = harness();
    let ids = four_running(&h);
    h.advance(WATCHDOG_COOLDOWN_MS);
    // Slow growth over a long time is harmless.
    for mb in [100u64, 200, 300] {
        h.advance(SWAP_WINDOW_MS + 1);
        h.mgr.tick(Signals {
            swap_used: mb * 1024 * 1024,
            ..h.calm()
        });
    }
    assert_eq!(h.cancels_sent(&ids[3]), 0);
    // 600 MB in five seconds is not.
    h.advance(5_000);
    h.mgr.tick(Signals {
        swap_used: 900 * 1024 * 1024,
        ..h.calm()
    });
    assert_eq!(h.cancels_sent(&ids[3]), 1);
    assert_eq!(h.warnings(), [WarningReason::SwapGrowth]);
}

#[test]
fn thermal_rise_reduces_to_one_then_critical_pauses_everything() {
    let h = harness();
    let ids = four_running(&h);
    let hot = Signals {
        thermal: ThermalState::Serious,
        ..h.calm()
    };
    h.mgr.tick(hot);
    assert_eq!(h.cancels_sent(&ids[0]), 0, "oldest keeps running");
    for id in &ids[1..] {
        assert_eq!(h.cancels_sent(id), 1);
        h.exit(id, true, false);
    }
    assert_eq!(h.warnings(), [WarningReason::ThermalSerious]);
    // Staying hot neither repeats the warning nor touches the last job.
    h.advance(1000);
    h.mgr.tick(hot);
    assert_eq!(h.warnings().len(), 1);
    assert_eq!(h.cancels_sent(&ids[0]), 0);

    h.advance(1000);
    h.mgr.tick(Signals {
        thermal: ThermalState::Critical,
        ..h.calm()
    });
    assert_eq!(h.cancels_sent(&ids[0]), 1);
    h.exit(&ids[0], true, false);
    assert_eq!(
        h.warnings(),
        [
            WarningReason::ThermalSerious,
            WarningReason::ThermalCritical
        ]
    );
    h.assert_nothing_held();
}

#[test]
fn heat_and_low_battery_hold_new_jobs_back_with_a_reason() {
    let h = harness();
    h.mgr.tick(Signals {
        thermal: ThermalState::Serious,
        ..h.calm()
    });
    let id = h.add(1, "whisper-tiny").remove(0);
    let job = h.mgr.get(&id).unwrap();
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(
        job.wait_reason.as_deref(),
        Some(DenyReason::Thermal.message())
    );

    h.mgr.tick(Signals {
        battery_low: true,
        ..h.calm()
    });
    let job = h.mgr.get(&id).unwrap();
    assert_eq!(
        job.wait_reason.as_deref(),
        Some(DenyReason::Battery.message())
    );

    h.mgr.tick(h.calm());
    let job = h.mgr.get(&id).unwrap();
    assert_eq!(job.state, JobState::Starting);
    assert_eq!(job.wait_reason, None);
}

#[test]
fn a_full_machine_keeps_jobs_waiting_until_memory_frees() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 8);
    h.mgr.tick(Signals {
        available_ram: 2 * GIB, // under the safety reserve
        ..h.calm()
    });
    let id = h.add(1, "whisper-tiny").remove(0);
    let job = h.mgr.get(&id).unwrap();
    assert_eq!(
        job.wait_reason.as_deref(),
        Some(DenyReason::Memory.message())
    );
    h.mgr.tick(h.calm());
    assert_eq!(h.state(&id), JobState::Starting);
}

#[test]
fn jobs_still_loading_their_model_are_not_counted_as_memory_already_in_use() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 15);
    let job_mem =
        JobCost::for_model(catalog().get("whisper-tiny").unwrap(), Some(60_000), false).mem_bytes;
    // Room for exactly two jobs beyond the safety reserve.
    let signals = Signals {
        available_ram: safety_reserve(h.machine.total_ram) + 2 * job_mem,
        ..h.calm()
    };
    h.mgr.tick(signals);
    h.add(6, "whisper-tiny");
    assert_eq!(h.mgr.held_resources().0, 2);

    // Neither job has loaded yet, so live memory has not moved. However many
    // ticks pass, that must not make room for more.
    for _ in 0..5 {
        h.advance(1_000);
        h.mgr.tick(signals);
    }
    assert_eq!(h.mgr.held_resources().0, 2);

    // Once loaded, their memory is really gone from `available`, and the
    // reservation is what stands in for it. Still two.
    h.advance(MODEL_LOAD_SETTLE_MS);
    h.mgr.tick(Signals {
        available_ram: safety_reserve(h.machine.total_ram),
        ..h.calm()
    });
    assert_eq!(h.mgr.held_resources().0, 2);
}

#[test]
fn an_oom_kill_fails_the_job_and_lowers_the_learned_limit() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 3);
    let ids = h.add(3, "whisper-tiny");
    for id in &ids {
        h.ready(id);
    }
    h.exit(&ids[2], false, true);
    let job = h.mgr.get(&ids[2]).unwrap();
    assert_eq!(job.error_code.as_deref(), Some("OUT_OF_MEMORY"));
    assert_eq!(h.warnings(), [WarningReason::WorkerOutOfMemory]);

    // Three at once was too many: this model now runs two at a time.
    let more = h.add(1, "whisper-tiny").remove(0);
    assert_eq!(h.state(&more), JobState::Queued);
    assert_eq!(
        h.mgr.get(&more).unwrap().wait_reason.as_deref(),
        Some(DenyReason::LearnedLimit.message())
    );
    // Another model is not held back.
    h.mgr.set_priority(&more, Priority::Low).unwrap();
    let other = h.add(1, "whisper-base").remove(0);
    assert_eq!(h.state(&other), JobState::Starting);
}

#[test]
fn gpu_failure_retries_once_on_the_cpu() {
    let h = harness_with(Machine {
        total_ram: 32 * GIB,
        logical_cores: 16,
        gpu_slots: 1,
    });
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    let CoreToWorker::Start { limits, .. } = h.start_of(&id) else {
        panic!("not a start");
    };
    assert!(limits.use_gpu);
    let first = h.attempt(&id);

    h.send(
        &id,
        WorkerToCore::Error {
            error: ErrorInfo::from(OasError::GpuInitFailed { reason: "x".into() }),
        },
    );
    h.exit(&id, true, false);

    assert_ne!(h.attempt(&id), first, "a new worker");
    assert_eq!(h.state(&id), JobState::Starting);
    assert_eq!(h.warnings(), [WarningReason::GpuFallback]);
    h.ready(&id);
    let CoreToWorker::Start { limits, .. } = h.start_of(&id) else {
        panic!("not a start");
    };
    assert!(!limits.use_gpu);

    // The fallback is not repeated: a second failure is final.
    h.send(
        &id,
        WorkerToCore::Error {
            error: ErrorInfo::from(OasError::GpuInitFailed { reason: "x".into() }),
        },
    );
    h.exit(&id, true, false);
    assert_eq!(h.state(&id), JobState::Failed);
    h.assert_nothing_held();
}

#[test]
fn auto_mode_starts_at_one_and_grows_while_the_machine_has_room() {
    let h = harness();
    let ids = h.add(4, "whisper-tiny");
    assert_eq!(h.mgr.held_resources().0, 1, "starts at one job");
    h.advance(AutoConcurrency::RAISE_INTERVAL_MS);
    h.mgr.tick(h.calm());
    assert_eq!(h.mgr.held_resources().0, 2);
    h.advance(AutoConcurrency::RAISE_INTERVAL_MS);
    h.mgr.tick(Signals {
        cpu_total_pct: 95.0,
        ..h.calm()
    });
    assert_eq!(h.mgr.held_resources().0, 2, "a busy CPU holds it");
    assert_eq!(h.state(&ids[3]), JobState::Queued);
}

#[test]
fn the_cpu_budget_is_shared_so_later_jobs_are_not_blocked() {
    let h = harness();
    let ids = h.add(4, "whisper-tiny");
    h.ready(&ids[0]);
    let CoreToWorker::Start { limits, .. } = h.start_of(&ids[0]) else {
        panic!("not a start");
    };
    // 15 CPU budget split between 4 waiting jobs.
    assert_eq!(limits.max_threads, 3);
}

#[test]
fn status_reports_the_gauge_and_queue_eta() {
    let h = harness();
    assert_eq!(h.mgr.status().gauge, None);
    assert_eq!(h.mgr.status().queue_eta_secs, None);

    h.mgr.set_parallel(ParallelMode::Manual, 2);
    h.mgr.tick(h.calm());
    h.add(4, "whisper-tiny");
    let status = h.mgr.status();
    assert_eq!(status.mode, ParallelMode::Manual);
    assert_eq!((status.running, status.queued), (2, 2));
    assert_eq!(status.parallel_limit, 2);
    assert!(status.reserved_mem > 0);
    assert_eq!(status.gauge.unwrap().level, crate::GaugeLevel::Safe);
    // 4 × 60 s of audio × 0.5 RTF = 120 s of work, sped up by 2^0.8.
    let eta = status.queue_eta_secs.unwrap();
    assert!((eta - 120.0 / 2f64.powf(0.8)).abs() < 0.01, "{eta}");
}

#[test]
fn manual_mode_is_held_to_the_hard_cap() {
    let h = harness_with(Machine {
        total_ram: GIB,
        logical_cores: 16,
        gpu_slots: 0,
    });
    assert_eq!(h.machine.total_ram, GIB);
    h.mgr.set_parallel(ParallelMode::Manual, 500);
    let model = catalog();
    let cost = JobCost::for_model(model.get("whisper-tiny").unwrap(), Some(60_000), false);
    h.add(1, "whisper-tiny");
    assert_eq!(h.mgr.status().parallel_limit, hard_cap(GIB, cost.mem_bytes));
}

#[test]
fn export_renders_stored_segments() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.segment(&id, 0.0, 1.5, "hello");
    h.send(&id, WorkerToCore::Done);
    h.exit(&id, true, false);

    let out = h.dir.path().join("résumé out.srt");
    h.mgr.export(&id, "srt", &out).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("00:00:00,000 --> 00:00:01,500"));
    assert!(text.contains("hello"));
    assert_eq!(
        h.mgr.export(&id, "doc", &out).unwrap_err().code(),
        "INVALID_INPUT"
    );
}

// --- Restart recovery ------------------------------------------------------

#[test]
fn restart_recovers_every_kind_of_job() {
    let h = harness();
    h.mgr.set_parallel(ParallelMode::Manual, 2);
    let ids = h.add(6, "whisper-tiny");
    h.ready(&ids[0]); // running, with a partial transcript
    h.segment(&ids[0], 0.0, 1.0, "partial");
    // ids[1] is starting. Finish one, leave one queued, pause two.
    h.mgr.pause(&ids[3]).unwrap();
    h.mgr.pause(&ids[4]).unwrap();
    h.mgr.cancel(&ids[5]).unwrap();

    // Stop without any shutdown, as a crash would.
    let restarted = manager_on(
        Arc::clone(&h.db),
        h.dir.path(),
        Arc::clone(&h.clock),
        FakeSpawner::default(),
        Arc::new(Recorder::default()),
        h.machine,
    );
    let get = |id: &str| restarted.get(id).unwrap();
    for id in &ids[..2] {
        assert_eq!(get(id).state, JobState::Failed);
        assert_eq!(get(id).error_code.as_deref(), Some("JOB_INTERRUPTED"));
    }
    assert_eq!(get(&ids[2]).state, JobState::Queued);
    assert_eq!(
        get(&ids[3]).state,
        JobState::Paused,
        "the user's pause stands"
    );
    assert_eq!(get(&ids[5]).state, JobState::Cancelled);
    assert_eq!(restarted.held_resources(), (0, 0, 0));
    assert_eq!(
        restarted.segments(&ids[0]).unwrap().len(),
        1,
        "partial kept"
    );

    restarted.start();
    assert_eq!(get(&ids[2]).state, JobState::Starting);
    restarted.retry(&ids[0]).unwrap();
    assert_eq!(get(&ids[0]).retries, 1);
}

#[test]
fn restart_requeues_jobs_the_governor_paused() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    h.ready(&id);
    h.advance(WATCHDOG_COOLDOWN_MS);
    h.mgr.tick(Signals {
        available_ram: 1,
        ..h.calm()
    });
    h.exit(&id, true, false);
    assert_eq!(h.state(&id), JobState::Paused);

    let restarted = manager_on(
        Arc::clone(&h.db),
        h.dir.path(),
        Arc::clone(&h.clock),
        FakeSpawner::default(),
        Arc::new(Recorder::default()),
        h.machine,
    );
    assert_eq!(restarted.get(&id).unwrap().state, JobState::Queued);
}

#[test]
fn a_stored_state_this_build_does_not_know_becomes_a_failed_job() {
    let h = harness();
    let id = h.add(1, "whisper-tiny").remove(0);
    let mut row = h.db.list_jobs().unwrap().remove(0);
    row.state = "from_the_future".into();
    h.db.save_job(&row).unwrap();
    let restarted = manager_on(
        Arc::clone(&h.db),
        h.dir.path(),
        Arc::clone(&h.clock),
        FakeSpawner::default(),
        Arc::new(Recorder::default()),
        h.machine,
    );
    let job = restarted.get(&id).unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error_code.as_deref(), Some("INTERNAL_ERROR"));
}

// --- Any sequence of events must leave the books balanced ------------------

#[derive(Debug, Clone)]
enum Op {
    Add,
    Ready(usize),
    Done(usize),
    Crash(usize),
    Cleanly(usize),
    Pause(usize),
    Resume(usize),
    Cancel(usize),
    Retry(usize),
    Delete(usize),
    Tick(u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Add),
        (0usize..12).prop_map(Op::Ready),
        (0usize..12).prop_map(Op::Done),
        (0usize..12).prop_map(Op::Crash),
        (0usize..12).prop_map(Op::Cleanly),
        (0usize..12).prop_map(Op::Pause),
        (0usize..12).prop_map(Op::Resume),
        (0usize..12).prop_map(Op::Cancel),
        (0usize..12).prop_map(Op::Retry),
        (0usize..12).prop_map(Op::Delete),
        (0u8..4).prop_map(Op::Tick),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn no_event_order_leaks_resources_or_breaks_the_limit(
        ops in proptest::collection::vec(op(), 1..80)
    ) {
        let h = harness();
        h.mgr.set_parallel(ParallelMode::Manual, 3);
        let mut ids: Vec<String> = h.add(3, "whisper-tiny");
        let pick = |ids: &[String], i: usize| ids.get(i % ids.len().max(1)).cloned();
        let live = |h: &Harness, id: &str| {
            h.mgr.get(id).map(|j| j.state.is_active()).unwrap_or(false)
        };

        for op in ops {
            match op {
                Op::Add => ids.extend(h.add(1, "whisper-tiny")),
                Op::Ready(i) => if let Some(id) = pick(&ids, i) {
                    if live(&h, &id) { h.ready(&id); }
                },
                Op::Done(i) => if let Some(id) = pick(&ids, i) {
                    if live(&h, &id) {
                        h.send(&id, WorkerToCore::Done);
                        h.exit(&id, true, false);
                    }
                },
                Op::Crash(i) => if let Some(id) = pick(&ids, i) {
                    if live(&h, &id) { h.exit(&id, false, i % 2 == 0); }
                },
                Op::Cleanly(i) => if let Some(id) = pick(&ids, i) {
                    if live(&h, &id) { h.exit(&id, true, false); }
                },
                Op::Pause(i) => if let Some(id) = pick(&ids, i) { let _ = h.mgr.pause(&id); },
                Op::Resume(i) => if let Some(id) = pick(&ids, i) { let _ = h.mgr.resume(&id); },
                Op::Cancel(i) => if let Some(id) = pick(&ids, i) { let _ = h.mgr.cancel(&id); },
                Op::Retry(i) => if let Some(id) = pick(&ids, i) { let _ = h.mgr.retry(&id); },
                Op::Delete(i) => if let Some(id) = pick(&ids, i) { let _ = h.mgr.delete(&id); },
                Op::Tick(kind) => {
                    h.advance(5_000);
                    let signals = match kind {
                        0 => Signals { available_ram: 1, ..h.calm() },
                        1 => Signals { thermal: ThermalState::Critical, ..h.calm() },
                        _ => h.calm(),
                    };
                    h.mgr.tick(signals);
                }
            }
            let (workers, mem, _) = h.mgr.held_resources();
            prop_assert!(workers <= 3, "limit broken: {workers}");
            prop_assert_eq!(mem == 0, workers == 0);
        }

        // Every worker that is still going ends, one way or another.
        for id in h.mgr.list().into_iter().map(|j| j.id) {
            if live(&h, &id) { h.exit(&id, false, false); }
        }
        // Whatever those exits started in turn ends too.
        loop {
            let live_ids: Vec<String> = h.mgr.list().into_iter()
                .filter(|j| j.state.is_active())
                .map(|j| j.id)
                .collect();
            if live_ids.is_empty() { break; }
            for id in live_ids { h.exit(&id, false, false); }
        }
        prop_assert_eq!(h.mgr.held_resources(), (0, 0, 0));
    }
}
