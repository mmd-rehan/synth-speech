//! App state and Tauri commands for hardware profiling and live usage.
//!
//! The profile is cached in SQLite; live usage is sampled on a background
//! thread and pushed to the UI as `hardware:usage` events (1 Hz while the
//! window is focused, 0.2 Hz otherwise).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use oas_core::telemetry::LogGuard;
use oas_core::Paths;
use oas_hardware::{
    build_profile, MachineProfile, RealProbe, SystemProbe, UsageHistory, UsageSample,
};
use oas_storage::Database;
use tauri::{AppHandle, Emitter, Manager, State};

/// Shared application state, managed by Tauri.
pub struct AppState {
    pub db: Database,
    pub paths: Paths,
    pub catalog: oas_catalog::Catalog,
    probe: Mutex<RealProbe>,
    history: Mutex<UsageHistory>,
    visible: AtomicBool,
    // Kept alive so file logging runs for the whole process lifetime.
    _log_guard: LogGuard,
}

impl AppState {
    pub fn new(
        db: Database,
        paths: Paths,
        catalog: oas_catalog::Catalog,
        log_guard: LogGuard,
    ) -> Self {
        AppState {
            db,
            paths,
            catalog,
            probe: Mutex::new(RealProbe::new()),
            history: Mutex::new(UsageHistory::new()),
            visible: AtomicBool::new(true),
            _log_guard: log_guard,
        }
    }

    pub fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Relaxed);
    }
}

/// The current machine profile (cached or freshly built). Shared with the
/// catalog/fit commands.
pub(crate) fn current_profile(state: &AppState) -> Result<MachineProfile, String> {
    get_or_build(state)
}

/// The most recent usage sample, if the monitor has produced one.
pub(crate) fn latest_usage(state: &AppState) -> Option<UsageSample> {
    lock(&state.history).latest().cloned()
}

/// Recover a mutex guard even if a previous holder panicked.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn persist(state: &AppState, profile: &MachineProfile) -> Result<(), String> {
    let json = serde_json::to_string(profile).map_err(|e| e.to_string())?;
    state
        .db
        .save_machine_profile(&json, profile.captured_at_ms)
        .map_err(|e| e.user_message())
}

fn build_fresh(state: &AppState) -> MachineProfile {
    let probe = lock(&state.probe);
    build_profile(&*probe, &state.paths.models_dir())
}

/// Return the cached profile, building and caching one on first use.
fn get_or_build(state: &AppState) -> Result<MachineProfile, String> {
    if let Some((json, _)) = state
        .db
        .load_machine_profile()
        .map_err(|e| e.user_message())?
    {
        if let Ok(profile) = serde_json::from_str::<MachineProfile>(&json) {
            return Ok(profile);
        }
        // A stored profile that no longer parses (schema changed) is rebuilt.
    }
    let profile = build_fresh(state);
    persist(state, &profile)?;
    Ok(profile)
}

/// Return the cached machine profile, building and caching one on first use.
#[tauri::command]
#[specta::specta]
pub fn hardware_get_profile(state: State<'_, AppState>) -> Result<MachineProfile, String> {
    get_or_build(&state)
}

/// Re-scan the machine and replace the cached profile.
#[tauri::command]
#[specta::specta]
pub fn hardware_refresh(state: State<'_, AppState>) -> Result<MachineProfile, String> {
    let profile = build_fresh(&state);
    persist(&state, &profile)?;
    Ok(profile)
}

/// The recent usage samples held in the ring buffer, to pre-fill the chart.
#[tauri::command]
#[specta::specta]
pub fn hardware_usage_history(state: State<'_, AppState>) -> Vec<UsageSample> {
    lock(&state.history).as_vec()
}

/// Assemble a diagnostics report: app + profile + recent logs, no user content.
#[tauri::command]
#[specta::specta]
pub fn copy_diagnostics(state: State<'_, AppState>) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("Open Audio Studio diagnostics\n");
    out.push_str(&format!("app version: {}\n", env!("CARGO_PKG_VERSION")));

    match get_or_build(&state) {
        Ok(p) => {
            out.push_str(&format!(
                "os: {} {} ({})\ncpu: {} ({} cores)\nram: {} MB total\naccelerators: {:?}\n",
                p.os.name,
                p.os.version,
                p.os.arch,
                p.cpu.brand,
                p.cpu.logical_cores,
                p.memory.total_bytes / (1024 * 1024),
                p.accel,
            ));
        }
        Err(e) => out.push_str(&format!("profile: unavailable ({e})\n")),
    }

    out.push_str("\n--- recent logs ---\n");
    out.push_str(&recent_logs(&state.paths, 50));
    Ok(out)
}

/// Read the tail of the most recent log file (best effort).
fn recent_logs(paths: &Paths, max_lines: usize) -> String {
    let dir = paths.logs_dir();
    let latest = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".log"))
        .max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .unwrap_or(std::time::UNIX_EPOCH)
        });

    match latest {
        Some(entry) => match std::fs::read_to_string(entry.path()) {
            Ok(contents) => {
                let lines: Vec<&str> = contents.lines().collect();
                let start = lines.len().saturating_sub(max_lines);
                lines[start..].join("\n")
            }
            Err(e) => format!("(could not read log: {e})"),
        },
        None => "(no logs yet)".to_string(),
    }
}

/// Start the background usage sampler. Emits `hardware:usage` every second when
/// the window is focused, every five seconds when it is not.
pub fn spawn_usage_monitor(app: AppHandle) {
    std::thread::spawn(move || loop {
        let interval = {
            let Some(state) = app.try_state::<AppState>() else {
                // State not ready yet (or app shutting down); back off.
                std::thread::sleep(Duration::from_millis(200));
                continue;
            };
            let sample = {
                let mut probe = lock(&state.probe);
                probe.sample_usage(&state.paths.models_dir())
            };
            lock(&state.history).push(sample.clone());
            let _ = app.emit("hardware:usage", &sample);

            if state.visible.load(Ordering::Relaxed) {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(5)
            }
        };
        std::thread::sleep(interval);
    });
}
