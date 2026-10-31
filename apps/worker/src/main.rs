//! `oas-worker`: the inference worker process, shipped as a Tauri sidecar.
//!
//! Protocol (JSONL over stdio): emits `ready` at startup, then on a `start`
//! message decodes the audio, runs the engine, and streams `progress` /
//! `segment` events, finishing with `metrics` + `done` (or `error`). A `cancel`
//! message stops the current job. Engine panics are caught and reported as
//! `error` rather than crashing silently.
//!
//! stdin is read on its own thread so `cancel` is seen while the engine runs.
//! End of stdin means the core is gone (it exited or crashed), so the worker
//! exits at once. This is what prevents orphan workers on every platform.

#![forbid(unsafe_code)]

use std::io::{BufRead, Write};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

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

/// What the worker was asked to do.
enum Work {
    Job(StartRequest),
    Probe(Vec<String>),
}

/// Everything a `start` message asks for.
struct StartRequest {
    audio_path: String,
    model_path: String,
    engine: EngineKind,
    options: JobOptions,
    limits: Limits,
    resume_from_sec: Option<f64>,
}

/// Read stdin until it closes: route `cancel` to the token and `start` to the
/// main thread. If stdin closes while a job runs, the core died, so exit now.
fn spawn_stdin_reader(cancel: CancelToken, job_running: Arc<AtomicBool>, work: mpsc::Sender<Work>) {
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
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
                        resume_from_sec,
                        ..
                    } => {
                        job_running.store(true, Ordering::SeqCst);
                        let _ = work.send(Work::Job(StartRequest {
                            audio_path,
                            model_path,
                            engine,
                            options,
                            limits,
                            resume_from_sec,
                        }));
                    }
                    CoreToWorker::Probe { paths } => {
                        let _ = work.send(Work::Probe(paths));
                    }
                    CoreToWorker::Cancel => cancel.cancel(),
                },
                Err(e) => emit_error(&e),
            }
        }
        if job_running.load(Ordering::SeqCst) {
            std::process::exit(0);
        }
        // Idle worker: dropping `work` lets main return normally.
    });
}

fn main() -> ExitCode {
    emit(WorkerToCore::Ready {
        worker_version: env!("CARGO_PKG_VERSION").to_string(),
    });

    let cancel = CancelToken::new();
    let (tx, rx) = mpsc::channel();
    spawn_stdin_reader(cancel.clone(), Arc::new(AtomicBool::new(false)), tx);

    // One task per worker process; exit after it finishes.
    match rx.recv() {
        Ok(Work::Job(request)) => run_job(request, &cancel),
        Ok(Work::Probe(paths)) => emit(WorkerToCore::Durations {
            seconds: paths
                .iter()
                .map(|p| oas_audio::probe_duration_sec(&PathBuf::from(p)))
                .collect(),
        }),
        Err(_) => {}
    }
    ExitCode::SUCCESS
}

fn run_job(request: StartRequest, cancel: &CancelToken) {
    let StartRequest {
        audio_path,
        model_path,
        engine,
        options,
        limits,
        resume_from_sec,
    } = request;

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
    let duration_sec = pcm.samples.len() as f64 / pcm.sample_rate.max(1) as f64;
    emit(WorkerToCore::Audio { duration_sec });

    // The share of the file already done before this run started.
    let offset = resume_from_sec.unwrap_or(0.0).max(0.0);
    let done_share = (offset / duration_sec.max(0.001)).min(1.0) as f32;
    emit(WorkerToCore::Progress {
        pct: overall_pct(done_share, 0.0),
        stage: "decoded".into(),
    });

    // Paused right at the end: nothing is left to transcribe.
    if offset + 0.1 >= duration_sec && resume_from_sec.is_some() {
        emit(WorkerToCore::Done);
        return;
    }
    if cancel.is_cancelled() {
        return;
    }

    // Load the model.
    let mut whisper = match WhisperEngine::load(&PathBuf::from(&model_path), limits.use_gpu) {
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
        start_offset_sec: resume_from_sec.map(|s| s as f32),
    };

    let callbacks = Callbacks {
        on_segment: Box::new(|seg: oas_engine::Segment| {
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
        }),
        on_progress: Box::new(move |p: f32| {
            emit(WorkerToCore::Progress {
                pct: overall_pct(done_share, p),
                stage: "transcribing".into(),
            });
        }),
    };

    // Catch engine panics (native crashes surface as unwraps/panics) and report
    // them as a structured error instead of aborting the process silently.
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        whisper.transcribe(&pcm.samples, pcm.sample_rate, &run_opts, cancel, callbacks)
    }));

    // A cancelled run aborts inside the engine; that is not a failure. The core
    // knows it asked for this, and treats the plain exit as the acknowledgement.
    if cancel.is_cancelled() {
        return;
    }

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

/// Whole-file progress in percent: the first 5% is decoding, the rest is split
/// between the part already done before this run and this run's progress `p`.
fn overall_pct(done_share: f32, p: f32) -> f32 {
    5.0 + (done_share + (1.0 - done_share) * p.clamp(0.0, 1.0)) * 95.0
}
