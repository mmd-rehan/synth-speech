//! Resumable, verified model downloads.
//!
//! A download streams to `models/.tmp/<id>/<file>.part` (with a sidecar JSON
//! recording the byte offset so it survives a crash), hashes with SHA-256 as it
//! writes, and on success atomically renames the file into `models/<id>/` and
//! writes a `manifest.json`. A checksum mismatch deletes the file and returns
//! [`OasError::HashMismatch`].

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use oas_core::{OasError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What to download.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub id: String,
    pub file_name: String,
    pub url: String,
    pub mirrors: Vec<String>,
    pub sha256: String,
    pub expected_size: u64,
}

/// Tunables for retry and throttling.
#[derive(Debug, Clone, Copy)]
pub struct DownloadConfig {
    pub max_retries: u32,
    pub backoff_base_ms: u64,
    pub bandwidth_limit_bps: Option<u64>,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        DownloadConfig {
            max_retries: 4,
            backoff_base_ms: 400,
            bandwidth_limit_bps: None,
        }
    }
}

/// Progress update (throttled to at most ~4/sec by the engine).
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
    pub speed_bps: f64,
    pub eta_secs: Option<f64>,
}

/// Result of a download attempt.
#[derive(Debug, Clone)]
pub enum DownloadOutcome {
    /// Finished and verified; the model is installed.
    Completed(InstalledManifest),
    /// Stopped early because the cancel flag was set; the `.part` is kept so it
    /// can be resumed later.
    Stopped,
}

/// Written into `models/<id>/manifest.json` on a successful install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledManifest {
    pub id: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub installed_at_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// On-disk locations for a model id, rooted at the models directory.
pub struct Layout {
    pub final_dir: PathBuf,
    pub tmp_dir: PathBuf,
    pub part_file: PathBuf,
    pub sidecar: PathBuf,
    pub final_file: PathBuf,
    pub manifest: PathBuf,
}

impl Layout {
    pub fn new(models_dir: &Path, id: &str, file_name: &str) -> Self {
        let final_dir = models_dir.join(id);
        let tmp_dir = models_dir.join(".tmp").join(id);
        Layout {
            part_file: tmp_dir.join(format!("{file_name}.part")),
            sidecar: tmp_dir.join(format!("{file_name}.part.json")),
            final_file: final_dir.join(file_name),
            manifest: final_dir.join("manifest.json"),
            final_dir,
            tmp_dir,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Sidecar {
    offset: u64,
}

fn net_err(e: impl std::fmt::Display) -> OasError {
    OasError::Network {
        reason: e.to_string(),
    }
}

/// Download and install a model. Calls `on_progress` as bytes arrive. Setting
/// `cancel` stops the download (keeping the `.part` for resume).
pub async fn download(
    spec: &DownloadSpec,
    models_dir: &Path,
    free_bytes: u64,
    config: &DownloadConfig,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
) -> Result<DownloadOutcome> {
    // Disk pre-check: require size * 1.1 free.
    let needed = spec.expected_size + spec.expected_size / 10;
    if free_bytes < needed {
        return Err(OasError::InsufficientDisk {
            needed_bytes: needed,
            available_bytes: free_bytes,
        });
    }

    let layout = Layout::new(models_dir, &spec.id, &spec.file_name);
    tokio::fs::create_dir_all(&layout.tmp_dir).await?;

    let urls: Vec<&String> = std::iter::once(&spec.url)
        .chain(spec.mirrors.iter())
        .collect();
    let mut last_err: Option<OasError> = None;

    for url in urls {
        match attempt_url(spec, &layout, url, config, &cancel, &mut on_progress).await {
            Ok(Some(hex)) => return finalize(spec, &layout, hex).await,
            Ok(None) => return Ok(DownloadOutcome::Stopped),
            Err(e) => {
                tracing::warn!(url, error = %e, "download attempt failed; trying next source");
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or(OasError::Network {
        reason: "no download sources available".into(),
    }))
}

/// Try one URL with retries/backoff. Returns `Some(hex)` when the whole file is
/// downloaded, `None` if cancelled, or an error if all retries failed.
async fn attempt_url(
    spec: &DownloadSpec,
    layout: &Layout,
    url: &str,
    config: &DownloadConfig,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut impl FnMut(Progress),
) -> Result<Option<String>> {
    let client = reqwest::Client::builder().build().map_err(net_err)?;

    let mut attempt = 0u32;
    loop {
        match stream_once(spec, layout, url, &client, config, cancel, on_progress).await {
            Ok(res) => return Ok(res),
            Err(e) if e.is_retryable() && attempt < config.max_retries => {
                let delay = config.backoff_base_ms * 2u64.pow(attempt);
                tracing::warn!(url, attempt, delay_ms = delay, error = %e, "retrying");
                tokio::time::sleep(Duration::from_millis(delay)).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn stream_once(
    spec: &DownloadSpec,
    layout: &Layout,
    url: &str,
    client: &reqwest::Client,
    config: &DownloadConfig,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut impl FnMut(Progress),
) -> Result<Option<String>> {
    // Resume offset from the existing .part, if any.
    let mut offset = match tokio::fs::metadata(&layout.part_file).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };

    let mut request = client.get(url);
    if offset > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let resp = request.send().await.map_err(net_err)?;
    let status = resp.status();
    if !status.is_success() {
        return Err(OasError::Network {
            reason: format!("server returned HTTP {status}"),
        });
    }
    // If we asked for a range but the server sent the whole file (200), restart.
    let resuming = offset > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    if offset > 0 && !resuming {
        offset = 0;
    }

    // Seed the hasher with any bytes already on disk (for a true resume).
    let mut hasher = Sha256::new();
    if resuming {
        let mut existing = tokio::fs::File::open(&layout.part_file).await?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = existing.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resuming)
        .truncate(!resuming)
        .open(&layout.part_file)
        .await?;

    let mut downloaded = offset;
    let mut resp = resp;
    let start = Instant::now();
    let mut last_emit = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut ema_speed = 0.0f64;
    let mut last_bytes = downloaded;
    let mut last_time = Instant::now();

    while let Some(chunk) = resp.chunk().await.map_err(net_err)? {
        if cancel.load(Ordering::Relaxed) {
            file.flush().await?;
            persist_sidecar(layout, downloaded).await?;
            return Ok(None);
        }
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;

        // Optional bandwidth limit.
        if let Some(limit) = config.bandwidth_limit_bps {
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            let allowed = limit as f64 * elapsed;
            if (downloaded as f64) > allowed {
                let over = downloaded as f64 - allowed;
                let sleep = (over / limit as f64).min(0.5);
                tokio::time::sleep(Duration::from_secs_f64(sleep)).await;
            }
        }

        // Throttled progress, with an EMA-smoothed speed.
        if last_emit.elapsed() >= Duration::from_millis(250) {
            let dt = last_time.elapsed().as_secs_f64().max(0.001);
            let inst = (downloaded - last_bytes) as f64 / dt;
            ema_speed = if ema_speed == 0.0 {
                inst
            } else {
                0.6 * ema_speed + 0.4 * inst
            };
            let eta = if ema_speed > 1.0 {
                Some((spec.expected_size.saturating_sub(downloaded)) as f64 / ema_speed)
            } else {
                None
            };
            on_progress(Progress {
                downloaded,
                total: spec.expected_size,
                speed_bps: ema_speed,
                eta_secs: eta,
            });
            persist_sidecar(layout, downloaded).await?;
            last_emit = Instant::now();
            last_bytes = downloaded;
            last_time = Instant::now();
        }
    }

    file.flush().await?;
    persist_sidecar(layout, downloaded).await?;

    // A final progress tick at 100%.
    on_progress(Progress {
        downloaded,
        total: spec.expected_size,
        speed_bps: ema_speed,
        eta_secs: Some(0.0),
    });

    Ok(Some(format!("{:x}", hasher.finalize())))
}

async fn persist_sidecar(layout: &Layout, offset: u64) -> Result<()> {
    let json = serde_json::to_string(&Sidecar { offset }).map_err(|e| OasError::Io {
        reason: e.to_string(),
    })?;
    tokio::fs::write(&layout.sidecar, json).await?;
    Ok(())
}

async fn finalize(spec: &DownloadSpec, layout: &Layout, hex: String) -> Result<DownloadOutcome> {
    if !hex.eq_ignore_ascii_case(&spec.sha256) {
        // Corrupt: remove the whole tmp dir so a retry starts clean.
        let _ = tokio::fs::remove_dir_all(&layout.tmp_dir).await;
        return Err(OasError::HashMismatch {
            expected: spec.sha256.clone(),
            actual: hex,
        });
    }

    tokio::fs::create_dir_all(&layout.final_dir).await?;
    // Rename within the same volume (models/.tmp -> models/<id>) is atomic.
    tokio::fs::rename(&layout.part_file, &layout.final_file).await?;

    let manifest = InstalledManifest {
        id: spec.id.clone(),
        file_name: spec.file_name.clone(),
        size_bytes: spec.expected_size,
        sha256: spec.sha256.clone(),
        installed_at_ms: now_ms(),
    };
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| OasError::Io {
        reason: e.to_string(),
    })?;
    tokio::fs::write(&layout.manifest, json).await?;
    let _ = tokio::fs::remove_dir_all(&layout.tmp_dir).await;

    Ok(DownloadOutcome::Completed(manifest))
}

/// Hash a local file (for "add model from file" import). Returns lowercase hex.
pub async fn hash_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests;
