//! The SynthSpeech desktop shell.
//!
//! This crate wires the React UI to the Rust core over Tauri commands. All IPC
//! types are generated into `../src/bindings.ts` by `tauri-specta`, so the
//! TypeScript side never hand-copies a type.

mod catalog;
mod downloads;
mod hardware;
mod jobs;
mod settings;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder};

use oas_core::Paths;

/// Basic identity of the running app, surfaced in the UI and diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppInfo {
    /// Product name.
    pub name: String,
    /// Semantic version of the app.
    pub version: String,
}

/// Return the app's name and version.
#[tauri::command]
#[specta::specta]
fn app_version() -> AppInfo {
    AppInfo {
        name: "SynthSpeech".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Spawn the `oas-worker` sidecar, read its first JSONL line, and return the
/// reported worker version. Proves the bundled worker can run.
#[tauri::command]
#[specta::specta]
async fn spawn_worker_probe() -> Result<String, String> {
    let exe = jobs::worker_exe()?;
    tauri::async_runtime::spawn_blocking(move || oas_jobs::probe_worker(exe))
        .await
        .map_err(|e| format!("the worker check stopped: {e}"))?
        .map_err(|e| e.user_message())
}

/// Construct the `tauri-specta` builder shared by the running app and the
/// `export-bindings` binary, so both always agree on the command set.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            app_version,
            spawn_worker_probe,
            hardware::hardware_get_profile,
            hardware::hardware_refresh,
            hardware::hardware_usage_history,
            hardware::copy_diagnostics,
            catalog::catalog_list,
            catalog::catalog_get_fit,
            catalog::catalog_fit_all,
            catalog::catalog_recommend,
            downloads::models_download,
            downloads::models_pause,
            downloads::models_resume,
            downloads::models_cancel,
            downloads::models_delete,
            downloads::models_list_installed,
            downloads::models_import_file,
            jobs::jobs_create,
            jobs::jobs_list,
            jobs::jobs_get,
            jobs::jobs_segments,
            jobs::jobs_pause,
            jobs::jobs_resume,
            jobs::jobs_cancel,
            jobs::jobs_delete,
            jobs::jobs_retry,
            jobs::jobs_set_priority,
            jobs::jobs_reorder,
            jobs::jobs_pause_all,
            jobs::jobs_resume_all,
            jobs::jobs_clear_completed,
            jobs::jobs_export,
            jobs::governor_get_status,
            jobs::governor_gauge,
            jobs::governor_set_parallel,
            jobs::jobs_probe_durations,
            jobs::models_test_speed,
            jobs::jobs_allow_playback,
            jobs::jobs_update_segment,
            settings::settings_get,
            settings::settings_set,
            settings::settings_move_models,
        ])
        .events(collect_events![
            downloads::DownloadProgress,
            downloads::DownloadState,
            jobs::JobStateChanged,
            jobs::JobRemoved,
            jobs::JobProgressChanged,
            jobs::JobSegmentsAdded,
            jobs::GovernorWarned
        ])
}

/// Absolute path to the generated bindings file, resolved from this crate's
/// location so it is independent of the current working directory.
fn bindings_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bindings.ts")
}

/// Regenerate the TypeScript IPC bindings from the Rust command set.
///
/// Invoked by the `export-bindings` binary (used in CI and `scripts/check.sh`)
/// and on every debug launch. Kept out of a `#[test]` on purpose: `tauri-build`
/// only applies its required linker arguments to *binaries*, so a Tauri-linked
/// test harness fails to start on Windows (STATUS_ENTRYPOINT_NOT_FOUND). A real
/// binary does not have that problem.
pub fn export_bindings() -> Result<(), String> {
    // 64-bit byte/timestamp fields are annotated `#[specta(type = f64)]` on the
    // Rust types (all well under 2^53), so the default exporter is fine.
    specta_builder()
        .export(specta_typescript::Typescript::default(), bindings_path())
        .map_err(|e| e.to_string())
}

/// Entry point used by both desktop `main` and the mobile entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // In debug builds, regenerate the TypeScript bindings on every launch so
    // they can never drift from the Rust command definitions.
    #[cfg(debug_assertions)]
    export_bindings().expect("failed to export TypeScript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(builder.invoke_handler())
        .on_window_event(|window, event| {
            // Focus is a good-enough proxy for visibility: slow the usage
            // sampler when the window is in the background.
            if let tauri::WindowEvent::Focused(focused) = event {
                if let Some(state) = window.app_handle().try_state::<hardware::AppState>() {
                    state.set_visible(*focused);
                }
            }
        })
        .setup(move |app| {
            builder.mount_events(app);

            // Resolve the app data directory and ensure the layout exists.
            let root = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("could not resolve the app data directory: {e}"))?;
            let paths = Paths::new(root);
            std::fs::create_dir_all(paths.root())
                .map_err(|e| format!("could not create the app folder: {e}"))?;

            // Logging to the app data dir, then the database.
            std::fs::create_dir_all(paths.logs_dir())
                .map_err(|e| format!("could not create the logs folder: {e}"))?;
            let log_guard = oas_core::telemetry::init(&paths.logs_dir(), "oas-desktop");
            let db = oas_storage::Database::open(&paths.database_file())
                .map(std::sync::Arc::new)
                .map_err(|e| e.user_message())?;

            // Settings can move the models folder, so they come before the layout.
            let settings = db.load_settings().map_err(|e| e.user_message())?;
            let paths = match &settings.models_dir {
                Some(dir) => paths.with_models_dir(dir),
                None => paths,
            };
            for dir in paths.required_dirs() {
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
            }

            // The catalog is embedded at build time and validated on load.
            let catalog =
                oas_catalog::Catalog::from_json(include_str!("../../../../catalog/models.v1.json"))
                    .map_err(|e| e.user_message())?;

            app.manage(hardware::AppState::new(
                db,
                paths,
                catalog,
                settings.clone(),
                log_guard,
            ));
            let state = app.state::<hardware::AppState>();
            downloads::reconcile(state.inner());
            let profile = hardware::current_profile(state.inner())?;
            let jobs = jobs::create_manager(app.handle().clone(), state.inner(), profile)?;
            settings::apply(&jobs, &settings);
            if let Err(e) = jobs.purge_finished_older_than(settings.retention_days) {
                tracing::error!(error = %e, "could not remove old jobs");
            }
            jobs.start();
            app.manage(jobs);
            hardware::spawn_usage_monitor(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the SynthSpeech app");
}
