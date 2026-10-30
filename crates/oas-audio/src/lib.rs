//! Decode arbitrary audio files to 16 kHz mono f32 (what Whisper expects), and
//! split long audio into <=30 s chunks on silence boundaries.
//!
//! Decoding uses Symphonia (pure Rust, no external binary). Resampling uses a
//! built-in linear resampler with box-averaging when downsampling — adequate
//! for 16 kHz speech; a higher-quality sinc resampler (rubato) can replace it
//! later (see ADR-04).

#![forbid(unsafe_code)]

mod chunk;
mod decode;
mod resample;

pub use chunk::{chunk_audio, AudioChunk, ChunkOptions};
pub use decode::decode_file;
pub use resample::resample_linear;

/// The sample rate Whisper and the rest of the pipeline operate at.
pub const TARGET_RATE: u32 = 16_000;

/// Decoded audio: mono f32 samples at [`TARGET_RATE`].
#[derive(Debug, Clone, PartialEq)]
pub struct AudioPcm {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl AudioPcm {
    /// Duration in seconds.
    pub fn duration_sec(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.samples.len() as f32 / self.sample_rate as f32
        }
    }
}
