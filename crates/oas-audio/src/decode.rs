//! Decode an audio file to 16 kHz mono f32 using Symphonia.

use std::path::Path;

use oas_core::{OasError, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{resample::resample_linear, AudioPcm, TARGET_RATE};

fn decode_failed(reason: impl std::fmt::Display) -> OasError {
    OasError::DecodeFailed {
        reason: reason.to_string(),
    }
}

/// Decode `path` into mono f32 samples at [`TARGET_RATE`].
pub fn decode_file(path: &Path) -> Result<AudioPcm> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| decode_failed(format!("unrecognised or corrupt audio: {e}")))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| decode_failed("no audio track in file"))?;
    let track_id = track.id;
    let src_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| decode_failed("audio has no sample rate"))?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| decode_failed(format!("no decoder for this codec: {e}")))?;

    let mut mono: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Clean end of stream.
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(decode_failed(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                if sample_buf.is_none() {
                    let spec = *decoded.spec();
                    let dur = decoded.capacity() as u64;
                    sample_buf = Some(SampleBuffer::<f32>::new(dur, spec));
                }
                let channels = decoded.spec().channels.count().max(1);
                if let Some(buf) = sample_buf.as_mut() {
                    buf.copy_interleaved_ref(decoded);
                    for frame in buf.samples().chunks(channels) {
                        let sum: f32 = frame.iter().copied().sum();
                        mono.push(sum / channels as f32);
                    }
                }
            }
            // A single bad packet is skipped, not fatal.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(decode_failed(e)),
        }
    }

    if mono.is_empty() {
        return Err(decode_failed("decoded zero audio samples"));
    }

    let samples = resample_linear(&mono, src_rate, TARGET_RATE);
    Ok(AudioPcm {
        samples,
        sample_rate: TARGET_RATE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a minimal 16-bit PCM WAV (RIFF) and return its path.
    fn write_wav(
        dir: &Path,
        name: &str,
        rate: u32,
        channels: u16,
        samples: &[i16],
    ) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        let bits = 16u16;
        let block_align = channels * bits / 8;
        let byte_rate = rate * block_align as u32;
        let data_len = (samples.len() * 2) as u32;
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
        f.write_all(&channels.to_le_bytes()).unwrap();
        f.write_all(&rate.to_le_bytes()).unwrap();
        f.write_all(&byte_rate.to_le_bytes()).unwrap();
        f.write_all(&block_align.to_le_bytes()).unwrap();
        f.write_all(&bits.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_len.to_le_bytes()).unwrap();
        for s in samples {
            f.write_all(&s.to_le_bytes()).unwrap();
        }
        path
    }

    fn tmpdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("oas-audio-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn decodes_stereo_44k_wav_to_mono_16k() {
        let dir = tmpdir();
        // 1 second of 44.1 kHz stereo: left = +amp, right = -amp → mono ~0.
        let rate = 44100u32;
        let mut samples = Vec::new();
        for i in 0..rate {
            let v = ((i as f32 / 100.0).sin() * 10000.0) as i16;
            samples.push(v); // L
            samples.push(v); // R (same → mono = v)
        }
        let path = write_wav(&dir, "test.wav", rate, 2, &samples);
        let pcm = decode_file(&path).expect("decode");
        assert_eq!(pcm.sample_rate, 16000);
        // ~1 second at 16 kHz.
        assert!(
            (pcm.samples.len() as i64 - 16000).abs() < 50,
            "len {}",
            pcm.samples.len()
        );
        assert!((pcm.duration_sec() - 1.0).abs() < 0.01);
    }

    #[test]
    fn corrupt_file_is_decode_failed() {
        let dir = tmpdir();
        let path = dir.join("bad.wav");
        std::fs::write(&path, b"not really a wav file at all").unwrap();
        let err = decode_file(&path).unwrap_err();
        assert_eq!(err.code(), "DECODE_FAILED");
    }

    #[test]
    fn missing_file_is_file_not_found() {
        let err = decode_file(Path::new("/no/such/audio.wav")).unwrap_err();
        assert_eq!(err.code(), "FILE_NOT_FOUND");
    }
}
