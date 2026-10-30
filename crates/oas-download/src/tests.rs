use super::*;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// Start a tiny HTTP/1.1 server that serves `data` with Range support, on a
/// background thread. Returns the base URL. The thread lives until the process
/// exits (fine for short tests).
fn serve(data: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let data = data.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range_start: u64 = 0;
                let mut is_range = false;
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(rest) = lower.strip_prefix("range:") {
                        if let Some(bytes) = rest.trim().strip_prefix("bytes=") {
                            if let Some(start) = bytes.split('-').next() {
                                range_start = start.trim().parse().unwrap_or(0);
                                is_range = true;
                            }
                        }
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let start = (range_start as usize).min(data.len());
                let body = &data[start..];
                let header = if is_range {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        body.len(),
                        start,
                        data.len().saturating_sub(1),
                        data.len()
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        data.len()
                    )
                };
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.flush();
            });
        }
    });
    format!("http://127.0.0.1:{port}/model.bin")
}

fn spec(url: String, data: &[u8]) -> DownloadSpec {
    DownloadSpec {
        id: "whisper-tiny".into(),
        file_name: "model.bin".into(),
        url,
        mirrors: vec![],
        sha256: sha256_hex(data),
        expected_size: data.len() as u64,
    }
}

fn noop(_: Progress) {}

#[tokio::test]
async fn completes_and_verifies() {
    let data = vec![7u8; 100_000];
    let url = serve(data.clone());
    let dir = tempdir("ok");
    let out = download(
        &spec(url, &data),
        &dir,
        1 << 30,
        &DownloadConfig::default(),
        Arc::new(AtomicBool::new(false)),
        noop,
    )
    .await
    .expect("download");
    assert!(matches!(out, DownloadOutcome::Completed(_)));
    let installed = dir.join("whisper-tiny").join("model.bin");
    assert!(installed.exists());
    assert_eq!(std::fs::read(&installed).unwrap(), data);
    assert!(dir.join("whisper-tiny").join("manifest.json").exists());
    // tmp cleaned up.
    assert!(!dir.join(".tmp").join("whisper-tiny").exists());
}

#[tokio::test]
async fn resumes_from_a_partial_part() {
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    for k in [0usize, 17, 100_000, data.len() - 1] {
        let url = serve(data.clone());
        let dir = tempdir(&format!("resume{k}"));
        // Pre-seed a partial .part + sidecar as if interrupted at k bytes.
        let tmp = dir.join(".tmp").join("whisper-tiny");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("model.bin.part"), &data[..k]).unwrap();
        std::fs::write(
            tmp.join("model.bin.part.json"),
            format!("{{\"offset\":{k}}}"),
        )
        .unwrap();

        let out = download(
            &spec(url, &data),
            &dir,
            1 << 30,
            &DownloadConfig::default(),
            Arc::new(AtomicBool::new(false)),
            noop,
        )
        .await
        .unwrap_or_else(|e| panic!("resume k={k} failed: {e}"));
        assert!(matches!(out, DownloadOutcome::Completed(_)), "k={k}");
        let installed = dir.join("whisper-tiny").join("model.bin");
        assert_eq!(std::fs::read(&installed).unwrap(), data, "k={k}");
    }
}

#[tokio::test]
async fn hash_mismatch_is_reported_and_cleaned() {
    let data = vec![1u8; 50_000];
    let url = serve(data.clone());
    let dir = tempdir("badhash");
    let mut s = spec(url, &data);
    s.sha256 = "0".repeat(64); // wrong on purpose
    let err = download(
        &s,
        &dir,
        1 << 30,
        &DownloadConfig::default(),
        Arc::new(AtomicBool::new(false)),
        noop,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), "HASH_MISMATCH");
    assert!(!dir.join("whisper-tiny").join("model.bin").exists());
    assert!(!dir.join(".tmp").join("whisper-tiny").exists());
}

#[tokio::test]
async fn insufficient_disk_fails_before_download() {
    let data = vec![1u8; 100_000];
    let url = serve(data.clone());
    let dir = tempdir("disk");
    let err = download(
        &spec(url, &data),
        &dir,
        1000, // far less than needed
        &DownloadConfig::default(),
        Arc::new(AtomicBool::new(false)),
        noop,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), "INSUFFICIENT_DISK");
}

#[tokio::test]
async fn cancel_returns_stopped_without_installing() {
    let data = vec![3u8; 100_000];
    let url = serve(data.clone());
    let dir = tempdir("cancel");
    let out = download(
        &spec(url, &data),
        &dir,
        1 << 30,
        &DownloadConfig::default(),
        Arc::new(AtomicBool::new(true)), // cancelled up front
        noop,
    )
    .await
    .expect("cancel is not an error");
    assert!(matches!(out, DownloadOutcome::Stopped));
    assert!(!dir.join("whisper-tiny").join("model.bin").exists());
}

#[tokio::test]
async fn works_with_unicode_and_space_paths() {
    let data = vec![9u8; 80_000];
    let url = serve(data.clone());
    let dir = tempdir("space dir 练习");
    let out = download(
        &spec(url, &data),
        &dir,
        1 << 30,
        &DownloadConfig::default(),
        Arc::new(AtomicBool::new(false)),
        noop,
    )
    .await
    .expect("download in unicode path");
    assert!(matches!(out, DownloadOutcome::Completed(_)));
    assert!(dir.join("whisper-tiny").join("model.bin").exists());
}

/// A unique temp models directory for a test.
fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "oas-dl-{}-{}-{}",
        tag,
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
