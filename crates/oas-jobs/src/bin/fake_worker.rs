//! A stand-in for `oas-worker` used by the soak and chaos tests. It speaks the
//! real protocol over real stdio but needs no model. Its behaviour comes from
//! words in the audio path:
//!
//! - `slow`: many segments, slowly, so the test can act mid-run.
//! - `crash`: exit with a failure status after the first segment.
//! - `hang`: ignore `cancel`, so the core has to kill it.
//! - `fail`: report a decode error.
//! - `gpubad`: report a GPU start failure when asked to use the GPU.

#![forbid(unsafe_code)]

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use oas_core::error::ErrorInfo;
use oas_core::OasError;
use oas_protocol::{CoreToWorker, Envelope, WorkerToCore};

fn emit(msg: WorkerToCore) {
    if let Ok(line) = Envelope::new(msg).to_line() {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = writeln!(lock, "{line}");
        let _ = lock.flush();
    }
}

fn main() {
    emit(WorkerToCore::Ready {
        worker_version: "fake".into(),
    });

    let cancelled = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let flag = Arc::clone(&cancelled);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match Envelope::<CoreToWorker>::from_line(&line).map(|e| e.msg) {
                Ok(CoreToWorker::Start {
                    audio_path,
                    resume_from_sec,
                    limits,
                    ..
                }) => {
                    let _ = tx.send((audio_path, resume_from_sec, limits.use_gpu));
                }
                Ok(CoreToWorker::Cancel) => flag.store(true, Ordering::SeqCst),
                Ok(CoreToWorker::Probe { paths }) if paths.iter().any(|p| p.contains("hang")) => {
                    // Never answers, as a parser stuck on a hostile file would.
                }
                Ok(CoreToWorker::Probe { paths }) => {
                    emit(WorkerToCore::Durations {
                        seconds: paths.iter().map(|_| Some(12.5)).collect(),
                    });
                    std::process::exit(0);
                }
                Err(_) => {}
            }
        }
        // End of stdin: the core is gone, so exit whatever we were doing.
        std::process::exit(0);
    });

    let Ok((audio, resume, use_gpu)) = rx.recv() else {
        return;
    };

    if audio.contains("gpubad") && use_gpu {
        emit(WorkerToCore::Error {
            error: ErrorInfo::from(&OasError::GpuInitFailed {
                reason: "fake".into(),
            }),
        });
        return;
    }

    if audio.contains("fail") {
        emit(WorkerToCore::Error {
            error: ErrorInfo::from(&OasError::DecodeFailed {
                reason: "fake".into(),
            }),
        });
        return;
    }

    let (total, delay_ms) = if audio.contains("slow") {
        (200u32, 25)
    } else {
        (3, 2)
    };
    emit(WorkerToCore::Audio {
        duration_sec: total as f64,
    });

    let first = resume.unwrap_or(0.0).round() as u32;
    for i in first..total {
        if cancelled.load(Ordering::SeqCst) && !audio.contains("hang") {
            return;
        }
        emit(WorkerToCore::Segment {
            start: i as f64,
            end: i as f64 + 1.0,
            text: format!("line {i}"),
            words: vec![],
        });
        emit(WorkerToCore::Progress {
            pct: (i + 1) as f32 / total as f32 * 100.0,
            stage: "transcribing".into(),
        });
        if audio.contains("crash") {
            std::process::exit(3);
        }
        std::thread::sleep(Duration::from_millis(delay_ms));
    }

    emit(WorkerToCore::Metrics {
        rtf: 0.1,
        peak_rss_mib: 1,
    });
    emit(WorkerToCore::Done);
}
