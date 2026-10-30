//! CLI integration tests.
//!
//! The argument/error paths run everywhere. A real transcription is exercised
//! only when `OAS_TEST_MODEL` (a ggml model path) and `OAS_TEST_AUDIO` (an audio
//! file) are set, so CI does not need to ship a model.

use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_oas-cli"))
}

#[test]
fn missing_model_flag_errors() {
    let out = cli().args(["transcribe", "some.wav"]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("missing --model"), "stderr: {err}");
}

#[test]
fn no_subcommand_prints_usage() {
    let out = cli().output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("usage:"), "stderr: {err}");
}

#[test]
fn missing_audio_file_is_a_clean_error() {
    let out = cli()
        .args([
            "transcribe",
            "/no/such/audio.wav",
            "--model",
            "/no/such/model.bin",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    // Decoding is attempted first; the missing file surfaces as FILE_NOT_FOUND.
    assert!(err.contains("FILE_NOT_FOUND"), "stderr: {err}");
}

#[test]
fn real_transcription_when_model_provided() {
    let (model, audio) = match (
        std::env::var("OAS_TEST_MODEL"),
        std::env::var("OAS_TEST_AUDIO"),
    ) {
        (Ok(m), Ok(a)) => (m, a),
        _ => {
            eprintln!("skipping: set OAS_TEST_MODEL and OAS_TEST_AUDIO to run");
            return;
        }
    };
    let out = cli()
        .args(["transcribe", &audio, "--model", &model])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.trim().is_empty(), "expected some transcript text");
}
