//! The wire protocol spoken between the core and an `oas-worker` process.
//!
//! One JSON object per line (JSONL) on the worker's stdin (core -> worker) and
//! stdout (worker -> core). Every message carries the protocol version `v` and
//! a `type` tag so both sides can reject mismatches early.
//!
//! This crate is deliberately tiny and depends only on `oas-core`, so both the
//! core and the worker binary can share the exact same types.

#![forbid(unsafe_code)]

use oas_core::error::ErrorInfo;
use serde::{Deserialize, Serialize};

/// Current protocol version. Bump on any breaking change to the messages.
pub const PROTOCOL_VERSION: u32 = 1;

/// Which inference backend the worker should use for a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    /// whisper.cpp via `whisper-rs` (Phase 1 primary engine).
    WhisperCpp,
    /// sherpa-onnx (Phase 2).
    SherpaOnnx,
}

/// Per-job options that affect transcription output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobOptions {
    /// BCP-47 language hint, or `None` to auto-detect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Translate to English rather than transcribe in the source language.
    #[serde(default)]
    pub translate: bool,
    /// Emit word-level timestamps when the engine supports it.
    #[serde(default)]
    pub word_timestamps: bool,
}

/// Resource limits the core imposes on a single worker. The governor fills
/// these in; the worker must respect them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Maximum CPU threads the engine may use.
    pub max_threads: u32,
    /// Soft memory budget in mebibytes; the worker should stay under it.
    pub mem_budget_mib: u32,
    /// Whether the worker may use the GPU for this job.
    pub use_gpu: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_threads: 1,
            mem_budget_mib: 1024,
            use_gpu: false,
        }
    }
}

/// Messages sent from the core to the worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreToWorker {
    /// Begin a job. All paths are absolute and already validated by the core.
    Start {
        job_id: String,
        audio_path: String,
        model_path: String,
        engine: EngineKind,
        #[serde(default)]
        options: JobOptions,
        limits: Limits,
        /// Resume point: skip audio before this time. Used to continue a job
        /// that was paused, keeping the segments already stored.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume_from_sec: Option<f64>,
    },
    /// Read the length of these files from their headers, reply with
    /// `durations`, and exit. Media is only ever parsed in a worker.
    Probe { paths: Vec<String> },
    /// Ask the worker to stop the current job and exit cleanly.
    Cancel,
}

/// Messages sent from the worker back to the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerToCore {
    /// Sent once at startup to confirm the worker launched successfully.
    Ready {
        /// The worker binary version, for diagnostics.
        worker_version: String,
    },
    /// The audio was decoded; its full length is now known.
    Audio { duration_sec: f64 },
    /// Reply to `probe`, in the same order. `None` for an unreadable file.
    Durations { seconds: Vec<Option<f64>> },
    /// Coarse progress update.
    Progress { pct: f32, stage: String },
    /// A finished transcript segment.
    Segment {
        start: f64,
        end: f64,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        words: Vec<Word>,
    },
    /// Performance metrics emitted near the end of a job.
    Metrics {
        /// Real-time factor: processing seconds per second of audio.
        rtf: f32,
        /// Peak resident memory in mebibytes.
        peak_rss_mib: u32,
    },
    /// The job finished successfully.
    Done,
    /// The job failed. `error` carries a stable code and a user message.
    Error { error: ErrorInfo },
}

/// A single timed word within a segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// An addressed, versioned message. Serialising this is what goes on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// Protocol version. Receivers reject anything they do not understand.
    pub v: u32,
    #[serde(flatten)]
    pub msg: T,
}

impl<T> Envelope<T> {
    /// Wrap a message with the current protocol version.
    pub fn new(msg: T) -> Self {
        Envelope {
            v: PROTOCOL_VERSION,
            msg,
        }
    }
}

impl<T: Serialize> Envelope<T> {
    /// Encode as a single JSONL line (no trailing newline).
    pub fn to_line(&self) -> oas_core::Result<String> {
        serde_json::to_string(self).map_err(|e| oas_core::OasError::Protocol {
            reason: format!("failed to encode message: {e}"),
        })
    }
}

impl<T: for<'de> Deserialize<'de>> Envelope<T> {
    /// Parse one JSONL line, rejecting unknown protocol versions.
    pub fn from_line(line: &str) -> oas_core::Result<Self> {
        let env: Envelope<T> =
            serde_json::from_str(line).map_err(|e| oas_core::OasError::Protocol {
                reason: format!("failed to decode message: {e}"),
            })?;
        if env.v != PROTOCOL_VERSION {
            return Err(oas_core::OasError::Protocol {
                reason: format!(
                    "unsupported protocol version {} (this build speaks {})",
                    env.v, PROTOCOL_VERSION
                ),
            });
        }
        Ok(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_roundtrips_as_jsonl() {
        let env = Envelope::new(WorkerToCore::Ready {
            worker_version: "0.1.0".into(),
        });
        let line = env.to_line().unwrap();
        assert!(line.contains("\"type\":\"ready\""));
        assert!(line.contains("\"v\":1"));
        let back: Envelope<WorkerToCore> = Envelope::from_line(&line).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn start_roundtrips_with_paths_containing_spaces() {
        let env = Envelope::new(CoreToWorker::Start {
            job_id: "job-1".into(),
            audio_path: "/Users/a b/My Clips/hello world.wav".into(),
            model_path: "/models/whisper tiny/ggml.bin".into(),
            engine: EngineKind::WhisperCpp,
            options: JobOptions {
                language: Some("en".into()),
                translate: false,
                word_timestamps: true,
            },
            limits: Limits {
                max_threads: 4,
                mem_budget_mib: 2048,
                use_gpu: true,
            },
            resume_from_sec: Some(12.5),
        });
        let line = env.to_line().unwrap();
        let back: Envelope<CoreToWorker> = Envelope::from_line(&line).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn wrong_version_is_rejected() {
        let line = r#"{"v":999,"type":"ready","worker_version":"0.1.0"}"#;
        let err = Envelope::<WorkerToCore>::from_line(line).unwrap_err();
        assert_eq!(err.code(), "PROTOCOL_ERROR");
    }

    #[test]
    fn garbage_is_a_protocol_error() {
        let err = Envelope::<WorkerToCore>::from_line("not json").unwrap_err();
        assert_eq!(err.code(), "PROTOCOL_ERROR");
    }

    use proptest::prelude::*;

    proptest! {
        // Decoding an arbitrary line must never panic — it either parses or
        // returns a PROTOCOL_ERROR.
        #[test]
        fn arbitrary_lines_never_panic(line in ".{0,400}") {
            match Envelope::<WorkerToCore>::from_line(&line) {
                Ok(_) => {}
                Err(e) => prop_assert_eq!(e.code(), "PROTOCOL_ERROR"),
            }
            match Envelope::<CoreToWorker>::from_line(&line) {
                Ok(_) => {}
                Err(e) => prop_assert_eq!(e.code(), "PROTOCOL_ERROR"),
            }
        }

        // Any envelope with a well-formed but wrong version is rejected cleanly.
        #[test]
        fn wrong_versions_rejected(v in 2u32..u32::MAX) {
            let line = format!("{{\"v\":{v},\"type\":\"ready\",\"worker_version\":\"x\"}}");
            let err = Envelope::<WorkerToCore>::from_line(&line).unwrap_err();
            prop_assert_eq!(err.code(), "PROTOCOL_ERROR");
        }
    }
}
