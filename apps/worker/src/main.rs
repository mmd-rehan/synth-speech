//! `oas-worker`: the inference worker process, shipped as a Tauri sidecar.
//!
//! In M00 it only announces readiness and exits, which is enough to prove the
//! app can spawn the sidecar and read a line of the JSONL protocol. Later
//! milestones add the stdin read loop that handles `start`/`cancel` and runs a
//! model, streaming `progress`/`segment`/`metrics`/`done` back to the core.

#![forbid(unsafe_code)]

use std::io::Write;
use std::process::ExitCode;

use oas_protocol::{Envelope, WorkerToCore};

fn main() -> ExitCode {
    let ready = Envelope::new(WorkerToCore::Ready {
        worker_version: env!("CARGO_PKG_VERSION").to_string(),
    });

    let line = match ready.to_line() {
        Ok(line) => line,
        Err(err) => {
            eprintln!("worker failed to encode ready message: {err}");
            return ExitCode::FAILURE;
        }
    };

    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    if writeln!(lock, "{line}")
        .and_then(|()| lock.flush())
        .is_err()
    {
        // stdout closed; nothing useful we can report.
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
