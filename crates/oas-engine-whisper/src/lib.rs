//! whisper.cpp backend via `whisper-rs` (CPU build).
//!
//! Loads a ggml model and transcribes 16 kHz mono f32 audio, normalising output
//! to [`oas_engine::Segment`]. Cancellation is cooperative through whisper's
//! abort callback. GPU backends are added in M07.

#![forbid(unsafe_code)]

use std::path::Path;
use std::time::Instant;

use oas_core::{OasError, Result};
use oas_engine::{Callbacks, CancelToken, Engine, RunOptions, RunSummary, Segment};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn map_err(e: whisper_rs::WhisperError) -> OasError {
    OasError::Internal {
        reason: format!("whisper engine error: {e}"),
    }
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
        callbacks: &mut Callbacks<'_>,
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
        // Keep whisper.cpp quiet on stdout/stderr.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // Cooperative cancellation. NOTE: this ggml abort callback returns
        // `true` to CONTINUE and `false` to ABORT (the opposite of older
        // whisper.cpp docs — verified empirically: returning the cancelled flag
        // directly aborted every run). So return "not cancelled".
        let token = cancel.clone();
        params.set_abort_callback_safe(move || !token.is_cancelled());

        (callbacks.on_progress)(0.0);
        let start = Instant::now();
        state.full(params, audio).map_err(map_err)?;

        let n = state.full_n_segments();
        for i in 0..n {
            if cancel.is_cancelled() {
                break;
            }
            if let Some(seg) = state.get_segment(i) {
                let text = seg.to_str_lossy().map_err(map_err)?.trim().to_string();
                let t0 = seg.start_timestamp() as f64 * 0.01;
                let t1 = seg.end_timestamp() as f64 * 0.01;
                if !text.is_empty() {
                    (callbacks.on_segment)(Segment {
                        start: t0,
                        end: t1,
                        text,
                        words: vec![],
                    });
                }
            }
        }
        (callbacks.on_progress)(1.0);

        let audio_sec = audio.len() as f32 / rate.max(1) as f32;
        let rtf = start.elapsed().as_secs_f32() / audio_sec.max(0.001);
        Ok(RunSummary {
            rtf,
            peak_rss_mib: 0,
            audio_sec,
        })
    }
}
