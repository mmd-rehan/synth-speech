//! Tauri commands for the model catalog and fit engine.

use oas_catalog::ModelEntry;
use oas_fit::{
    accelerator_key, compute_fit, compute_fit_calibrated, recommend, update_correction, FitConfig,
    FitResult,
};
use oas_hardware::Accelerator;
use tauri::State;

use crate::hardware::{current_profile, latest_usage, AppState};

/// The fit for a model with this machine's measured correction applied.
fn calibrated_fit(
    state: &AppState,
    model: &ModelEntry,
    accelerator: Option<Accelerator>,
) -> Result<FitResult, String> {
    let profile = current_profile(state)?;
    let usage = latest_usage(state);
    let config = FitConfig::default();
    let raw = compute_fit(
        &profile,
        usage.as_ref(),
        model,
        accelerator.clone(),
        &config,
    );
    let correction = state
        .db
        .get_calibration(accelerator_key(&raw.accelerator), &model.family)
        .map_err(|e| e.user_message())?
        .map(|(correction, _)| correction);
    Ok(compute_fit_calibrated(
        &profile,
        usage.as_ref(),
        model,
        accelerator,
        &config,
        correction,
    ))
}

/// Fold a finished job's measured speed into this machine's calibration.
pub fn record_measurement(state: &AppState, model_id: &str, rtf: f32, used_gpu: bool) {
    let Some(model) = state.catalog.get(model_id) else {
        return;
    };
    let Ok(profile) = current_profile(state) else {
        return;
    };
    let accelerator = profile.accel.iter().find(|a| {
        let is_cpu = matches!(
            a,
            Accelerator::CpuAvx2 | Accelerator::CpuAvx512 | Accelerator::CpuNeon
        );
        is_cpu != used_gpu
    });
    let Some(accelerator) = accelerator.cloned() else {
        return;
    };
    let key = accelerator_key(&accelerator);
    // Compared with the uncorrected estimate, so corrections never compound.
    let config = FitConfig::default();
    let estimate = compute_fit(&profile, None, model, Some(accelerator), &config).rtf_est;
    let previous = state.db.get_calibration(key, &model.family).unwrap_or(None);
    if let Some((correction, samples)) = update_correction(previous, rtf, estimate) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        if let Err(e) = state
            .db
            .save_calibration(key, &model.family, correction, samples, now)
        {
            tracing::error!(error = %e, "could not save a measurement");
        }
    }
}

/// List every model in the catalog.
#[tauri::command]
#[specta::specta]
pub fn catalog_list(state: State<'_, AppState>) -> Vec<ModelEntry> {
    state.catalog.models.clone()
}

/// Compute the fit verdict for one model on this machine, optionally for a
/// specific accelerator (otherwise the machine's best is used).
#[tauri::command]
#[specta::specta]
pub fn catalog_get_fit(
    state: State<'_, AppState>,
    model_id: String,
    accelerator: Option<Accelerator>,
) -> Result<FitResult, String> {
    let model = state
        .catalog
        .get(&model_id)
        .ok_or_else(|| format!("unknown model: {model_id}"))?
        .clone();
    calibrated_fit(&state, &model, accelerator)
}

/// Compute fits for every catalog model at once (auto accelerator). Aligned to
/// `catalog_list` order. Used by the Models screen and recomputed as live usage
/// changes.
#[tauri::command]
#[specta::specta]
pub fn catalog_fit_all(state: State<'_, AppState>) -> Result<Vec<FitResult>, String> {
    state
        .catalog
        .models
        .iter()
        .map(|m| calibrated_fit(&state, m, None))
        .collect()
}

/// The recommended model id for this machine, if any reaches Good/Excellent.
#[tauri::command]
#[specta::specta]
pub fn catalog_recommend(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let profile = current_profile(&state)?;
    let usage = latest_usage(&state);
    Ok(recommend(
        &profile,
        usage.as_ref(),
        &state.catalog,
        &FitConfig::default(),
    ))
}
