//! User settings (doc 08 section 8.2). Each field is one row in `settings`, so
//! a value this build cannot read falls back to its default on its own, and a
//! newer build's extra keys are left alone.

use oas_core::{OasError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;

/// Field name to stored key.
const KEYS: &[(&str, &str)] = &[
    ("theme", "ui.theme"),
    ("language", "ui.language"),
    ("first_run_done", "ui.first_run_done"),
    ("models_dir", "models.dir"),
    ("parallel_mode", "perf.parallel_mode"),
    ("max_parallel_jobs", "perf.max_parallel_jobs"),
    ("thread_cap_per_job", "perf.thread_cap_per_job"),
    ("gpu_preference", "perf.gpu_preference"),
    ("pause_on_battery_below", "perf.pause_on_battery_below"),
    ("offline_mode", "net.offline_mode"),
    ("retention_days", "jobs.retention_days"),
];

/// Everything the user can set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Settings {
    /// `system`, `light`, or `dark`.
    pub theme: String,
    pub language: String,
    pub first_run_done: bool,
    /// `None` keeps models in the app data folder.
    pub models_dir: Option<String>,
    /// `auto` or `manual`.
    pub parallel_mode: String,
    pub max_parallel_jobs: u32,
    pub thread_cap_per_job: u32,
    /// `auto`, or `cpu` to never use a GPU.
    pub gpu_preference: String,
    /// Pause the queue on battery below this percent; 0 turns this off.
    pub pause_on_battery_below: u8,
    pub offline_mode: bool,
    /// Delete finished jobs after this many days; 0 keeps them forever.
    pub retention_days: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "system".into(),
            language: "en".into(),
            first_run_done: false,
            models_dir: None,
            parallel_mode: "auto".into(),
            max_parallel_jobs: 2,
            thread_cap_per_job: 8,
            gpu_preference: "auto".into(),
            pause_on_battery_below: 20,
            offline_mode: false,
            retention_days: 0,
        }
    }
}

impl Settings {
    /// Build from stored rows of (key, JSON value).
    pub(crate) fn from_stored(rows: Vec<(String, String)>) -> Self {
        let defaults = Settings::default();
        let Ok(Value::Object(mut fields)) = serde_json::to_value(&defaults) else {
            return defaults;
        };
        for (key, json) in rows {
            let Some((field, _)) = KEYS.iter().find(|(_, stored)| *stored == key) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&json) else {
                continue;
            };
            let mut candidate: Map<String, Value> = fields.clone();
            candidate.insert((*field).to_string(), value);
            if serde_json::from_value::<Settings>(Value::Object(candidate.clone())).is_ok() {
                fields = candidate;
            }
        }
        serde_json::from_value(Value::Object(fields)).unwrap_or(defaults)
    }

    /// The rows of (key, JSON value) to store.
    pub(crate) fn to_stored(&self) -> Result<Vec<(&'static str, String)>> {
        let fail = |e: serde_json::Error| OasError::Internal {
            reason: format!("could not encode settings: {e}"),
        };
        let fields = serde_json::to_value(self).map_err(fail)?;
        KEYS.iter()
            .map(|(field, key)| Ok((*key, fields[*field].to_string())))
            .collect()
    }
}
