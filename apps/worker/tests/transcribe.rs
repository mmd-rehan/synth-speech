//! Full worker protocol test: send `start`, expect `ready`, at least one
//! `segment`, and `done`. Gated on OAS_TEST_MODEL + OAS_TEST_AUDIO so CI does
//! not need a model.

use std::io::Write;
use std::process::{Command, Stdio};

use oas_protocol::{CoreToWorker, EngineKind, Envelope, JobOptions, Limits};

#[test]
fn start_produces_segments_and_done() {
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

    let start = Envelope::new(CoreToWorker::Start {
        job_id: "test-job".into(),
        audio_path: audio,
        model_path: model,
        engine: EngineKind::WhisperCpp,
        options: JobOptions::default(),
        limits: Limits::default(),
    })
    .to_line()
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_oas-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn worker");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{start}\n").as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("worker output");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("\"type\":\"ready\""), "no ready: {stdout}");
    assert!(
        stdout.contains("\"type\":\"segment\""),
        "no segment: {stdout}"
    );
    assert!(stdout.contains("\"type\":\"done\""), "no done: {stdout}");
}
