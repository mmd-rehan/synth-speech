//! whisper.cpp backend via `whisper-rs` (CPU build).
//!
//! Loads a ggml model and transcribes 16 kHz mono f32 audio, normalising output
//! to [`oas_engine::Segment`]. Cancellation is cooperative through whisper's
//! abort callback. GPU backends are added in M07.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use oas_core::{OasError, Result};
use oas_engine::{Callbacks, CancelToken, Engine, RunOptions, RunSummary, Segment};
use whisper_rs::{
    FullParams, SamplingStrategy, SegmentCallbackData, WhisperContext, WhisperContextParameters,
};

fn map_err(e: whisper_rs::WhisperError) -> OasError {
    OasError::Internal {
        reason: format!("whisper engine error: {e}"),
    }
}

/// The token of the run in progress. A process runs one transcription at a time
/// (a worker runs one job and exits, and the CLI runs one file).
static RUN_CANCEL: Mutex<Option<CancelToken>> = Mutex::new(None);

/// The abort callback: `true` stops the run.
///
/// This must stay a plain function and never become a capturing closure.
/// whisper-rs 0.16's `set_abort_callback_safe` stores a boxed trait object but
/// reads the pointer back as the closure's own type, so a capture would be read
/// from unrelated heap memory and the run would abort at random (ADR-17). A
/// function has no data to misread, which is why the token lives in a static.
fn run_was_cancelled() -> bool {
    RUN_CANCEL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(CancelToken::is_cancelled)
}

/// The value handed to whisper-rs, typed so that it stays the zero-sized
/// function item. Casting it to a `fn()` pointer would give the bad read above
/// eight bytes to misuse, and crash.
fn abort_callback() -> impl FnMut() -> bool + 'static {
    run_was_cancelled
}

/// A loaded whisper.cpp model ready to transcribe.
pub struct WhisperEngine {
    ctx: WhisperContext,
    id: String,
}

impl WhisperEngine {
    /// Load a ggml model file.
    pub fn load(model_path: &Path) -> Result<Self> {
        let path = model_path.to_str().ok_or_else(|| OasError::InvalidInput {
            reason: "model path is not valid UTF-8".into(),
        })?;
        if !model_path.exists() {
            return Err(OasError::ModelNotInstalled {
                model_id: model_path.display().to_string(),
            });
        }
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(map_err)?;
        Ok(WhisperEngine {
            ctx,
            id: "whisper.cpp".to_string(),
        })
    }
}

impl Engine for WhisperEngine {
    fn id(&self) -> &str {
        &self.id
    }

    fn transcribe(
        &mut self,
        audio: &[f32],
        rate: u32,
        opts: &RunOptions,
        cancel: &CancelToken,
        callbacks: Callbacks,
    ) -> Result<RunSummary> {
        let mut state = self.ctx.create_state().map_err(map_err)?;

        let strategy = match opts.beam_size {
            Some(b) if b > 1 => SamplingStrategy::BeamSearch {
                beam_size: b as i32,
                patience: -1.0,
            },
            _ => SamplingStrategy::Greedy { best_of: 1 },
        };
        let mut params = FullParams::new(strategy);
        if let Some(threads) = opts.threads {
            params.set_n_threads(threads as i32);
        }
        params.set_translate(opts.translate);
        params.set_language(opts.language.as_deref());
        params.set_token_timestamps(opts.word_timestamps);
        if let Some(prompt) = &opts.initial_prompt {
            params.set_initial_prompt(prompt);
        }
        if let Some(offset) = opts.start_offset_sec {
            params.set_offset_ms((offset.max(0.0) * 1000.0) as i32);
        }
        // Keep whisper.cpp quiet on stdout/stderr.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // Cooperative cancellation.
        *RUN_CANCEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel.clone());
        params.set_abort_callback_safe(abort_callback());

        // Stream output as it is produced, so a crash keeps partial results and
        // the UI can show live text. Timestamps arrive in 10 ms units.
        let Callbacks {
            mut on_segment,
            mut on_progress,
        } = callbacks;
        params.set_segment_callback_safe_lossy(move |data: SegmentCallbackData| {
            let text = data.text.trim().to_string();
            if !text.is_empty() {
                on_segment(Segment {
                    start: data.start_timestamp as f64 * 0.01,
                    end: data.end_timestamp as f64 * 0.01,
                    text,
                    words: vec![],
                });
            }
        });
        params.set_progress_callback_safe(move |pct: i32| on_progress(pct as f32 / 100.0));

        let start = Instant::now();
        state.full(params, audio).map_err(map_err)?;

        let audio_sec = audio.len() as f32 / rate.max(1) as f32;
        let rtf = start.elapsed().as_secs_f32() / audio_sec.max(0.001);
        Ok(RunSummary {
            rtf,
            peak_rss_mib: 0,
            audio_sec,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Guards the workaround: with no data, the callback cannot read bad memory.
    #[test]
    fn abort_callback_carries_no_data() {
        assert_eq!(std::mem::size_of_val(&abort_callback()), 0);
    }
}
