//! Running worker processes. The manager only sees the [`WorkerSpawner`] and
//! [`WorkerHandle`] traits, so tests can swap in fake workers.
//!
//! Workers are spawned directly rather than through a shell. The core holds the
//! write end of each worker's stdin for the worker's whole life, and a worker
//! exits when that closes, so a core that dies for any reason takes its workers
//! with it on every platform.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oas_core::{OasError, Result};
use oas_protocol::{CoreToWorker, Envelope, WorkerToCore};

/// How a worker process ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerExit {
    /// Ended with a success status.
    pub success: bool,
    /// The way it ended looks like the OS killed it for using too much memory.
    pub oom_suspected: bool,
    /// A short description for logs and error messages.
    pub detail: String,
}

/// Something that happened to a worker.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkerEventKind {
    Message(WorkerToCore),
    Exited(WorkerExit),
}

/// An event from one worker. `attempt` identifies the spawn, so a late event
/// from an old worker can never be confused with the job's current one.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerEvent {
    pub job_id: String,
    pub attempt: u64,
    pub kind: WorkerEventKind,
}

/// Where a worker's events are delivered. Called from the worker's reader
/// thread, and must not be called while spawning.
pub type WorkerSink = Arc<dyn Fn(WorkerEvent) + Send + Sync>;

/// Control of one running worker.
pub trait WorkerHandle: Send {
    /// Send a message to the worker.
    fn send(&mut self, msg: &CoreToWorker) -> Result<()>;
    /// Kill the worker. Its exit still arrives as an `Exited` event.
    fn kill(&mut self);
}

/// Starts workers.
pub trait WorkerSpawner: Send {
    fn spawn(&self, job_id: &str, attempt: u64, sink: WorkerSink) -> Result<Box<dyn WorkerHandle>>;
}

/// The live worker processes, for leak checks and diagnostics.
#[derive(Debug, Clone, Default)]
pub struct WorkerRegistry {
    live: Arc<Mutex<HashMap<u64, Arc<Mutex<Child>>>>>,
    next: Arc<AtomicU64>,
}

impl WorkerRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<Mutex<Child>>>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// How many worker processes have not yet been reaped.
    pub fn count(&self) -> usize {
        self.lock().len()
    }

    /// Kill one live worker, as the OS would. Returns whether one was found.
    pub fn kill_any(&self) -> bool {
        let child = self.lock().values().next().cloned();
        match child {
            Some(child) => {
                let _ = lock_child(&child).kill();
                true
            }
            None => false,
        }
    }
}

fn lock_child(child: &Mutex<Child>) -> std::sync::MutexGuard<'_, Child> {
    child.lock().unwrap_or_else(|e| e.into_inner())
}

/// Spawns the real worker binary.
pub struct ProcessSpawner {
    exe: PathBuf,
    registry: WorkerRegistry,
}

impl ProcessSpawner {
    pub fn new(exe: PathBuf) -> Self {
        ProcessSpawner {
            exe,
            registry: WorkerRegistry::default(),
        }
    }

    /// A handle to the live worker set.
    pub fn registry(&self) -> WorkerRegistry {
        self.registry.clone()
    }
}

/// A command for the worker with its stdio piped. stderr is discarded, since
/// nothing would drain it and native libraries can be chatty.
fn worker_command(exe: &PathBuf) -> Command {
    let mut command = Command::new(exe);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

impl WorkerSpawner for ProcessSpawner {
    fn spawn(&self, job_id: &str, attempt: u64, sink: WorkerSink) -> Result<Box<dyn WorkerHandle>> {
        let mut child = worker_command(&self.exe)
            .spawn()
            .map_err(|e| OasError::WorkerCrashed {
                reason: format!("could not start the worker: {e}"),
            })?;
        let missing = || OasError::Internal {
            reason: "worker stdio was not piped".into(),
        };
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let stdout = child.stdout.take().ok_or_else(missing)?;

        let child = Arc::new(Mutex::new(child));
        let key = self.registry.next.fetch_add(1, Ordering::Relaxed);
        self.registry.lock().insert(key, Arc::clone(&child));

        let reader_child = Arc::clone(&child);
        let registry = self.registry.clone();
        let job_id = job_id.to_string();
        std::thread::spawn(move || {
            let event = |kind| WorkerEvent {
                job_id: job_id.clone(),
                attempt,
                kind,
            };
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match Envelope::<WorkerToCore>::from_line(&line) {
                    Ok(env) => sink(event(WorkerEventKind::Message(env.msg))),
                    Err(e) => tracing::warn!(error = %e, "ignoring a bad line from a worker"),
                }
            }
            let exit = reap(&reader_child);
            registry.lock().remove(&key);
            sink(event(WorkerEventKind::Exited(exit)));
        });

        Ok(Box::new(ProcessHandle {
            child,
            stdin: Some(stdin),
        }))
    }
}

/// Wait for a worker whose stdout has closed, without holding the lock across a
/// blocking wait (kill needs it). A worker that lingers is killed.
fn reap(child: &Mutex<Child>) -> WorkerExit {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        {
            let mut guard = lock_child(child);
            match guard.try_wait() {
                Ok(Some(status)) => return classify(status),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = guard.kill();
                }
                Ok(None) => {}
                Err(e) => {
                    return WorkerExit {
                        success: false,
                        oom_suspected: false,
                        detail: format!("could not read the exit status: {e}"),
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn classify(status: ExitStatus) -> WorkerExit {
    WorkerExit {
        success: status.success(),
        oom_suspected: looks_like_oom(status),
        detail: status.to_string(),
    }
}

/// macOS and Linux kill memory hogs with SIGKILL.
#[cfg(unix)]
fn looks_like_oom(status: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    status.signal() == Some(9)
}

/// Windows reports STATUS_NO_MEMORY as the exit code.
#[cfg(windows)]
fn looks_like_oom(status: ExitStatus) -> bool {
    const STATUS_NO_MEMORY: i32 = 0xC000_0017_u32 as i32;
    status.code() == Some(STATUS_NO_MEMORY)
}

#[cfg(not(any(unix, windows)))]
fn looks_like_oom(_status: ExitStatus) -> bool {
    false
}

struct ProcessHandle {
    child: Arc<Mutex<Child>>,
    /// Held open on purpose: closing it tells the worker the core is gone.
    stdin: Option<ChildStdin>,
}

impl WorkerHandle for ProcessHandle {
    fn send(&mut self, msg: &CoreToWorker) -> Result<()> {
        let line = Envelope::new(msg.clone()).to_line()?;
        let stdin = self.stdin.as_mut().ok_or_else(|| OasError::Internal {
            reason: "worker stdin is closed".into(),
        })?;
        writeln!(stdin, "{line}")?;
        stdin.flush()?;
        Ok(())
    }

    fn kill(&mut self) {
        let _ = lock_child(&self.child).kill();
    }
}

/// Longest time a batch of files may take to probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Read audio lengths in a worker, so media is never parsed in the core
/// process. A file that cannot be read gives `None`, and so does every file if
/// the worker crashes or stalls: the lengths only refine estimates.
pub fn probe_durations(exe: &PathBuf, paths: &[String]) -> Vec<Option<f64>> {
    probe_durations_within(exe, paths, PROBE_TIMEOUT)
}

/// [`probe_durations`] with a chosen time limit.
pub fn probe_durations_within(
    exe: &PathBuf,
    paths: &[String],
    limit: Duration,
) -> Vec<Option<f64>> {
    let unknown = vec![None; paths.len()];
    let Ok(mut child) = worker_command(exe).spawn() else {
        return unknown;
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return unknown;
    };

    let request = CoreToWorker::Probe {
        paths: paths.to_vec(),
    };
    let sent = Envelope::new(request)
        .to_line()
        .is_ok_and(|line| writeln!(stdin, "{line}").is_ok() && stdin.flush().is_ok());

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            if let Ok(env) = Envelope::<WorkerToCore>::from_line(&line) {
                if let WorkerToCore::Durations { seconds } = env.msg {
                    let _ = tx.send(seconds);
                    return;
                }
            }
        }
    });

    let answer = if sent {
        rx.recv_timeout(limit).ok()
    } else {
        None
    };
    let _ = child.kill();
    let _ = child.wait();
    answer
        .filter(|seconds| seconds.len() == paths.len())
        .unwrap_or(unknown)
}

/// Start the worker, read its `ready` line, and return its version. Proves the
/// bundled worker can run.
pub fn probe_worker(exe: PathBuf) -> Result<String> {
    let mut child = worker_command(&exe)
        .spawn()
        .map_err(|e| OasError::WorkerCrashed {
            reason: format!("could not start the worker: {e}"),
        })?;
    let stdout = child.stdout.take().ok_or_else(|| OasError::Internal {
        reason: "worker stdout was not piped".into(),
    })?;
    let mut first = String::new();
    let read = BufReader::new(stdout).read_line(&mut first);
    // Closing stdin ends an idle worker.
    drop(child.stdin.take());
    let _ = child.wait();
    read?;
    match Envelope::<WorkerToCore>::from_line(first.trim())?.msg {
        WorkerToCore::Ready { worker_version } => Ok(worker_version),
        other => Err(OasError::Protocol {
            reason: format!("expected a ready message, got {other:?}"),
        }),
    }
}
