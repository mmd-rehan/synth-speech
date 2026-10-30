//! Energy-based voice activity detection and chunking.
//!
//! Splits audio into windows of at most `max_sec`, preferring to cut at the
//! quietest frame near the window edge so we don't slice through a word.
//! Fully-silent windows are dropped. This is a simple energy VAD behind a
//! stable interface; a learned VAD (Silero ONNX) can replace it later.

/// Chunking parameters.
#[derive(Debug, Clone, Copy)]
pub struct ChunkOptions {
    pub max_sec: f32,
    pub overlap_sec: f32,
    pub frame_ms: f32,
    /// RMS below this is treated as silence.
    pub silence_rms: f32,
    /// How far back from the window edge to look for a silent cut point.
    pub search_sec: f32,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        ChunkOptions {
            max_sec: 30.0,
            overlap_sec: 0.5,
            frame_ms: 30.0,
            silence_rms: 0.01,
            search_sec: 4.0,
        }
    }
}

/// A chunk of audio with its position in the original timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub start_sec: f32,
    pub end_sec: f32,
    pub samples: Vec<f32>,
}

fn frame_rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = frame.iter().map(|s| s * s).sum();
    (sum_sq / frame.len() as f32).sqrt()
}

/// Split `samples` (mono at `rate`) into chunks.
pub fn chunk_audio(samples: &[f32], rate: u32, opts: &ChunkOptions) -> Vec<AudioChunk> {
    if samples.is_empty() || rate == 0 {
        return Vec::new();
    }
    let n = samples.len();
    let frame_len = ((rate as f32) * opts.frame_ms / 1000.0).max(1.0) as usize;
    let max_len = ((rate as f32) * opts.max_sec) as usize;
    let overlap_len = ((rate as f32) * opts.overlap_sec) as usize;
    let search_len = ((rate as f32) * opts.search_sec) as usize;

    // Per-frame RMS, indexed by frame.
    let rms: Vec<f32> = samples.chunks(frame_len).map(frame_rms).collect();
    let frame_rms_at =
        |sample_idx: usize| -> f32 { *rms.get(sample_idx / frame_len).unwrap_or(&0.0) };

    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < n {
        let target_end = (start + max_len).min(n);
        let cut = if target_end >= n {
            n
        } else {
            // Find the quietest frame boundary within the search window.
            let search_from = target_end.saturating_sub(search_len).max(start + frame_len);
            let mut best = target_end;
            let mut best_rms = f32::MAX;
            let mut idx = search_from;
            while idx < target_end {
                let r = frame_rms_at(idx);
                if r < best_rms {
                    best_rms = r;
                    best = idx;
                }
                idx += frame_len;
            }
            best
        };

        let slice = &samples[start..cut];
        let max_rms = slice
            .chunks(frame_len)
            .map(frame_rms)
            .fold(0.0f32, f32::max);
        // Drop pure-silence chunks.
        if max_rms >= opts.silence_rms {
            chunks.push(AudioChunk {
                start_sec: start as f32 / rate as f32,
                end_sec: cut as f32 / rate as f32,
                samples: slice.to_vec(),
            });
        }

        if cut >= n {
            break;
        }
        // Advance, keeping a small overlap, but always make progress.
        let next = cut.saturating_sub(overlap_len);
        start = if next > start { next } else { cut };
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    fn tone(secs: f32, amp: f32) -> Vec<f32> {
        let n = (secs * RATE as f32) as usize;
        (0..n).map(|i| (i as f32 * 0.05).sin() * amp).collect()
    }

    #[test]
    fn short_audio_is_one_chunk() {
        let a = tone(5.0, 0.5);
        let chunks = chunk_audio(&a, RATE, &ChunkOptions::default());
        assert_eq!(chunks.len(), 1);
        assert!((chunks[0].end_sec - 5.0).abs() < 0.1);
    }

    #[test]
    fn long_audio_splits_into_bounded_chunks() {
        let a = tone(70.0, 0.5);
        let opts = ChunkOptions::default();
        let chunks = chunk_audio(&a, RATE, &opts);
        assert!(chunks.len() >= 3, "got {}", chunks.len());
        for c in &chunks {
            assert!(
                c.end_sec - c.start_sec <= opts.max_sec + 0.01,
                "chunk {:.1}-{:.1} too long",
                c.start_sec,
                c.end_sec
            );
        }
        // Chunks cover the whole timeline.
        assert!((chunks.last().unwrap().end_sec - 70.0).abs() < 0.1);
    }

    #[test]
    fn pure_silence_yields_no_chunks() {
        let a = tone(5.0, 0.0); // all zeros
        let chunks = chunk_audio(&a, RATE, &ChunkOptions::default());
        assert!(chunks.is_empty());
    }

    #[test]
    fn prefers_cutting_at_a_silent_gap() {
        // 29 s loud, 2 s silent, 29 s loud. The first cut should land in the gap.
        let mut a = tone(29.0, 0.5);
        a.extend(tone(2.0, 0.0));
        a.extend(tone(29.0, 0.5));
        let chunks = chunk_audio(&a, RATE, &ChunkOptions::default());
        // First chunk should end inside the silent gap (29..31 s).
        let end = chunks[0].end_sec;
        assert!((29.0..=31.0).contains(&end), "first cut at {end:.1}s");
    }
}
