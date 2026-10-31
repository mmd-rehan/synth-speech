//! Job queue commands, the bridge from job events to the UI, and the worker's
//! location. The queue itself lives in `oas-jobs`.

use std::path::PathBuf;
use std::sync::Arc;

use oas_fit::{compute_fit, FitConfig};
use oas_hardware::{Accelerator, MachineProfile, UsageSample};
use oas_jobs::{
    Gauge, GovernorStatus, GovernorWarning, Job, JobEvents, JobManager, JobProgress, JobRequest,
    JobSegment, Machine, ManagerDeps, ParallelMode, Priority, ProcessSpawner, Signals, SystemClock,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::hardware::AppState;

/// A job changed state (or its wait reason, or its audio length).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct JobStateChanged {
    pub job: Job,
}

/// Progress for a running job (at most 4 per second per job).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct JobProgressChanged {
    pub progress: JobProgress,
}

/// New transcript lines for a running job (batched, at most 4 per second).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct JobSegmentsAdded {
    pub job_id: String,
    pub segments: Vec<JobSegment>,
}

/// The governor acted to protect the machine, or has news about it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct GovernorWarned {
    pub warning: GovernorWarning,
}

/// Forwards job events to the webview.
struct TauriJobEvents(AppHandle);

impl JobEvents for TauriJobEvents {
    fn job_state(&self, job: &Job) {
        let _ = JobStateChanged { job: job.clone() }.emit(&self.0);
    }

    fn job_progress(&self, progress: &JobProgress) {
        let _ = JobProgressChanged {
            progress: progress.clone(),
        }
        .emit(&self.0);
    }

    fn job_segments(&self, job_id: &str, segments: &[JobSegment]) {
        let _ = JobSegmentsAdded {
            job_id: job_id.to_string(),
            segments: segments.to_vec(),
        }
        .emit(&self.0);
    }

    fn governor_warning(&self, warning: &GovernorWarning) {
        let _ = GovernorWarned {
            warning: warning.clone(),
        }
        .emit(&self.0);
    }
}

/// The worker sits next to the app binary in every layout: Tauri copies
/// `externalBin` there for `tauri dev` and for each bundle format.
pub fn worker_exe() -> Result<PathBuf, String> {
    let app = std::env::current_exe().map_err(|e| format!("could not find the app: {e}"))?;
    let dir = app
        .parent()
        .ok_or_else(|| "the app has no parent folder".to_string())?;
    Ok(dir.join(format!("oas-worker{}", std::env::consts::EXE_SUFFIX)))
}

/// Build the manager and recover the queue from the last run.
pub fn create_manager(
    app: AppHandle,
    state: &AppState,
    profile: MachineProfile,
) -> Result<JobManager, String> {
    // Workers run on the CPU, so estimates must not assume a GPU.
    let cpu = profile.accel.iter().find(|a| {
        matches!(
            a,
            Accelerator::CpuAvx2 | Accelerator::CpuAvx512 | Accelerator::CpuNeon
        )
    });
    let cpu = cpu.cloned();
    let catalog = state.catalog.clone();
    let machine = Machine {
        total_ram: profile.memory.total_bytes,
        logical_cores: profile.cpu.logical_cores,
        // No GPU worker builds exist yet.
        gpu_slots: 0,
    };
    let rtf_estimate = {
        let catalog = catalog.clone();
        Box::new(move |model_id: &str| {
            catalog.get(model_id).map_or(1.0, |model| {
                let config = FitConfig::default();
                compute_fit(&profile, None, model, cpu.clone(), &config).rtf_est
            })
        })
    };

    JobManager::new(ManagerDeps {
        db: Arc::clone(&state.db),
        catalog,
        paths: state.paths.clone(),
        clock: Arc::new(SystemClock),
        spawner: Box::new(ProcessSpawner::new(worker_exe()?)),
        events: Arc::new(TauriJobEvents(app)),
        machine,
        rtf_estimate,
    })
    .map_err(|e| e.user_message())
}

/// Live signals for the governor from a usage sample.
pub fn signals_from(sample: &UsageSample, on_battery: bool) -> Signals {
    const PAUSE_ON_BATTERY_BELOW_PCT: u8 = 20;
    Signals {
        available_ram: sample.ram_available_bytes,
        swap_used: sample.swap_used_bytes,
        thermal: sample.thermal,
        cpu_total_pct: sample.cpu_total_pct,
        battery_low: on_battery
            && sample
                .battery_pct
                .is_some_and(|pct| pct < PAUSE_ON_BATTERY_BELOW_PCT),
    }
}

fn user_error(e: oas_core::OasError) -> String {
    e.user_message()
}

/// Add one job per file to the queue.
#[tauri::command]
#[specta::specta]
pub async fn jobs_create(
    jobs: State<'_, JobManager>,
    request: JobRequest,
) -> Result<Vec<Job>, String> {
    let durations = request
        .paths
        .iter()
        .map(|p| {
            oas_audio::probe_duration_sec(std::path::Path::new(p)).map(|s| (s * 1000.0) as u64)
        })
        .collect();
    jobs.create(request, durations).map_err(user_error)
}

/// Every job, in queue order.
#[tauri::command]
#[specta::specta]
pub fn jobs_list(jobs: State<'_, JobManager>) -> Vec<Job> {
    jobs.list()
}

#[tauri::command]
#[specta::specta]
pub fn jobs_get(jobs: State<'_, JobManager>, id: String) -> Result<Job, String> {
    jobs.get(&id).map_err(user_error)
}

/// A job's stored transcript.
#[tauri::command]
#[specta::specta]
pub fn jobs_segments(jobs: State<'_, JobManager>, id: String) -> Result<Vec<JobSegment>, String> {
    jobs.segments(&id).map_err(user_error)
}

#[tauri::command]
#[specta::specta]
pub fn jobs_pause(jobs: State<'_, JobManager>, id: String) -> Result<(), String> {
    jobs.pause(&id).map_err(user_error)
}

#[tauri::command]
#[specta::specta]
pub fn jobs_resume(jobs: State<'_, JobManager>, id: String) -> Result<(), String> {
    jobs.resume(&id).map_err(user_error)
}

#[tauri::command]
#[specta::specta]
pub fn jobs_cancel(jobs: State<'_, JobManager>, id: String) -> Result<(), String> {
    jobs.cancel(&id).map_err(user_error)
}

#[tauri::command]
#[specta::specta]
pub fn jobs_delete(jobs: State<'_, JobManager>, id: String) -> Result<(), String> {
    jobs.delete(&id).map_err(user_error)
}

#[tauri::command]
#[specta::specta]
pub fn jobs_retry(jobs: State<'_, JobManager>, id: String) -> Result<(), String> {
    jobs.retry(&id).map_err(user_error)
}

#[tauri::command]
#[specta::specta]
pub fn jobs_set_priority(
    jobs: State<'_, JobManager>,
    id: String,
    priority: Priority,
) -> Result<(), String> {
    jobs.set_priority(&id, priority).map_err(user_error)
}

/// Put these jobs in this order within the slots they already hold.
#[tauri::command]
#[specta::specta]
pub fn jobs_reorder(jobs: State<'_, JobManager>, ids: Vec<String>) -> Result<(), String> {
    jobs.reorder(&ids).map_err(user_error)
}

#[tauri::command]
#[specta::specta]
pub fn jobs_pause_all(jobs: State<'_, JobManager>) {
    jobs.pause_all();
}

#[tauri::command]
#[specta::specta]
pub fn jobs_resume_all(jobs: State<'_, JobManager>) {
    jobs.resume_all();
}

#[tauri::command]
#[specta::specta]
pub fn jobs_clear_completed(jobs: State<'_, JobManager>) -> Result<(), String> {
    jobs.clear_completed().map_err(user_error)
}

/// Write a transcript to `path` as `format` (txt, srt, vtt, or json).
#[tauri::command]
#[specta::specta]
pub async fn jobs_export(
    jobs: State<'_, JobManager>,
    id: String,
    format: String,
    path: String,
) -> Result<(), String> {
    jobs.export(&id, &format, std::path::Path::new(&path))
        .map_err(user_error)
}

#[tauri::command]
#[specta::specta]
pub fn governor_get_status(jobs: State<'_, JobManager>) -> GovernorStatus {
    jobs.status()
}

/// The gauge for running `desired` jobs of a model.
#[tauri::command]
#[specta::specta]
pub fn governor_gauge(
    jobs: State<'_, JobManager>,
    model_id: String,
    desired: u32,
) -> Result<Gauge, String> {
    jobs.gauge_for(&model_id, desired).map_err(user_error)
}

/// Choose Auto or Manual mode, and the Manual limit.
#[tauri::command]
#[specta::specta]
pub fn governor_set_parallel(jobs: State<'_, JobManager>, mode: ParallelMode, manual_max: u32) {
    jobs.set_parallel(mode, manual_max);
}
