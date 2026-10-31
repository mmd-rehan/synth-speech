//! `oas-cli` — a thin developer front-end over the audio + engine + export
//! crates, for fast iteration and CI integration tests.
//!
//! Usage:
//!   oas-cli transcribe <audio> --model <ggml.bin> [--format txt|srt|vtt|json]
//!           [--out <file>] [--language <code>] [--translate] [--threads <n>]

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use std::cell::RefCell;
use std::rc::Rc;

use oas_core::Result;
use oas_engine::{Callbacks, CancelToken, Engine, RunOptions, Segment};
use oas_engine_whisper::WhisperEngine;
use oas_export::Format;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> std::result::Result<(), String> {
    if args.first().map(String::as_str) != Some("transcribe") {
        return Err(
            "usage: oas-cli transcribe <audio> --model <ggml.bin> [--format txt|srt|vtt|json] \
             [--out <file>] [--language <code>] [--translate] [--threads <n>]"
                .into(),
        );
    }

    let mut audio: Option<String> = None;
    let mut model: Option<String> = None;
    let mut format = Format::Txt;
    let mut out: Option<String> = None;
    let mut language: Option<String> = None;
    let mut translate = false;
    let mut threads: Option<u32> = None;

    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--model" => {
                model = Some(next(args, &mut i, "--model")?);
            }
            "--format" => {
                let f = next(args, &mut i, "--format")?;
                format = Format::from_str_opt(&f).ok_or_else(|| format!("unknown format: {f}"))?;
            }
            "--out" => out = Some(next(args, &mut i, "--out")?),
            "--language" => language = Some(next(args, &mut i, "--language")?),
            "--threads" => {
                threads = Some(
                    next(args, &mut i, "--threads")?
                        .parse()
                        .map_err(|_| "--threads must be a number".to_string())?,
                );
            }
            "--translate" => translate = true,
            other if other.starts_with("--") => {
                return Err(format!("unknown flag: {other}"));
            }
            _ => {
                if audio.is_none() {
                    audio = Some(a.clone());
                } else {
                    return Err(format!("unexpected argument: {a}"));
                }
            }
        }
        i += 1;
    }

    let audio = audio.ok_or("missing <audio> argument")?;
    let model = model.ok_or("missing --model")?;

    transcribe(
        &audio,
        &model,
        format,
        out.as_deref(),
        language,
        translate,
        threads,
    )
    .map_err(|e| format!("[{}] {}", e.code(), e.user_message()))
}

fn next(args: &[String], i: &mut usize, flag: &str) -> std::result::Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

#[allow(clippy::too_many_arguments)]
fn transcribe(
    audio: &str,
    model: &str,
    format: Format,
    out: Option<&str>,
    language: Option<String>,
    translate: bool,
    threads: Option<u32>,
) -> Result<()> {
    let pcm = oas_audio::decode_file(&PathBuf::from(audio))?;
    eprintln!(
        "decoded {:.1}s of audio at {} Hz",
        pcm.duration_sec(),
        pcm.sample_rate
    );

    let mut engine = WhisperEngine::load(&PathBuf::from(model))?;
    let opts = RunOptions {
        language,
        translate,
        threads,
        ..Default::default()
    };

    // Segments arrive through an owned callback while the engine runs.
    let collected = Rc::new(RefCell::new(Vec::<Segment>::new()));
    let sink = Rc::clone(&collected);
    let cancel = CancelToken::new();
    let callbacks = Callbacks {
        on_segment: Box::new(move |s: Segment| sink.borrow_mut().push(s)),
        on_progress: Box::new(|_p: f32| {}),
    };
    let summary = engine.transcribe(&pcm.samples, pcm.sample_rate, &opts, &cancel, callbacks)?;
    let segments = collected.borrow();
    eprintln!(
        "transcribed {} segments, rtf {:.3}",
        segments.len(),
        summary.rtf
    );

    let rendered = oas_export::export(&segments, format)?;
    match out {
        Some(path) => {
            std::fs::write(path, rendered)?;
            eprintln!("wrote {path}");
        }
        None => print!("{rendered}"),
    }
    Ok(())
}
