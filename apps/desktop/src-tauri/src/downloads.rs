//! Download manager commands and typed progress/state events.
//!
//! Downloads run on background tasks, serialised to one at a time (default
//! concurrency). Progress and state changes are pushed to the UI as typed
//! `tauri-specta` events so the TypeScript side never hand-writes the shapes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use oas_download::{download, DownloadConfig, DownloadOutcome, DownloadSpec};
use oas_storage::InstalledModel;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::hardware::{models_dir_free_bytes, AppState};

/// Progress for an in-flight download (emitted ≤4/sec).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct DownloadProgress {
    pub id: String,
    #[specta(type = f64)]
    pub downloaded: u64,
    #[specta(type = f64)]
    pub total: u64,
    pub speed_bps: f64,
    pub eta_secs: Option<f64>,
}

/// A download's lifecycle state change.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct DownloadState {
    pub id: String,
    /// One of: `downloading`, `completed`, `paused`, `cancelled`, `failed`.
    pub state: String,
    /// Stable error code when `state == "failed"`.
    pub error_code: Option<String>,
    /// Human message when `state == "failed"`.
    pub error_message: Option<String>,
}

impl DownloadState {
    fn simple(id: &str, state: &str) -> Self {
        DownloadState {
            id: id.to_string(),
            state: state.to_string(),
            error_code: None,
            error_message: None,
        }
    }
}

/// Per-download control flags.
struct Flags {
    cancel: Arc<AtomicBool>,
    purge: Arc<AtomicBool>,
}

/// Tracks active downloads and enforces single-flight concurrency.
pub struct DownloadManager {
    active: Mutex<HashMap<String, Flags>>,
    gate: tokio::sync::Mutex<()>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        DownloadManager {
            active: Mutex::new(HashMap::new()),
            gate: tokio::sync::Mutex::new(()),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn file_name_from_url(url: &str) -> String {
    url.rsplit('/').next().unwrap_or("model.bin").to_string()
}

/// Start (or resume) a download. Returns immediately; progress arrives via
/// events. A `.part` left by a previous pause is resumed automatically.
#[tauri::command]
#[specta::specta]
pub async fn models_download(app: AppHandle, model_id: String) -> Result<(), String> {
    let state = app.state::<AppState>();

    if state
        .db
        .is_installed(&model_id)
        .map_err(|e| e.user_message())?
    {
        let _ = DownloadState::simple(&model_id, "completed").emit(&app);
        return Ok(());
    }

    let model = state
        .catalog
        .get(&model_id)
        .ok_or_else(|| format!("unknown model: {model_id}"))?
        .clone();

    let models_dir = state.paths.models_dir();
    let free = models_dir_free_bytes(&state);

    let spec = DownloadSpec {
        id: model.id.clone(),
        file_name: file_name_from_url(&model.url),
        url: model.url.clone(),
        mirrors: model.mirrors.clone(),
        sha256: model.sha256.clone(),
        expected_size: model.weights_bytes,
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let purge = Arc::new(AtomicBool::new(false));
    {
        let mut active = lock(&state.downloads.active);
        if active.contains_key(&model_id) {
            return Err("this model is already downloading".into());
        }
        active.insert(
            model_id.clone(),
            Flags {
                cancel: cancel.clone(),
                purge: purge.clone(),
            },
        );
    }

    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app2.state::<AppState>();
        // Serialise downloads (default concurrency 1).
        let _permit = state.downloads.gate.lock().await;

        let _ = DownloadState::simple(&spec.id, "downloading").emit(&app2);

        let progress_app = app2.clone();
        let progress_id = spec.id.clone();
        let result = download(
            &spec,
            &models_dir,
            free,
            &DownloadConfig::default(),
            cancel.clone(),
            move |p| {
                let _ = DownloadProgress {
                    id: progress_id.clone(),
                    downloaded: p.downloaded,
                    total: p.total,
                    speed_bps: p.speed_bps,
                    eta_secs: p.eta_secs,
                }
                .emit(&progress_app);
            },
        )
        .await;

        // Remove from the active set.
        lock(&state.downloads.active).remove(&spec.id);

        match result {
            Ok(DownloadOutcome::Completed(manifest)) => {
                let row = InstalledModel {
                    id: manifest.id.clone(),
                    file_name: manifest.file_name,
                    size_bytes: manifest.size_bytes,
                    sha256: manifest.sha256,
                    installed_at_ms: manifest.installed_at_ms,
                };
                if let Err(e) = state.db.save_installed_model(&row) {
                    let mut s = DownloadState::simple(&spec.id, "failed");
                    s.error_code = Some(e.code().to_string());
                    s.error_message = Some(e.user_message());
                    let _ = s.emit(&app2);
                } else {
                    let _ = DownloadState::simple(&spec.id, "completed").emit(&app2);
                }
            }
            Ok(DownloadOutcome::Stopped) => {
                if purge.load(Ordering::Relaxed) {
                    let tmp = models_dir.join(".tmp").join(&spec.id);
                    let _ = std::fs::remove_dir_all(&tmp);
                    let _ = DownloadState::simple(&spec.id, "cancelled").emit(&app2);
                } else {
                    let _ = DownloadState::simple(&spec.id, "paused").emit(&app2);
                }
            }
            Err(e) => {
                let mut s = DownloadState::simple(&spec.id, "failed");
                s.error_code = Some(e.code().to_string());
                s.error_message = Some(e.user_message());
                let _ = s.emit(&app2);
            }
        }
    });

    Ok(())
}

fn signal(state: &AppState, id: &str, purge: bool) {
    let active = lock(&state.downloads.active);
    if let Some(flags) = active.get(id) {
        if purge {
            flags.purge.store(true, Ordering::Relaxed);
        }
        flags.cancel.store(true, Ordering::Relaxed);
    }
}

/// Pause a download, keeping the partial file for later resume.
#[tauri::command]
#[specta::specta]
pub fn models_pause(state: State<'_, AppState>, model_id: String) {
    signal(&state, &model_id, false);
}

/// Resume a paused download (same as starting it again).
#[tauri::command]
#[specta::specta]
pub async fn models_resume(app: AppHandle, model_id: String) -> Result<(), String> {
    models_download(app, model_id).await
}

/// Cancel a download and discard the partial file.
#[tauri::command]
#[specta::specta]
pub fn models_cancel(state: State<'_, AppState>, model_id: String) {
    signal(&state, &model_id, true);
    // If nothing is active (never started / already stopped), clean any tmp.
    let tmp = state.paths.models_dir().join(".tmp").join(&model_id);
    if lock(&state.downloads.active).get(&model_id).is_none() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

/// Delete an installed model from disk and the database.
#[tauri::command]
#[specta::specta]
pub fn models_delete(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
    let dir = state.paths.models_dir().join(&model_id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    state
        .db
        .remove_installed_model(&model_id)
        .map_err(|e| e.user_message())
}

/// List installed models.
#[tauri::command]
#[specta::specta]
pub fn models_list_installed(state: State<'_, AppState>) -> Result<Vec<InstalledModel>, String> {
    state
        .db
        .list_installed_models()
        .map_err(|e| e.user_message())
}

/// Import an already-downloaded file: hash it, match it to a catalog model, and
/// install it if the checksum matches.
#[tauri::command]
#[specta::specta]
pub async fn models_import_file(
    app: AppHandle,
    model_id: String,
    source_path: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let model = state
        .catalog
        .get(&model_id)
        .ok_or_else(|| format!("unknown model: {model_id}"))?
        .clone();

    let src = std::path::PathBuf::from(&source_path);
    let hex = oas_download::hash_file(&src)
        .await
        .map_err(|e| e.user_message())?;
    if !hex.eq_ignore_ascii_case(&model.sha256) {
        return Err(oas_core::OasError::HashMismatch {
            expected: model.sha256.clone(),
            actual: hex,
        }
        .user_message());
    }

    let file_name = file_name_from_url(&model.url);
    let dir = state.paths.models_dir().join(&model.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::copy(&src, dir.join(&file_name)).map_err(|e| e.to_string())?;

    let size = std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
    let row = InstalledModel {
        id: model.id.clone(),
        file_name,
        size_bytes: size,
        sha256: model.sha256.clone(),
        installed_at_ms: now_ms(),
    };
    state
        .db
        .save_installed_model(&row)
        .map_err(|e| e.user_message())?;
    let _ = DownloadState::simple(&model.id, "completed").emit(&app);
    Ok(())
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Startup reconciliation: drop DB rows whose files are gone, and clear stale
/// `.tmp` download dirs that are not currently active.
pub fn reconcile(state: &AppState) {
    let models_dir = state.paths.models_dir();
    if let Ok(installed) = state.db.list_installed_models() {
        for m in installed {
            let file = models_dir.join(&m.id).join(&m.file_name);
            if !file.exists() {
                tracing::warn!(model = %m.id, "installed file missing; dropping record");
                let _ = state.db.remove_installed_model(&m.id);
            }
        }
    }
    // Remove leftover .tmp contents (no downloads run at startup).
    let tmp_root = models_dir.join(".tmp");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
}
