//! Tauri commands for the model catalog and fit engine.

use oas_catalog::ModelEntry;
use oas_fit::{compute_fit, recommend, FitConfig, FitResult};
use oas_hardware::Accelerator;
use tauri::State;

use crate::hardware::{current_profile, latest_usage, AppState};

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
    let profile = current_profile(&state)?;
    let usage = latest_usage(&state);
    Ok(compute_fit(
        &profile,
        usage.as_ref(),
        &model,
        accelerator,
        &FitConfig::default(),
    ))
}

/// Compute fits for every catalog model at once (auto accelerator). Aligned to
/// `catalog_list` order. Used by the Models screen and recomputed as live usage
/// changes.
#[tauri::command]
#[specta::specta]
pub fn catalog_fit_all(state: State<'_, AppState>) -> Result<Vec<FitResult>, String> {
    let profile = current_profile(&state)?;
    let usage = latest_usage(&state);
    let config = FitConfig::default();
    Ok(state
        .catalog
        .models
        .iter()
        .map(|m| compute_fit(&profile, usage.as_ref(), m, None, &config))
        .collect())
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
