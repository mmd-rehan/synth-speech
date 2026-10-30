//! The Open Audio Studio desktop shell.
//!
//! This crate wires the React UI to the Rust core over Tauri commands. All IPC
//! types are generated into `../src/bindings.ts` by `tauri-specta`, so the
//! TypeScript side never hand-copies a type.
//!
//! In M00 the surface is intentionally small: report the app version, and probe
//! the `oas-worker` sidecar to prove the app can spawn it and read its `ready`
//! line.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_plugin_shell::ShellExt;
use tauri_specta::{collect_commands, Builder};

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
        name: "Open Audio Studio".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Spawn the `oas-worker` sidecar, read its first JSONL line, and return the
/// reported worker version. Proves the sidecar mechanism end to end.
#[tauri::command]
#[specta::specta]
async fn spawn_worker_probe(app: tauri::AppHandle) -> Result<String, String> {
    let command = app
        .shell()
        .sidecar("oas-worker")
        .map_err(|e| format!("could not locate the worker sidecar: {e}"))?;

    let output = command
        .output()
        .await
        .map_err(|e| format!("could not run the worker sidecar: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "worker exited unsuccessfully (code {:?})",
            output.status.code()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout
        .lines()
        .next()
        .ok_or_else(|| "worker produced no output".to_string())?;

    let envelope = oas_protocol::Envelope::<oas_protocol::WorkerToCore>::from_line(first_line)
        .map_err(|e| e.user_message())?;

    match envelope.msg {
        oas_protocol::WorkerToCore::Ready { worker_version } => Ok(worker_version),
        other => Err(format!("expected a ready message, got {other:?}")),
    }
}

/// Construct the `tauri-specta` builder shared by the running app and the
/// bindings-export test, so both always agree on the command set.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![app_version, spawn_worker_probe])
}

/// Entry point used by both desktop `main` and the mobile entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // In debug builds, regenerate the TypeScript bindings on every launch so
    // they can never drift from the Rust command definitions.
    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default(),
            "../src/bindings.ts",
        )
        .expect("failed to export TypeScript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Open Audio Studio app");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regenerates `../src/bindings.ts` and fails if the command set cannot be
    /// exported. CI also checks the committed file matches this output.
    #[test]
    fn exports_typescript_bindings() {
        specta_builder()
            .export(
                specta_typescript::Typescript::default(),
                "../src/bindings.ts",
            )
            .expect("export bindings");
    }
}
