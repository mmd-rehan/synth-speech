//! `oas-worker`: the inference worker process, shipped as a Tauri sidecar.
//!
//! Protocol (JSONL over stdio): emits `ready` at startup, then on a `start`
//! message decodes the audio, runs the engine, and streams `progress` /
//! `segment` events, finishing with `metrics` + `done` (or `error`). A `cancel`
//! message stops the current job. Engine panics are caught and reported as
//! `error` rather than crashing silently.

#![forbid(unsafe_code)]

use std::io::{BufRead, Write};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;

use oas_core::error::ErrorInfo;
use oas_core::OasError;
use oas_engine::{Callbacks, CancelToken, Engine, RunOptions};
use oas_engine_whisper::WhisperEngine;
use oas_protocol::{CoreToWorker, EngineKind, Envelope, JobOptions, Limits, WorkerToCore};

/// Serialise stdout writes so concurrent emits never interleave.
static STDOUT_LOCK: Mutex<()> = Mutex::new(());

/// Write one `WorkerToCore` message as a JSONL line.
fn emit(msg: WorkerToCore) {
    let env = Envelope::new(msg);
    if let Ok(line) = env.to_line() {
        let _guard = STDOUT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = writeln!(lock, "{line}");
        let _ = lock.flush();
    }
}

fn emit_error(err: &OasError) {
    emit(WorkerToCore::Error {
        error: ErrorInfo::from(err),
    });
}

fn main() -> ExitCode {
    emit(WorkerToCore::Ready {
        worker_version: env!("CARGO_PKG_VERSION").to_string(),
    });

    let cancel = CancelToken::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        match Envelope::<CoreToWorker>::from_line(&line) {
            Ok(env) => match env.msg {
                CoreToWorker::Start {
                    audio_path,
                    model_path,
                    engine,
                    options,
                    limits,
                    ..
                } => {
                    // Run synchronously; a `cancel` can still arrive because the
                    // engine polls the shared token from its abort callback on
                    // its own worker threads.
                    run_job(audio_path, model_path, engine, options, limits, &cancel);
                    // One job per worker process; exit after it finishes.
                    return ExitCode::SUCCESS;
                }
                CoreToWorker::Cancel => {
                    cancel.cancel();
                }
            },
            Err(e) => emit_error(&e),
        }
    }

    ExitCode::SUCCESS
}

fn run_job(
    audio_path: String,
    model_path: String,
    engine: EngineKind,
    options: JobOptions,
    limits: Limits,
    cancel: &CancelToken,
) {
    if engine != EngineKind::WhisperCpp {
        emit_error(&OasError::InvalidInput {
            reason: "this worker build only supports the whisper.cpp engine".into(),
        });
        return;
    }

    // Decode audio to 16 kHz mono f32.
    let pcm = match oas_audio::decode_file(&PathBuf::from(&audio_path)) {
        Ok(p) => p,
        Err(e) => return emit_error(&e),
    };
    emit(WorkerToCore::Progress {
        pct: 5.0,
        stage: "decoded".into(),
    });

    // Load the model.
    let mut whisper = match WhisperEngine::load(&PathBuf::from(&model_path)) {
        Ok(e) => e,
        Err(e) => return emit_error(&e),
    };

    let run_opts = RunOptions {
        language: options.language.clone(),
        translate: options.translate,
        word_timestamps: options.word_timestamps,
        beam_size: None,
        threads: Some(limits.max_threads),
        initial_prompt: None,
    };

    let mut on_segment = |seg: oas_engine::Segment| {
        emit(WorkerToCore::Segment {
            start: seg.start,
            end: seg.end,
            text: seg.text,
            words: seg
                .words
                .into_iter()
                .map(|w| oas_protocol::Word {
                    start: w.start,
                    end: w.end,
                    text: w.text,
                })
                .collect(),
        });
    };
    let mut on_progress = |p: f32| {
        emit(WorkerToCore::Progress {
            // Map engine [0,1] onto the 5..100 band after decode.
            pct: 5.0 + p * 95.0,
            stage: "transcribing".into(),
        });
    };

    // Catch engine panics (native crashes surface as unwraps/panics) and report
    // them as a structured error instead of aborting the process silently.
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut callbacks = Callbacks {
            on_segment: &mut on_segment,
            on_progress: &mut on_progress,
        };
        whisper.transcribe(
            &pcm.samples,
            pcm.sample_rate,
            &run_opts,
            cancel,
            &mut callbacks,
        )
    }));

    match result {
        Ok(Ok(summary)) => {
            emit(WorkerToCore::Metrics {
                rtf: summary.rtf,
                peak_rss_mib: summary.peak_rss_mib,
            });
            emit(WorkerToCore::Done);
        }
        Ok(Err(e)) => emit_error(&e),
        Err(_) => emit_error(&OasError::Internal {
            reason: "the engine crashed while transcribing".into(),
        }),
    }
}
