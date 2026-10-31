//! Integration test: the built worker binary emits exactly one valid `ready`
//! JSONL line and exits successfully. This exercises the real stdout path the
//! core relies on when spawning the sidecar.

use std::process::Command;

use oas_protocol::{Envelope, WorkerToCore, PROTOCOL_VERSION};

#[test]
fn worker_prints_a_single_valid_ready_line() {
    // Cargo sets this to the path of the compiled `oas-worker` binary.
    let exe = env!("CARGO_BIN_EXE_oas-worker");
    let output = Command::new(exe).output().expect("spawn oas-worker");

    assert!(
        output.status.success(),
        "worker exited with failure: {:?}",
        output.status
    );

    let stdout = String::from_utf8(output.stdout).expect("worker stdout is valid UTF-8");
    let mut lines = stdout.lines();
    let first = lines.next().expect("worker printed at least one line");
    assert!(
        lines.next().is_none(),
        "worker should print exactly one line in M00"
    );

    let env: Envelope<WorkerToCore> =
        Envelope::from_line(first).expect("ready line parses as a protocol message");
    assert_eq!(env.v, PROTOCOL_VERSION);
    match env.msg {
        WorkerToCore::Ready { worker_version } => {
            assert!(!worker_version.is_empty(), "worker reports its version");
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

/// Probing runs in the real worker: a real file gives its length, and a corrupt
/// one gives `None` rather than an error. Uses a path with a space and Unicode.
#[test]
fn worker_reads_durations_from_headers() {
    let dir = std::env::temp_dir().join(format!("oas probe é {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("clip one.wav");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../desktop/src-tauri/resources/speed-test-clip.wav"
        ),
        &good,
    )
    .unwrap();
    let bad = dir.join("not audio.wav");
    std::fs::write(&bad, b"this is not audio").unwrap();

    let line = Envelope::new(oas_protocol::CoreToWorker::Probe {
        paths: vec![
            good.to_string_lossy().into_owned(),
            bad.to_string_lossy().into_owned(),
        ],
    })
    .to_line()
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_oas-worker"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{line}").unwrap();
        // Dropping stdin here must not stop a probe in progress from answering
        // or leave the worker running.
    }
    let output = child.wait_with_output().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let durations = stdout
        .lines()
        .filter_map(|l| Envelope::<WorkerToCore>::from_line(l).ok())
        .find_map(|e| match e.msg {
            WorkerToCore::Durations { seconds } => Some(seconds),
            _ => None,
        })
        .expect("a durations reply");
    assert_eq!(durations.len(), 2);
    assert!((durations[0].expect("clip length") - 11.0).abs() < 0.1);
    assert_eq!(durations[1], None);
}
