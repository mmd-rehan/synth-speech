//! The abstraction every model runner implements, plus the shared types that
//! cross between the audio pipeline, the engines, and the exporters.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use oas_core::Result;
use serde::{Deserialize, Serialize};

/// A single timed word inside a segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A finished transcript segment (a line of text with a time span).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
}

/// Options for a transcription run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunOptions {
    /// BCP-47 language hint, or `None` to auto-detect.
    pub language: Option<String>,
    /// Translate to English instead of transcribing in the source language.
    pub translate: bool,
    /// Emit word-level timestamps when supported.
    pub word_timestamps: bool,
    /// Beam size (engine default when `None`).
    pub beam_size: Option<u32>,
    /// CPU threads (engine default when `None`).
    pub threads: Option<u32>,
    /// Initial prompt to bias decoding.
    pub initial_prompt: Option<String>,
    /// Skip audio before this many seconds (used to resume a paused job).
    /// Timestamps in the output stay relative to the start of the file.
    pub start_offset_sec: Option<f32>,
}

/// Summary metrics produced at the end of a run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunSummary {
    /// Real-time factor: processing time / audio duration.
    pub rtf: f32,
    /// Peak resident memory in mebibytes (0 if not measured).
    pub peak_rss_mib: u32,
    /// Audio duration processed, in seconds.
    pub audio_sec: f32,
}

/// A cooperative cancellation flag shared with a running engine.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        CancelToken(Arc::new(AtomicBool::new(false)))
    }
    /// Request cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Callbacks an engine invokes as it produces output, *while* it runs. They are
/// owned because native backends keep them for the length of the run.
pub struct Callbacks {
    /// Called for each finished segment, as soon as it is finished.
    pub on_segment: Box<dyn FnMut(Segment)>,
    /// Called with progress in \[0, 1] for this run.
    pub on_progress: Box<dyn FnMut(f32)>,
}

/// A speech-to-text engine. Implemented by backends such as whisper.cpp.
pub trait Engine {
    /// A stable identifier, e.g. `whisper.cpp`.
    fn id(&self) -> &str;

    /// Transcribe mono f32 `audio` sampled at `rate`, invoking callbacks as
    /// output is produced. Must poll `cancel` and stop promptly when set; a
    /// cancelled run may return an error, so callers check `cancel` first.
    fn transcribe(
        &mut self,
        audio: &[f32],
        rate: u32,
        opts: &RunOptions,
        cancel: &CancelToken,
        callbacks: Callbacks,
    ) -> Result<RunSummary>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_token_round_trips() {
        let t = CancelToken::new();
        assert!(!t.is_cancelled());
        let clone = t.clone();
        clone.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn segment_json_round_trips() {
        let seg = Segment {
            start: 0.0,
            end: 1.5,
            text: "hello".into(),
            words: vec![Word {
                start: 0.0,
                end: 0.5,
                text: "hello".into(),
            }],
        };
        let json = serde_json::to_string(&seg).unwrap();
        let back: Segment = serde_json::from_str(&json).unwrap();
        assert_eq!(seg, back);
    }
}
