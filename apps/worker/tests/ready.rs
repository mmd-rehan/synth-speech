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
