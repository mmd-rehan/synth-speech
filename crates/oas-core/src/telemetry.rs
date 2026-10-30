//! Logging setup: human-readable lines to stdout plus a rolling daily log
//! file in the app data directory. `tracing` is used everywhere; this is the
//! one place the subscriber is configured.
//!
//! The returned [`LogGuard`] must be kept alive for as long as the process
//! logs — dropping it flushes and stops the background writer thread.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Keeps the non-blocking log writer alive. Hold it in `main` (or app state)
/// for the lifetime of the process.
#[must_use = "dropping the guard stops file logging"]
pub struct LogGuard(#[allow(dead_code)] WorkerGuard);

/// Initialise global logging.
///
/// Writes rolling daily files named `{app_name}.log` into `logs_dir` and also
/// echoes to stdout. The level is taken from the `RUST_LOG` environment
/// variable, defaulting to `info`. Calling this more than once in a process is
/// safe (subsequent calls are ignored), which keeps tests from fighting over
/// the global subscriber.
pub fn init(logs_dir: &Path, app_name: &str) -> LogGuard {
    let file_appender = tracing_appender::rolling::daily(logs_dir, format!("{app_name}.log"));
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // `try_init` returns an error (rather than panicking) if a global
    // subscriber is already installed; we deliberately ignore that.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stdout).with_target(false))
        .with(fmt::layer().with_ansi(false).with_writer(non_blocking))
        .try_init();

    LogGuard(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent_and_writes_a_file() {
        let dir = std::env::temp_dir().join(format!("oas-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("make temp log dir");

        {
            let _g1 = init(&dir, "oas-test");
            // A second init must not panic.
            let _g2 = init(&dir, "oas-test");
            tracing::info!("hello from the telemetry test");
        } // guards drop here, flushing the writer

        let has_log = std::fs::read_dir(&dir)
            .expect("read temp log dir")
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with("oas-test.log"));
        assert!(has_log, "a rolling log file should have been created");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
