//! Settings commands. Saving applies what can change live; moving the models
//! folder takes effect on the next start.

use oas_jobs::{JobManager, ParallelMode};
use oas_storage::Settings;
use tauri::State;

use crate::hardware::AppState;

/// Hand the running job system the settings it uses.
pub fn apply(jobs: &JobManager, settings: &Settings) {
    let mode = match settings.parallel_mode.as_str() {
        "manual" => ParallelMode::Manual,
        _ => ParallelMode::Auto,
    };
    jobs.set_parallel(mode, settings.max_parallel_jobs);
    jobs.set_thread_cap(settings.thread_cap_per_job);
}

#[tauri::command]
#[specta::specta]
pub fn settings_get(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// Save and apply settings. The models folder cannot change here: use
/// `settings_move_models`, which moves the files too.
#[tauri::command]
#[specta::specta]
pub fn settings_set(
    state: State<'_, AppState>,
    jobs: State<'_, JobManager>,
    settings: Settings,
) -> Result<(), String> {
    let settings = Settings {
        models_dir: state.settings().models_dir,
        ..settings
    };
    state
        .db
        .save_settings(&settings)
        .map_err(|e| e.user_message())?;
    apply(&jobs, &settings);
    if let Err(e) = jobs.purge_finished_older_than(settings.retention_days) {
        tracing::error!(error = %e, "could not remove old jobs");
    }
    state.set_settings(settings);
    Ok(())
}

/// Move the models folder. Refused while jobs or downloads are using it. The
/// app must restart to use the new location.
#[tauri::command]
#[specta::specta]
pub async fn settings_move_models(
    state: State<'_, AppState>,
    jobs: State<'_, JobManager>,
    path: String,
) -> Result<(), String> {
    if jobs.held_resources().0 > 0 || state.downloads.is_busy() {
        return Err("Wait for running jobs and downloads to finish, then try again.".into());
    }
    oas_download::relocate_models(&state.paths.models_dir(), std::path::Path::new(&path))
        .map_err(|e| e.user_message())?;
    let settings = Settings {
        models_dir: Some(path),
        ..state.settings()
    };
    state
        .db
        .save_settings(&settings)
        .map_err(|e| e.user_message())?;
    state.set_settings(settings);
    Ok(())
}
