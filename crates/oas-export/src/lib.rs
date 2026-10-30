//! Transcript exporters. Given a list of [`Segment`]s, render plain text, SRT,
//! WebVTT, or JSON. Timestamp formatting is the subtle part and is property
//! tested.

#![forbid(unsafe_code)]

use oas_core::{OasError, Result};
use oas_engine::Segment;
use serde::Serialize;

/// Supported export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Txt,
    Srt,
    Vtt,
    Json,
}

impl Format {
    /// Parse from a lowercase extension or name.
    pub fn from_str_opt(s: &str) -> Option<Format> {
        match s.trim().to_lowercase().as_str() {
            "txt" | "text" => Some(Format::Txt),
            "srt" => Some(Format::Srt),
            "vtt" | "webvtt" => Some(Format::Vtt),
            "json" => Some(Format::Json),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Txt => "txt",
            Format::Srt => "srt",
            Format::Vtt => "vtt",
            Format::Json => "json",
        }
    }
}

/// Render `segments` in `format`.
pub fn export(segments: &[Segment], format: Format) -> Result<String> {
    match format {
        Format::Txt => Ok(to_txt(segments)),
        Format::Srt => Ok(to_srt(segments)),
        Format::Vtt => Ok(to_vtt(segments)),
        Format::Json => to_json(segments),
    }
}

fn to_txt(segments: &[Segment]) -> String {
    let mut out = String::new();
    for s in segments {
        let line = s.text.trim();
        if !line.is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn to_srt(segments: &[Segment]) -> String {
    let mut out = String::new();
    for (i, s) in segments.iter().enumerate() {
        out.push_str(&format!("{}\n", i + 1));
        out.push_str(&format!(
            "{} --> {}\n",
            timestamp(s.start, ','),
            timestamp(s.end, ',')
        ));
        out.push_str(s.text.trim());
        out.push_str("\n\n");
    }
    out
}

fn to_vtt(segments: &[Segment]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for s in segments {
        out.push_str(&format!(
            "{} --> {}\n",
            timestamp(s.start, '.'),
            timestamp(s.end, '.')
        ));
        out.push_str(s.text.trim());
        out.push_str("\n\n");
    }
    out
}

#[derive(Serialize)]
struct JsonDoc<'a> {
    segments: &'a [Segment],
}

fn to_json(segments: &[Segment]) -> Result<String> {
    serde_json::to_string_pretty(&JsonDoc { segments }).map_err(|e| OasError::Internal {
        reason: format!("failed to serialise transcript: {e}"),
    })
}

/// Format seconds as `HH:MM:SS<sep>mmm` (SRT uses `,`, VTT uses `.`). Negative
/// times are clamped to zero.
fn timestamp(seconds: f64, millis_sep: char) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02}{millis_sep}{ms:03}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn seg(start: f64, end: f64, text: &str) -> Segment {
        Segment {
            start,
            end,
            text: text.into(),
            words: vec![],
        }
    }

    #[test]
    fn timestamp_formatting_is_exact() {
        assert_eq!(timestamp(0.0, ','), "00:00:00,000");
        assert_eq!(timestamp(1.5, ','), "00:00:01,500");
        assert_eq!(timestamp(61.25, '.'), "00:01:01.250");
        assert_eq!(timestamp(3661.0, ','), "01:01:01,000");
        assert_eq!(timestamp(-5.0, ','), "00:00:00,000");
    }

    #[test]
    fn srt_has_index_and_arrow() {
        let out = to_srt(&[seg(0.0, 1.0, "hi"), seg(1.0, 2.0, "there")]);
        assert!(out.contains("1\n00:00:00,000 --> 00:00:01,000\nhi"));
        assert!(out.contains("2\n00:00:01,000 --> 00:00:02,000\nthere"));
    }

    #[test]
    fn vtt_starts_with_header_and_dot_millis() {
        let out = to_vtt(&[seg(0.0, 1.25, "hi")]);
        assert!(out.starts_with("WEBVTT\n"));
        assert!(out.contains("00:00:00.000 --> 00:00:01.250"));
    }

    #[test]
    fn txt_skips_blank_lines() {
        let out = to_txt(&[seg(0.0, 1.0, "  hello  "), seg(1.0, 2.0, "   ")]);
        assert_eq!(out, "hello\n");
    }

    #[test]
    fn json_round_trips() {
        let segs = vec![seg(0.0, 1.0, "hi")];
        let json = export(&segs, Format::Json).unwrap();
        assert!(json.contains("\"segments\""));
        assert!(json.contains("\"text\": \"hi\""));
    }

    proptest! {
        // Timestamp is always HH:MM:SS<sep>mmm with the right field widths.
        #[test]
        fn timestamp_shape(secs in 0.0f64..360000.0) {
            let t = timestamp(secs, ',');
            prop_assert_eq!(t.len(), 12);
            let bytes = t.as_bytes();
            prop_assert_eq!(bytes[2], b':');
            prop_assert_eq!(bytes[5], b':');
            prop_assert_eq!(bytes[8], b',');
            // milliseconds are in range
            let ms: u64 = t[9..12].parse().unwrap();
            prop_assert!(ms < 1000);
        }

        // Monotonic: a later time never formats to a smaller string.
        #[test]
        fn timestamp_monotonic(a in 0.0f64..100000.0, d in 0.0f64..100000.0) {
            let ta = timestamp(a, ',');
            let tb = timestamp(a + d, ',');
            prop_assert!(tb >= ta);
        }
    }
}
