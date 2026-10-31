//! The job manager: owns the queue, starts workers through the governor, and
//! reacts to worker events and live machine signals.
//!
//! It is one state machine behind one lock, and every entry point runs to
//! completion: a command, a worker event, or a tick changes the state and then
//! asks the scheduler to start what it can. Time, workers, and event delivery
//! are injected, so all of this runs deterministically in tests.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use oas_catalog::{Catalog, ModelEntry};
use oas_core::error::ErrorInfo;
use oas_core::{JobId, OasError, Paths, Result};
use oas_hardware::ThermalState;
use oas_protocol::{CoreToWorker, EngineKind, JobOptions, Limits, WorkerToCore};
use oas_storage::{Database, JobRow, SegmentRow};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::governor::{
    gauge, hard_cap, safety_reserve, AutoConcurrency, Budgets, DenyReason, Gauge, Governor,
    JobCost, Reservation,
};
use crate::schedule::{pick_next, queue_eta_secs};
use crate::watchdog::{decide, WarningReason, WatchdogAction, WatchdogConfig};
use crate::worker::{
    WorkerEvent, WorkerEventKind, WorkerExit, WorkerHandle, WorkerSink, WorkerSpawner,
};
use crate::{Job, JobState, PauseOrigin, Priority};

/// Time a worker gets to stop by itself before it is killed.
const STOP_GRACE_MS: u64 = 2_000;
/// Time a worker gets to report `ready`.
const START_TIMEOUT_MS: u64 = 30_000;
/// Fastest rate for `job:progress` and `job:segment` per job (4 per second).
const EMIT_INTERVAL_MS: u64 = 250;
/// Window for measuring swap growth.
const SWAP_WINDOW_MS: u64 = 10_000;
/// Time between protective pauses, so one pause can take effect before the next
/// is judged necessary, and between resumes after recovery.
const WATCHDOG_COOLDOWN_MS: u64 = 15_000;

/// Time after which a running job's memory is assumed to show in live usage.
/// Until then its reservation counts on top of live usage, not within it.
const MODEL_LOAD_SETTLE_MS: u64 = 10_000;

/// Shorter clips are dominated by start-up cost, and would mislead calibration.
const MIN_MEASURED_AUDIO_MS: u64 = 5_000;

/// A source of the current time.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// A transcript line as sent to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct JobSegment {
    pub idx: u32,
    #[specta(type = f64)]
    pub start_ms: u64,
    #[specta(type = f64)]
    pub end_ms: u64,
    pub text: String,
}

/// Live progress for a running job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct JobProgress {
    pub job_id: String,
    pub progress: f32,
    pub stage: String,
    pub eta_secs: Option<f64>,
}

/// Something the governor did to protect the machine, or wants the user to know.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct GovernorWarning {
    pub reason: WarningReason,
    pub message: String,
    /// The jobs it affected.
    pub job_ids: Vec<String>,
    #[specta(type = f64)]
    pub at_ms: u64,
}

/// Receives everything the UI needs to know.
pub trait JobEvents: Send + Sync {
    fn job_state(&self, job: &Job);
    fn job_removed(&self, job_id: &str);
    fn job_progress(&self, progress: &JobProgress);
    fn job_segments(&self, job_id: &str, segments: &[JobSegment]);
    fn governor_warning(&self, warning: &GovernorWarning);
    /// A whole file was processed in one run on the given device, so `rtf` is a
    /// clean measurement of this model on this machine.
    fn job_measured(&self, _model_id: &str, _rtf: f32, _used_gpu: bool) {}
}

/// The machine, as far as the governor cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Machine {
    pub total_ram: u64,
    pub logical_cores: u32,
    /// GPU jobs allowed at once (0 until GPU workers exist).
    pub gpu_slots: u32,
}

/// One live sample of the machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
    pub available_ram: u64,
    pub swap_used: u64,
    pub thermal: ThermalState,
    pub cpu_total_pct: f32,
    /// The "pause queue on battery below N%" policy is in force.
    pub battery_low: bool,
}

/// Auto (adaptive) or Manual (fixed) concurrency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ParallelMode {
    Auto,
    Manual,
}

/// A request to add files to the queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct JobRequest {
    pub paths: Vec<String>,
    pub model_id: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub translate: bool,
    #[serde(default)]
    pub word_timestamps: bool,
    #[serde(default)]
    pub priority: Priority,
}

/// Everything the "Parallel jobs" control and queue header show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct GovernorStatus {
    pub mode: ParallelMode,
    /// The current limit on jobs at once.
    pub parallel_limit: u32,
    pub running: u32,
    pub queued: u32,
    pub paused: u32,
    #[specta(type = f64)]
    pub mem_budget: u64,
    #[specta(type = f64)]
    pub reserved_mem: u64,
    pub cpu_budget: u32,
    pub reserved_cpu: u32,
    /// The gauge for the current limit, sized by the next job's model. `None`
    /// when no job is waiting or running.
    pub gauge: Option<Gauge>,
    /// Estimated seconds until the queue is finished. `None` if nothing is left
    /// or the audio length of a job is not known yet.
    pub queue_eta_secs: Option<f64>,
}

/// What the manager needs from the app.
pub struct ManagerDeps {
    pub db: Arc<Database>,
    pub catalog: Catalog,
    pub paths: Paths,
    pub clock: Arc<dyn Clock>,
    pub spawner: Box<dyn WorkerSpawner>,
    pub events: Arc<dyn JobEvents>,
    pub machine: Machine,
    /// Estimated processing seconds per audio second for a model.
    pub rtf_estimate: Box<dyn Fn(&str) -> f32 + Send>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Cancel,
    Pause(PauseOrigin),
}

/// A job with a live worker.
struct Active {
    attempt: u64,
    handle: Box<dyn WorkerHandle>,
    reservation: Reservation,
    /// Order of starting; the highest is the newest.
    seq: u64,
    use_gpu: bool,
    start_sent: bool,
    spawned_at_ms: u64,
    stop: Option<Stop>,
    stop_deadline_ms: Option<u64>,
    killed_for_timeout: bool,
    /// This run continues a paused job, so it did not cover the whole file.
    resumed: bool,
    done: bool,
    error: Option<ErrorInfo>,
    last_progress_emit_ms: u64,
    last_segment_emit_ms: u64,
    pending_segments: Vec<JobSegment>,
}

struct Core {
    db: Arc<Database>,
    catalog: Catalog,
    paths: Paths,
    clock: Arc<dyn Clock>,
    spawner: Box<dyn WorkerSpawner>,
    events: Arc<dyn JobEvents>,
    machine: Machine,
    rtf_estimate: Box<dyn Fn(&str) -> f32 + Send>,
    /// Delivers worker events back here. Set once, right after construction.
    sink: Option<WorkerSink>,

    jobs: HashMap<String, Job>,
    active: HashMap<String, Active>,
    /// Stored segment count and end of the last one, per job.
    transcript_ends: HashMap<String, (u32, u64)>,
    governor: Governor,
    watchdog: WatchdogConfig,
    mode: ParallelMode,
    manual_max: u32,
    auto: AutoConcurrency,
    /// Jobs that must not use the GPU again (it failed to start for them).
    force_cpu: HashSet<String>,
    /// Deleted while running; removed when the worker has gone.
    delete_on_exit: HashSet<String>,
    /// Most jobs of a model that were safe, learned from out-of-memory kills.
    /// Held for this session only.
    learned_caps: HashMap<String, u32>,
    swap_history: VecDeque<(u64, u64)>,
    signals: Signals,
    last_protective_pause_ms: u64,
    last_auto_resume_ms: u64,
    next_attempt: u64,
    next_seq: u64,
    next_rank: i64,
}

/// A thread-safe handle to the job manager.
pub struct JobManager {
    core: Arc<Mutex<Core>>,
}

fn lock(core: &Mutex<Core>) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(|e| e.into_inner())
}

fn parse_options(json: &str) -> JobOptions {
    serde_json::from_str(json).unwrap_or_default()
}

/// A job from its stored row. A row this build cannot understand becomes a
/// failed job rather than being lost or stopping the app.
fn job_from_row(row: JobRow) -> Job {
    let options = parse_options(&row.options_json);
    let state = JobState::parse(&row.state);
    let (error_code, error_message) = match state {
        Some(_) => (row.error_code, row.error_message),
        None => (
            Some("INTERNAL_ERROR".to_string()),
            Some(format!("Unknown stored job state: {}", row.state)),
        ),
    };
    let metrics: Option<serde_json::Value> = row
        .metrics_json
        .as_deref()
        .and_then(|m| serde_json::from_str(m).ok());
    Job {
        id: row.id,
        model_id: row.model_id,
        input_path: row.input_path,
        duration_ms: row.input_duration_ms,
        priority: Priority::parse(&row.priority),
        rank: row.rank,
        state: state.unwrap_or(JobState::Failed),
        pause_origin: row.pause_origin.as_deref().and_then(PauseOrigin::parse),
        wait_reason: None,
        progress: row.progress,
        language: options.language,
        translate: options.translate,
        word_timestamps: options.word_timestamps,
        created_at_ms: row.created_at_ms,
        started_at_ms: row.started_at_ms,
        finished_at_ms: row.finished_at_ms,
        error_code,
        error_message,
        retries: row.retries,
        rtf: metrics
            .as_ref()
            .and_then(|m| m.get("rtf"))
            .and_then(|v| v.as_f64())
            .map(|v| v as f32),
    }
}

fn job_to_row(job: &Job) -> JobRow {
    let options = JobOptions {
        language: job.language.clone(),
        translate: job.translate,
        word_timestamps: job.word_timestamps,
    };
    JobRow {
        id: job.id.clone(),
        input_path: job.input_path.clone(),
        input_duration_ms: job.duration_ms,
        model_id: job.model_id.clone(),
        options_json: serde_json::to_string(&options).unwrap_or_else(|_| "{}".into()),
        priority: job.priority.as_str().to_string(),
        rank: job.rank,
        state: job.state.as_str().to_string(),
        pause_origin: job.pause_origin.map(|o| o.as_str().to_string()),
        progress: job.progress,
        retries: job.retries,
        error_code: job.error_code.clone(),
        error_message: job.error_message.clone(),
        metrics_json: job
            .rtf
            .map(|rtf| serde_json::json!({ "rtf": rtf }).to_string()),
        created_at_ms: job.created_at_ms,
        started_at_ms: job.started_at_ms,
        finished_at_ms: job.finished_at_ms,
    }
}

impl JobManager {
    /// Load the queue and recover from whatever the last run left behind: jobs
    /// that were running become failed (interrupted), and jobs the governor
    /// had paused go back in the queue.
    pub fn new(deps: ManagerDeps) -> Result<Self> {
        let now = deps.clock.now_ms();
        let budgets = Budgets::new(
            deps.machine.total_ram,
            deps.machine.total_ram,
            deps.machine.logical_cores,
            deps.machine.gpu_slots,
        );
        let mut core = Core {
            db: deps.db,
            catalog: deps.catalog,
            paths: deps.paths,
            clock: deps.clock,
            spawner: deps.spawner,
            events: deps.events,
            machine: deps.machine,
            rtf_estimate: deps.rtf_estimate,
            sink: None,
            jobs: HashMap::new(),
            active: HashMap::new(),
            transcript_ends: HashMap::new(),
            governor: Governor::new(budgets),
            watchdog: WatchdogConfig {
                safety_reserve: safety_reserve(deps.machine.total_ram),
                ..WatchdogConfig::default()
            },
            mode: ParallelMode::Auto,
            manual_max: 1,
            auto: AutoConcurrency::new(),
            force_cpu: HashSet::new(),
            delete_on_exit: HashSet::new(),
            learned_caps: HashMap::new(),
            swap_history: VecDeque::new(),
            signals: Signals {
                available_ram: deps.machine.total_ram,
                swap_used: 0,
                thermal: ThermalState::Unknown,
                cpu_total_pct: 0.0,
                battery_low: false,
            },
            last_protective_pause_ms: 0,
            last_auto_resume_ms: 0,
            next_attempt: 1,
            next_seq: 1,
            next_rank: 1,
        };

        for row in core.db.list_jobs()? {
            let mut job = job_from_row(row);
            core.next_rank = core.next_rank.max(job.rank + 1);
            if job.state.is_active() {
                job.state = JobState::Failed;
                job.finished_at_ms = Some(now);
                let info = ErrorInfo::from(OasError::JobInterrupted);
                job.error_code = Some(info.code);
                job.error_message = Some(info.message);
                core.db.save_job(&job_to_row(&job))?;
            } else if job.state == JobState::Paused
                && job.pause_origin == Some(PauseOrigin::Governor)
            {
                job.state = JobState::Queued;
                job.pause_origin = None;
                core.db.save_job(&job_to_row(&job))?;
            }
            core.jobs.insert(job.id.clone(), job);
        }

        let core = Arc::new(Mutex::new(core));
        let weak: Weak<Mutex<Core>> = Arc::downgrade(&core);
        let sink: WorkerSink = Arc::new(move |event: WorkerEvent| {
            // After shutdown there is nothing left to tell.
            if let Some(core) = weak.upgrade() {
                lock(&core).on_worker_event(event);
            }
        });
        lock(&core).sink = Some(sink);
        Ok(JobManager { core })
    }

    fn core(&self) -> MutexGuard<'_, Core> {
        lock(&self.core)
    }

    /// Start whatever the governor allows. Called once after startup.
    pub fn start(&self) {
        self.core().try_start_next();
    }

    /// Queue one job per file. All-or-nothing: a bad request adds no jobs.
    /// `durations_ms` holds each file's length, when it could be read.
    pub fn create(&self, request: JobRequest, durations_ms: Vec<Option<u64>>) -> Result<Vec<Job>> {
        self.core().create(request, durations_ms)
    }

    pub fn list(&self) -> Vec<Job> {
        let core = self.core();
        let mut jobs: Vec<Job> = core.jobs.values().cloned().collect();
        jobs.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then(a.rank.cmp(&b.rank))
                .then(a.id.cmp(&b.id))
        });
        jobs
    }

    pub fn get(&self, id: &str) -> Result<Job> {
        self.core().job(id).cloned()
    }

    pub fn segments(&self, id: &str) -> Result<Vec<JobSegment>> {
        Ok(self
            .core()
            .db
            .list_segments(id)?
            .into_iter()
            .map(|s| JobSegment {
                idx: s.idx,
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text,
            })
            .collect())
    }

    pub fn pause(&self, id: &str) -> Result<()> {
        let mut core = self.core();
        core.pause(id, PauseOrigin::User)?;
        core.try_start_next();
        Ok(())
    }

    pub fn resume(&self, id: &str) -> Result<()> {
        let mut core = self.core();
        core.resume(id)?;
        core.try_start_next();
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> Result<()> {
        let mut core = self.core();
        core.cancel(id)?;
        core.try_start_next();
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut core = self.core();
        core.delete(id)?;
        core.try_start_next();
        Ok(())
    }

    pub fn retry(&self, id: &str) -> Result<()> {
        let mut core = self.core();
        core.retry(id)?;
        core.try_start_next();
        Ok(())
    }

    pub fn set_priority(&self, id: &str, priority: Priority) -> Result<()> {
        let mut core = self.core();
        let job = core.job_mut(id)?;
        job.priority = priority;
        core.changed(id);
        core.try_start_next();
        Ok(())
    }

    /// Put the listed queued jobs in this order, keeping the queue slots they
    /// already occupy, so unlisted jobs are not moved.
    pub fn reorder(&self, ids: &[String]) -> Result<()> {
        let mut core = self.core();
        let mut ranks = Vec::with_capacity(ids.len());
        for id in ids {
            ranks.push(core.job(id)?.rank);
        }
        ranks.sort_unstable();
        for (id, rank) in ids.iter().zip(ranks) {
            core.job_mut(id)?.rank = rank;
            core.changed(id);
        }
        core.try_start_next();
        Ok(())
    }

    pub fn pause_all(&self) {
        let mut core = self.core();
        let ids: Vec<String> = core
            .jobs
            .values()
            .filter(|j| j.state == JobState::Queued || j.state.is_active())
            .map(|j| j.id.clone())
            .collect();
        for id in ids {
            if let Err(e) = core.pause(&id, PauseOrigin::User) {
                tracing::warn!(error = %e, "could not pause a job");
            }
        }
    }

    pub fn resume_all(&self) {
        let mut core = self.core();
        let ids: Vec<String> = core
            .jobs
            .values()
            .filter(|j| j.state == JobState::Paused)
            .map(|j| j.id.clone())
            .collect();
        for id in ids {
            if let Err(e) = core.resume(&id) {
                tracing::warn!(error = %e, "could not resume a job");
            }
        }
        core.try_start_next();
    }

    /// Correct a transcript line. Exports use the corrected text.
    pub fn update_segment(&self, id: &str, idx: u32, text: &str) -> Result<()> {
        let core = self.core();
        core.job(id)?;
        core.db.update_segment_text(id, idx, text)
    }

    /// Limit threads per job from now on.
    pub fn set_thread_cap(&self, cap: u32) {
        self.core().governor.set_thread_cap(cap);
    }

    /// Delete jobs that finished more than `days` days ago. 0 keeps everything.
    /// Returns how many were removed.
    pub fn purge_finished_older_than(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let mut core = self.core();
        let cutoff = core.now().saturating_sub(days as u64 * 86_400_000);
        let old: Vec<String> = core
            .jobs
            .values()
            .filter(|j| j.state.is_terminal() && j.finished_at_ms.is_some_and(|t| t < cutoff))
            .map(|j| j.id.clone())
            .collect();
        for id in &old {
            core.delete(id)?;
        }
        Ok(old.len())
    }

    /// Delete every completed job.
    pub fn clear_completed(&self) -> Result<()> {
        let mut core = self.core();
        let ids: Vec<String> = core
            .jobs
            .values()
            .filter(|j| j.state == JobState::Completed)
            .map(|j| j.id.clone())
            .collect();
        for id in ids {
            core.delete(&id)?;
        }
        Ok(())
    }

    /// A job's transcript rendered as `format` (txt, srt, vtt, json).
    pub fn render_transcript(&self, id: &str, format: &str) -> Result<String> {
        let format =
            oas_export::Format::from_str_opt(format).ok_or_else(|| OasError::InvalidInput {
                reason: format!("unknown export format: {format}"),
            })?;
        let segments: Vec<oas_engine::Segment> = self
            .segments(id)?
            .into_iter()
            .map(|s| oas_engine::Segment {
                start: s.start_ms as f64 / 1000.0,
                end: s.end_ms as f64 / 1000.0,
                text: s.text,
                words: vec![],
            })
            .collect();
        oas_export::export(&segments, format)
    }

    /// Write a job's transcript to `path`, replacing it only once complete.
    pub fn export(&self, id: &str, format: &str, path: &Path) -> Result<()> {
        let rendered = self.render_transcript(id, format)?;
        let mut temp = path.as_os_str().to_owned();
        temp.push(".part");
        std::fs::write(&temp, rendered)?;
        std::fs::rename(&temp, path)?;
        Ok(())
    }

    /// Choose Auto or Manual. `manual_max` is limited by the hard cap for the
    /// next job's model when one is known.
    pub fn set_parallel(&self, mode: ParallelMode, manual_max: u32) {
        let mut core = self.core();
        core.mode = mode;
        core.manual_max = manual_max.max(1);
        core.try_start_next();
    }

    pub fn status(&self) -> GovernorStatus {
        self.core().status()
    }

    /// The gauge for running `desired` jobs of `model_id` right now.
    pub fn gauge_for(&self, model_id: &str, desired: u32) -> Result<Gauge> {
        let core = self.core();
        let model = core.model(model_id)?;
        let mem = JobCost::for_model(model, None, false).mem_bytes;
        Ok(gauge(desired, core.governor.budgets(), mem))
    }

    /// A worker event. Used directly by tests; real workers arrive via the sink.
    pub fn on_worker_event(&self, event: WorkerEvent) {
        self.core().on_worker_event(event);
    }

    /// The periodic step (about 1 Hz): new machine signals, stop deadlines,
    /// protection, Auto mode, and starting what now fits.
    pub fn tick(&self, signals: Signals) {
        self.core().tick(signals);
    }

    /// Workers currently running, and memory and CPU held. All zero when idle:
    /// this is the leak check.
    pub fn held_resources(&self) -> (usize, u64, u32) {
        let core = self.core();
        (
            core.active.len(),
            core.governor.reserved_mem(),
            core.governor.reserved_cpu(),
        )
    }
}

impl Core {
    fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    fn job(&self, id: &str) -> Result<&Job> {
        self.jobs.get(id).ok_or_else(|| OasError::InvalidInput {
            reason: format!("no such job: {id}"),
        })
    }

    fn job_mut(&mut self, id: &str) -> Result<&mut Job> {
        self.jobs.get_mut(id).ok_or_else(|| OasError::InvalidInput {
            reason: format!("no such job: {id}"),
        })
    }

    fn model(&self, id: &str) -> Result<&ModelEntry> {
        self.catalog
            .get(id)
            .ok_or_else(|| OasError::ModelNotInstalled {
                model_id: id.to_string(),
            })
    }

    /// Persist a job and tell the UI. A failed write is logged, not fatal: the
    /// job carries on in memory.
    fn changed(&self, id: &str) {
        if let Some(job) = self.jobs.get(id) {
            if let Err(e) = self.db.save_job(&job_to_row(job)) {
                tracing::error!(error = %e, job = id, "could not save a job");
            }
            self.events.job_state(job);
        }
    }

    fn warn(&self, reason: WarningReason, job_ids: Vec<String>) {
        self.events.governor_warning(&GovernorWarning {
            reason,
            message: reason.message().to_string(),
            job_ids,
            at_ms: self.now(),
        });
    }

    fn create(&mut self, request: JobRequest, durations_ms: Vec<Option<u64>>) -> Result<Vec<Job>> {
        if request.paths.is_empty() {
            return Err(OasError::InvalidInput {
                reason: "no files were given".into(),
            });
        }
        self.model(&request.model_id)?;
        if !self.db.is_installed(&request.model_id)? {
            return Err(OasError::ModelNotInstalled {
                model_id: request.model_id,
            });
        }
        for path in &request.paths {
            if !Path::new(path).is_file() {
                return Err(OasError::FileNotFound { path: path.clone() });
            }
        }

        let now = self.now();
        let mut created = Vec::with_capacity(request.paths.len());
        for (i, path) in request.paths.iter().enumerate() {
            let job = Job {
                id: JobId::new().to_string(),
                model_id: request.model_id.clone(),
                input_path: path.clone(),
                duration_ms: durations_ms.get(i).copied().flatten(),
                priority: request.priority,
                rank: self.next_rank,
                state: JobState::Queued,
                pause_origin: None,
                wait_reason: None,
                progress: 0.0,
                language: request.language.clone(),
                translate: request.translate,
                word_timestamps: request.word_timestamps,
                created_at_ms: now,
                started_at_ms: None,
                finished_at_ms: None,
                error_code: None,
                error_message: None,
                retries: 0,
                rtf: None,
            };
            self.next_rank += 1;
            self.db.save_job(&job_to_row(&job))?;
            self.events.job_state(&job);
            self.jobs.insert(job.id.clone(), job.clone());
            created.push(job);
        }
        self.try_start_next();
        Ok(created)
    }

    fn pause(&mut self, id: &str, origin: PauseOrigin) -> Result<()> {
        let state = self.job(id)?.state;
        if state == JobState::Queued {
            let job = self.job_mut(id)?;
            job.state = JobState::Paused;
            job.pause_origin = Some(origin);
            job.wait_reason = None;
            self.changed(id);
            Ok(())
        } else if state.is_active() {
            self.request_stop(id, Stop::Pause(origin));
            Ok(())
        } else if state == JobState::Paused {
            Ok(())
        } else {
            Err(OasError::InvalidInput {
                reason: "this job has already finished".into(),
            })
        }
    }

    fn resume(&mut self, id: &str) -> Result<()> {
        let job = self.job_mut(id)?;
        if !job.try_transition(JobState::Queued) || job.finished_at_ms.is_some() {
            return Err(OasError::InvalidInput {
                reason: "only a paused job can be resumed".into(),
            });
        }
        job.pause_origin = None;
        self.changed(id);
        Ok(())
    }

    fn cancel(&mut self, id: &str) -> Result<()> {
        let now = self.now();
        let state = self.job(id)?.state;
        if state.is_active() {
            self.request_stop(id, Stop::Cancel);
            return Ok(());
        }
        let job = self.job_mut(id)?;
        if !job.try_transition(JobState::Cancelled) {
            return Err(OasError::InvalidInput {
                reason: "this job has already finished".into(),
            });
        }
        job.pause_origin = None;
        job.wait_reason = None;
        job.finished_at_ms = Some(now);
        self.changed(id);
        Ok(())
    }

    fn delete(&mut self, id: &str) -> Result<()> {
        self.job(id)?;
        if self.active.contains_key(id) {
            self.delete_on_exit.insert(id.to_string());
            self.request_stop(id, Stop::Cancel);
            return Ok(());
        }
        self.db.delete_job(id)?;
        self.jobs.remove(id);
        self.transcript_ends.remove(id);
        self.force_cpu.remove(id);
        self.events.job_removed(id);
        Ok(())
    }

    fn retry(&mut self, id: &str) -> Result<()> {
        let job = self.job_mut(id)?;
        if !matches!(job.state, JobState::Failed | JobState::Cancelled)
            || !job.try_transition(JobState::Queued)
        {
            return Err(OasError::InvalidInput {
                reason: "only a failed or cancelled job can be retried".into(),
            });
        }
        job.retries += 1;
        job.progress = 0.0;
        job.error_code = None;
        job.error_message = None;
        job.started_at_ms = None;
        job.finished_at_ms = None;
        job.rtf = None;
        self.db.clear_segments(id)?;
        self.transcript_ends.remove(id);
        self.changed(id);
        Ok(())
    }

    /// Ask a live worker to stop. A worker that has not been given its job yet
    /// has nothing to wind down, so it is simply killed.
    fn request_stop(&mut self, id: &str, stop: Stop) {
        let deadline = self.now() + STOP_GRACE_MS;
        let Some(active) = self.active.get_mut(id) else {
            return;
        };
        // Cancel overrides a pause already in progress, never the other way.
        if active.stop == Some(Stop::Cancel) {
            return;
        }
        active.stop = Some(stop);
        if !active.start_sent {
            active.handle.kill();
            return;
        }
        if active.stop_deadline_ms.is_none() {
            active.stop_deadline_ms = Some(deadline);
            if let Err(e) = active.handle.send(&CoreToWorker::Cancel) {
                tracing::warn!(error = %e, "could not ask a worker to stop; killing it");
                active.handle.kill();
            }
        }
    }

    /// The most jobs that may run now.
    fn parallel_limit(&self) -> u32 {
        match self.mode {
            ParallelMode::Auto => self.auto.target(),
            ParallelMode::Manual => {
                let cap = self
                    .next_job_cost()
                    .map_or(u32::MAX, |c| hard_cap(self.machine.total_ram, c.mem_bytes));
                self.manual_max.min(cap)
            }
        }
    }

    fn cost_of(&self, job: &Job) -> Option<JobCost> {
        let model = self.catalog.get(&job.model_id)?;
        Some(JobCost::for_model(
            model,
            job.duration_ms,
            self.gpu_for(job, model),
        ))
    }

    fn gpu_for(&self, job: &Job, model: &ModelEntry) -> bool {
        self.machine.gpu_slots > 0 && model.gpu_capable && !self.force_cpu.contains(&job.id)
    }

    fn next_job_id(&self) -> Option<String> {
        let candidates: Vec<(String, Priority, i64)> = self
            .jobs
            .values()
            .filter(|j| j.state == JobState::Queued)
            .map(|j| (j.id.clone(), j.priority, j.rank))
            .collect();
        pick_next(&candidates)
    }

    /// The memory-sizing job: next in line, or else any running job.
    fn next_job_cost(&self) -> Option<JobCost> {
        let job = self
            .next_job_id()
            .and_then(|id| self.jobs.get(&id))
            .or_else(|| {
                self.active
                    .keys()
                    .next()
                    .and_then(|id| self.jobs.get(id.as_str()))
            })?;
        self.cost_of(job)
    }

    fn running_with_model(&self, model_id: &str) -> u32 {
        self.active
            .keys()
            .filter_map(|id| self.jobs.get(id.as_str()))
            .filter(|j| j.model_id == model_id)
            .count() as u32
    }

    /// Jobs the CPU budget should be divided between: all the work there is,
    /// up to what could run at once. Depends on the queue, not the Auto target,
    /// so a first job does not take every core and block the jobs behind it.
    fn thread_split(&self, per_job_mem: u64) -> u32 {
        let waiting = self
            .jobs
            .values()
            .filter(|j| j.state == JobState::Queued || j.state.is_active())
            .count() as u32;
        let ceiling = match self.mode {
            ParallelMode::Auto => self.governor.max_concurrency(per_job_mem),
            ParallelMode::Manual => self.parallel_limit(),
        };
        waiting.min(ceiling).max(1)
    }

    fn set_wait_reason(&mut self, id: &str, reason: Option<DenyReason>) {
        let text = reason.map(|r| r.message().to_string());
        let Some(job) = self.jobs.get_mut(id) else {
            return;
        };
        if job.wait_reason != text {
            job.wait_reason = text;
            if let Some(job) = self.jobs.get(id) {
                self.events.job_state(job);
            }
        }
    }

    /// Start jobs in order until the next one has to wait. Strict order: a
    /// small job behind a big one waits too, so the big one cannot starve.
    fn try_start_next(&mut self) {
        while let Some(id) = self.next_job_id() {
            if self.active.len() as u32 >= self.parallel_limit() {
                return;
            }
            let Some(job) = self.jobs.get(&id).cloned() else {
                return;
            };
            if let Some(cap) = self.learned_caps.get(&job.model_id) {
                if self.running_with_model(&job.model_id) >= *cap {
                    self.set_wait_reason(&id, Some(DenyReason::LearnedLimit));
                    return;
                }
            }
            let Some(cost) = self.cost_of(&job) else {
                self.fail_before_start(
                    &id,
                    &OasError::ModelNotInstalled {
                        model_id: job.model_id.clone(),
                    },
                );
                continue;
            };
            let split = self.thread_split(cost.mem_bytes);
            let signals = self.signals;
            match self
                .governor
                .admit(cost, split, signals.thermal, signals.battery_low)
            {
                Ok(reservation) => self.start_job(&id, reservation),
                Err(reason) => {
                    self.set_wait_reason(&id, Some(reason));
                    return;
                }
            }
        }
    }

    fn fail_before_start(&mut self, id: &str, error: &OasError) {
        let now = self.now();
        if let Ok(job) = self.job_mut(id) {
            job.state = JobState::Failed;
            job.finished_at_ms = Some(now);
            job.wait_reason = None;
            job.error_code = Some(error.code().to_string());
            job.error_message = Some(error.user_message());
        }
        self.changed(id);
    }

    fn start_job(&mut self, id: &str, reservation: Reservation) {
        let Some(sink) = self.sink.clone() else {
            self.governor.release(&reservation);
            return;
        };
        let attempt = self.next_attempt;
        self.next_attempt += 1;
        let handle = match self.spawner.spawn(id, attempt, sink) {
            Ok(handle) => handle,
            Err(e) => {
                self.governor.release(&reservation);
                self.fail_before_start(id, &e);
                return;
            }
        };

        let now = self.now();
        let seq = self.next_seq;
        self.next_seq += 1;
        if let Ok(job) = self.job_mut(id) {
            job.try_transition(JobState::Starting);
            job.wait_reason = None;
            job.started_at_ms.get_or_insert(now);
        }
        self.active.insert(
            id.to_string(),
            Active {
                attempt,
                handle,
                reservation,
                seq,
                use_gpu: reservation.gpu_slot,
                start_sent: false,
                spawned_at_ms: now,
                stop: None,
                stop_deadline_ms: None,
                killed_for_timeout: false,
                resumed: self.transcript_end(id).0 > 0,
                done: false,
                error: None,
                last_progress_emit_ms: 0,
                last_segment_emit_ms: 0,
                pending_segments: Vec::new(),
            },
        );
        self.changed(id);
    }

    fn on_worker_event(&mut self, event: WorkerEvent) {
        let current = self
            .active
            .get(&event.job_id)
            .is_some_and(|a| a.attempt == event.attempt);
        if !current {
            return;
        }
        let id = event.job_id;
        match event.kind {
            WorkerEventKind::Message(message) => self.on_message(&id, message),
            WorkerEventKind::Exited(exit) => {
                self.finish(&id, exit);
                self.try_start_next();
            }
        }
    }

    fn on_message(&mut self, id: &str, message: WorkerToCore) {
        match message {
            WorkerToCore::Ready { .. } => self.on_ready(id),
            // Only sent in reply to a probe, which never goes through a job.
            WorkerToCore::Durations { .. } => {}
            WorkerToCore::Audio { duration_sec } => {
                if let Ok(job) = self.job_mut(id) {
                    job.duration_ms = Some((duration_sec.max(0.0) * 1000.0) as u64);
                }
                self.changed(id);
            }
            WorkerToCore::Progress { pct, stage } => self.on_progress(id, pct, stage),
            WorkerToCore::Segment {
                start,
                end,
                text,
                words,
            } => self.on_segment(id, start, end, text, words),
            WorkerToCore::Metrics { rtf, .. } => {
                if let Ok(job) = self.job_mut(id) {
                    job.rtf = Some(rtf);
                }
            }
            WorkerToCore::Done => {
                if let Some(active) = self.active.get_mut(id) {
                    active.done = true;
                }
            }
            WorkerToCore::Error { error } => {
                if let Some(active) = self.active.get_mut(id) {
                    active.error = Some(error);
                }
            }
        }
    }

    /// The worker is up: hand it the job.
    fn on_ready(&mut self, id: &str) {
        let Ok(job) = self.job(id).cloned() else {
            return;
        };
        let start = self.start_message(&job);
        let Some(active) = self.active.get_mut(id) else {
            return;
        };
        if active.start_sent || active.stop.is_some() {
            return;
        }
        let start = start.and_then(|start| active.handle.send(&start));
        match start {
            Ok(()) => {
                active.start_sent = true;
                if let Ok(job) = self.job_mut(id) {
                    job.try_transition(JobState::Running);
                }
                self.changed(id);
            }
            Err(e) => {
                active.error = Some(ErrorInfo::from(&e));
                active.handle.kill();
            }
        }
    }

    fn start_message(&self, job: &Job) -> Result<CoreToWorker> {
        let active = self.active.get(&job.id).ok_or_else(|| OasError::Internal {
            reason: "starting a job with no worker".into(),
        })?;
        let installed = self
            .db
            .list_installed_models()?
            .into_iter()
            .find(|m| m.id == job.model_id)
            .ok_or_else(|| OasError::ModelNotInstalled {
                model_id: job.model_id.clone(),
            })?;
        let model_path = self
            .paths
            .models_dir()
            .join(&installed.id)
            .join(&installed.file_name);
        let (stored, end_ms) = self.transcript_end(&job.id);
        Ok(CoreToWorker::Start {
            job_id: job.id.clone(),
            audio_path: job.input_path.clone(),
            model_path: model_path.to_string_lossy().into_owned(),
            engine: EngineKind::WhisperCpp,
            options: JobOptions {
                language: job.language.clone(),
                translate: job.translate,
                word_timestamps: job.word_timestamps,
            },
            limits: Limits {
                max_threads: active.reservation.threads,
                mem_budget_mib: (active.reservation.mem_bytes / (1024 * 1024)) as u32,
                use_gpu: active.use_gpu,
            },
            resume_from_sec: (stored > 0).then_some(end_ms as f64 / 1000.0),
        })
    }

    /// Segments stored for a job, and where the last one ends.
    fn transcript_end(&self, id: &str) -> (u32, u64) {
        if let Some(end) = self.transcript_ends.get(id) {
            return *end;
        }
        match self.db.list_segments(id) {
            Ok(rows) => rows
                .last()
                .map_or((0, 0), |last| (last.idx + 1, last.end_ms)),
            Err(e) => {
                tracing::error!(error = %e, "could not read a transcript");
                (0, 0)
            }
        }
    }

    fn on_progress(&mut self, id: &str, pct: f32, stage: String) {
        let now = self.now();
        let Some(active) = self.active.get_mut(id) else {
            return;
        };
        let due = now.saturating_sub(active.last_progress_emit_ms) >= EMIT_INTERVAL_MS;
        if due {
            active.last_progress_emit_ms = now;
        }
        let Ok(job) = self.job_mut(id) else {
            return;
        };
        // A resumed run reports from where it is; progress never goes backwards.
        job.progress = job.progress.max((pct / 100.0).clamp(0.0, 1.0));
        if !due {
            return;
        }
        let progress = job.progress;
        let eta_secs = job.started_at_ms.filter(|_| progress > 0.0).map(|started| {
            let elapsed = now.saturating_sub(started) as f64 / 1000.0;
            elapsed * (1.0 - progress as f64) / progress as f64
        });
        let row = job_to_row(job);
        if let Err(e) = self.db.save_job(&row) {
            tracing::error!(error = %e, "could not save progress");
        }
        self.events.job_progress(&JobProgress {
            job_id: id.to_string(),
            progress,
            stage,
            eta_secs,
        });
    }

    fn on_segment(
        &mut self,
        id: &str,
        start: f64,
        end: f64,
        text: String,
        words: Vec<oas_protocol::Word>,
    ) {
        let (idx, _) = self.transcript_end(id);
        let segment = JobSegment {
            idx,
            start_ms: (start.max(0.0) * 1000.0) as u64,
            end_ms: (end.max(0.0) * 1000.0) as u64,
            text,
        };
        let words_json = (!words.is_empty())
            .then(|| {
                let words: Vec<_> = words
                    .iter()
                    .map(|w| serde_json::json!({"start": w.start, "end": w.end, "text": w.text}))
                    .collect();
                serde_json::to_string(&words).ok()
            })
            .flatten();
        let row = SegmentRow {
            idx,
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text: segment.text.clone(),
            words_json,
        };
        if let Err(e) = self.db.save_segment(id, &row) {
            tracing::error!(error = %e, "could not save a transcript segment");
        }
        self.transcript_ends
            .insert(id.to_string(), (idx + 1, segment.end_ms));

        let now = self.now();
        if let Some(active) = self.active.get_mut(id) {
            active.pending_segments.push(segment);
            if now.saturating_sub(active.last_segment_emit_ms) >= EMIT_INTERVAL_MS {
                self.flush_segments(id);
            }
        }
    }

    fn flush_segments(&mut self, id: &str) {
        let now = self.now();
        let Some(active) = self.active.get_mut(id) else {
            return;
        };
        if active.pending_segments.is_empty() {
            return;
        }
        active.last_segment_emit_ms = now;
        let pending = std::mem::take(&mut active.pending_segments);
        self.events.job_segments(id, &pending);
    }

    /// The worker is gone: release everything and settle the job's final state.
    /// This is the one path every job takes out of `Active`.
    fn finish(&mut self, id: &str, exit: WorkerExit) {
        self.flush_segments(id);
        let Some(active) = self.active.remove(id) else {
            return;
        };
        self.governor.release(&active.reservation);
        let now = self.now();

        if active.done {
            if let Ok(job) = self.job_mut(id) {
                // Started and finished between two events still ends cleanly.
                job.try_transition(JobState::Running);
                job.try_transition(JobState::Completed);
                job.progress = 1.0;
                job.finished_at_ms = Some(now);
            }
            self.report_measurement(id, &active);
        } else {
            match active.stop {
                Some(Stop::Cancel) => {
                    if let Ok(job) = self.job_mut(id) {
                        job.try_transition(JobState::Cancelled);
                        job.finished_at_ms = Some(now);
                    }
                }
                Some(Stop::Pause(origin)) => {
                    if let Ok(job) = self.job_mut(id) {
                        job.try_transition(JobState::Paused);
                        job.pause_origin = Some(origin);
                    }
                }
                None => self.settle_failure(id, &active, &exit),
            }
        }

        if self.delete_on_exit.remove(id) {
            if let Err(e) = self.delete(id) {
                tracing::error!(error = %e, "could not delete a job");
            }
        } else {
            self.changed(id);
        }
    }

    fn report_measurement(&self, id: &str, active: &Active) {
        let Some(job) = self.jobs.get(id) else {
            return;
        };
        let long_enough = job
            .duration_ms
            .is_some_and(|ms| ms >= MIN_MEASURED_AUDIO_MS);
        if let (Some(rtf), false, true) = (job.rtf, active.resumed, long_enough) {
            self.events.job_measured(&job.model_id, rtf, active.use_gpu);
        }
    }

    fn settle_failure(&mut self, id: &str, active: &Active, exit: &WorkerExit) {
        let now = self.now();
        let error = active.error.clone().unwrap_or_else(|| {
            ErrorInfo::from(if active.killed_for_timeout {
                OasError::WorkerCrashed {
                    reason: "the worker did not start in time".into(),
                }
            } else if exit.oom_suspected {
                OasError::OutOfMemory
            } else {
                OasError::WorkerCrashed {
                    reason: exit.detail.clone(),
                }
            })
        });

        // GPU failed to start: run this job on the CPU instead, once.
        if error.code == "GPU_INIT_FAILED" && active.use_gpu {
            self.force_cpu.insert(id.to_string());
            if let Ok(job) = self.job_mut(id) {
                job.state = JobState::Queued;
            }
            self.warn(WarningReason::GpuFallback, vec![id.to_string()]);
            return;
        }

        if error.code == "OUT_OF_MEMORY" {
            self.learn_from_oom(id);
        }
        if let Ok(job) = self.job_mut(id) {
            job.state = JobState::Failed;
            job.finished_at_ms = Some(now);
            job.error_code = Some(error.code);
            job.error_message = Some(error.message);
        }
    }

    /// This many jobs of the model were too many: allow one fewer from now on.
    fn learn_from_oom(&mut self, id: &str) {
        let Ok(model_id) = self.job(id).map(|j| j.model_id.clone()) else {
            return;
        };
        // The failed job has already left `active`.
        let concurrent = self.running_with_model(&model_id) + 1;
        let cap = concurrent.saturating_sub(1).max(1);
        let entry = self.learned_caps.entry(model_id).or_insert(cap);
        *entry = (*entry).min(cap);
        self.auto.on_pressure(self.clock.now_ms());
        self.warn(WarningReason::WorkerOutOfMemory, vec![id.to_string()]);
    }

    fn tick(&mut self, signals: Signals) {
        let now = self.now();
        self.signals = signals;

        // Budgets from live memory. A loaded job's memory is already missing
        // from `available` while its reservation also counts against the budget,
        // so it is added back. A job still loading has taken nothing yet.
        let loaded: u64 = self
            .active
            .values()
            .filter(|a| now.saturating_sub(a.spawned_at_ms) >= MODEL_LOAD_SETTLE_MS)
            .map(|a| a.reservation.mem_bytes)
            .sum();
        let pool = signals
            .available_ram
            .saturating_add(loaded)
            .min(self.machine.total_ram);
        self.governor.set_budgets(Budgets::new(
            pool,
            self.machine.total_ram,
            self.machine.logical_cores,
            self.machine.gpu_slots,
        ));

        self.enforce_deadlines(now);
        let ids: Vec<String> = self.active.keys().cloned().collect();
        for id in ids {
            self.flush_segments(&id);
        }

        let swap_growth = self.swap_growth(now, signals.swap_used);
        let actions = decide(
            signals.available_ram,
            swap_growth,
            signals.thermal,
            &self.watchdog,
        );
        let pressured = self.protect(now, &actions);
        if !pressured {
            self.resume_after_recovery(now, swap_growth);
            self.update_auto(now);
        }
        self.try_start_next();
    }

    /// Kill workers that overran a stop request or never became ready.
    fn enforce_deadlines(&mut self, now: u64) {
        for active in self.active.values_mut() {
            if active.stop_deadline_ms.is_some_and(|d| now >= d) {
                active.handle.kill();
            } else if !active.start_sent
                && !active.killed_for_timeout
                && now.saturating_sub(active.spawned_at_ms) >= START_TIMEOUT_MS
            {
                active.killed_for_timeout = true;
                active.handle.kill();
            }
        }
    }

    /// Swap growth over the last window.
    fn swap_growth(&mut self, now: u64, swap_used: u64) -> u64 {
        self.swap_history.push_back((now, swap_used));
        while self
            .swap_history
            .front()
            .is_some_and(|(t, _)| now.saturating_sub(*t) > SWAP_WINDOW_MS)
        {
            self.swap_history.pop_front();
        }
        let lowest = self
            .swap_history
            .iter()
            .map(|(_, s)| *s)
            .min()
            .unwrap_or(swap_used);
        swap_used.saturating_sub(lowest)
    }

    /// Jobs still running: those not already being stopped, newest first.
    fn stoppable_newest_first(&self) -> Vec<String> {
        let mut running: Vec<(u64, String)> = self
            .active
            .iter()
            .filter(|(_, a)| a.stop.is_none())
            .map(|(id, a)| (a.seq, id.clone()))
            .collect();
        running.sort_unstable_by_key(|(seq, _)| std::cmp::Reverse(*seq));
        running.into_iter().map(|(_, id)| id).collect()
    }

    /// Carry out the watchdog's actions. Returns whether the machine is under
    /// pressure, even if nothing was left to pause.
    fn protect(&mut self, now: u64, actions: &[WatchdogAction]) -> bool {
        let mut paused_now = false;
        for action in actions {
            let running = self.stoppable_newest_first();
            let (reason, targets) = match *action {
                WatchdogAction::PauseNewest(reason) => {
                    // One pause at a time: memory takes a moment to come back.
                    let cooling = paused_now
                        || now.saturating_sub(self.last_protective_pause_ms) < WATCHDOG_COOLDOWN_MS
                            && self.last_protective_pause_ms != 0;
                    let targets = if cooling {
                        Vec::new()
                    } else {
                        running.into_iter().take(1).collect()
                    };
                    (reason, targets)
                }
                WatchdogAction::ReduceToOne(reason) => {
                    let keep = usize::from(self.active.len() == running.len());
                    let count = running.len().saturating_sub(keep);
                    (reason, running.into_iter().take(count).collect())
                }
                WatchdogAction::PauseAll(reason) => (reason, running),
            };
            if targets.is_empty() {
                continue;
            }
            for id in &targets {
                self.request_stop(id, Stop::Pause(PauseOrigin::Governor));
            }
            paused_now = true;
            self.last_protective_pause_ms = now;
            self.auto.on_pressure(now);
            self.warn(reason, targets);
        }
        !actions.is_empty()
    }

    /// Bring back one governor-paused job once the machine is calm, oldest
    /// first. The queue's own admission rule still has the last word.
    fn resume_after_recovery(&mut self, now: u64, swap_growth: u64) {
        let thermal_ok = !matches!(
            self.signals.thermal,
            ThermalState::Serious | ThermalState::Critical
        );
        let calm = self.signals.available_ram >= self.watchdog.safety_reserve
            && swap_growth <= self.watchdog.swap_growth_limit
            && thermal_ok
            && now.saturating_sub(self.last_protective_pause_ms) >= WATCHDOG_COOLDOWN_MS
            && now.saturating_sub(self.last_auto_resume_ms) >= WATCHDOG_COOLDOWN_MS;
        if !calm {
            return;
        }
        let oldest = self
            .jobs
            .values()
            .filter(|j| {
                j.state == JobState::Paused && j.pause_origin == Some(PauseOrigin::Governor)
            })
            .min_by_key(|j| j.rank)
            .map(|j| j.id.clone());
        if let Some(id) = oldest {
            if self.resume(&id).is_ok() {
                self.last_auto_resume_ms = now;
                self.warn(WarningReason::Recovered, vec![id]);
            }
        }
    }

    fn update_auto(&mut self, now: u64) {
        let Some(next) = self.next_job_id().and_then(|id| {
            let job = self.jobs.get(&id)?;
            self.cost_of(job)
        }) else {
            return;
        };
        let saturated = self.active.len() as u32 >= self.auto.target();
        let fits = self.governor.mem_fits(next.mem_bytes);
        let max_safe = self.governor.max_concurrency(next.mem_bytes);
        self.auto
            .on_tick(now, saturated, fits, self.signals.cpu_total_pct, max_safe);
    }

    fn status(&self) -> GovernorStatus {
        let count = |want: fn(&Job) -> bool| self.jobs.values().filter(|j| want(j)).count() as u32;
        let limit = self.parallel_limit();
        let budgets = self.governor.budgets();

        // Whole-queue estimate, for the part of each job still to do.
        let mut unknown_length = false;
        let work: Vec<(f32, f32)> = self
            .jobs
            .values()
            .filter(|j| matches!(j.state, JobState::Queued) || j.state.is_active())
            .filter_map(|j| {
                if j.duration_ms.is_none() {
                    unknown_length = true;
                }
                let left = j.duration_ms? as f32 / 1000.0 * (1.0 - j.progress);
                Some((left, (self.rtf_estimate)(&j.model_id)))
            })
            .collect();
        let eta = (!work.is_empty() && !unknown_length).then(|| queue_eta_secs(&work, limit));

        GovernorStatus {
            mode: self.mode,
            parallel_limit: limit,
            running: count(|j| j.state.is_active()),
            queued: count(|j| j.state == JobState::Queued),
            paused: count(|j| j.state == JobState::Paused),
            mem_budget: budgets.mem_budget,
            reserved_mem: self.governor.reserved_mem(),
            cpu_budget: budgets.cpu_budget,
            reserved_cpu: self.governor.reserved_cpu(),
            gauge: self
                .next_job_cost()
                .map(|c| gauge(limit, budgets, c.mem_bytes)),
            queue_eta_secs: eta,
        }
    }
}

#[cfg(test)]
mod tests;
