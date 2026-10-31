//! Job model + state machine, the resource governor, the runtime watchdog, the
//! scheduler's pick-next rule, and queue ETA. The governor and watchdog are
//! pure functions over live data so they can be exhaustively tested.

#![forbid(unsafe_code)]

pub mod governor;
pub mod manager;
pub mod schedule;
pub mod watchdog;
pub mod worker;

pub use governor::{
    gauge, hard_cap, safety_reserve, AutoConcurrency, Budgets, DenyReason, Gauge, GaugeLevel,
    Governor, JobCost, Reservation,
};
pub use manager::{
    Clock, GovernorStatus, GovernorWarning, JobEvents, JobManager, JobProgress, JobRequest,
    JobSegment, Machine, ManagerDeps, ParallelMode, Signals, SystemClock,
};
pub use schedule::{pick_next, queue_eta_secs};
pub use watchdog::{WarningReason, WatchdogAction, WatchdogConfig};
pub use worker::{
    probe_durations, probe_durations_within, probe_worker, ProcessSpawner, WorkerEvent,
    WorkerEventKind, WorkerExit, WorkerHandle, WorkerRegistry, WorkerSink, WorkerSpawner,
};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Scheduling priority. Higher runs first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
        }
    }

    /// Parse a stored value; anything unknown is treated as normal.
    pub fn parse(s: &str) -> Self {
        match s {
            "low" => Priority::Low,
            "high" => Priority::High,
            _ => Priority::Normal,
        }
    }
}

/// Who paused a job. Only the governor's own pauses resume by themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PauseOrigin {
    User,
    Governor,
}

impl PauseOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            PauseOrigin::User => "user",
            PauseOrigin::Governor => "governor",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(PauseOrigin::User),
            "governor" => Some(PauseOrigin::Governor),
            _ => None,
        }
    }
}

/// Lifecycle state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Starting,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Starting => "starting",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }

    /// Parse a stored value; `None` if it is unknown.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => JobState::Queued,
            "starting" => JobState::Starting,
            "running" => JobState::Running,
            "paused" => JobState::Paused,
            "completed" => JobState::Completed,
            "failed" => JobState::Failed,
            "cancelled" => JobState::Cancelled,
            _ => return None,
        })
    }

    /// Whether this state holds a governor reservation (occupies resources).
    pub fn is_active(self) -> bool {
        matches!(self, JobState::Starting | JobState::Running)
    }

    /// Whether the job is finished for good (barring a retry).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        )
    }

    /// Whether a transition `self -> to` is legal.
    pub fn can_transition(self, to: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, to),
            (Queued, Starting)
                | (Queued, Paused)
                | (Queued, Cancelled)
                | (Queued, Failed)
                | (Starting, Running)
                | (Starting, Failed)
                | (Starting, Cancelled)
                | (Starting, Paused)
                | (Running, Completed)
                | (Running, Failed)
                | (Running, Cancelled)
                | (Running, Paused)
                | (Paused, Queued)
                | (Paused, Cancelled)
                | (Paused, Failed)
                // Retry from a finished-but-not-completed state.
                | (Failed, Queued)
                | (Cancelled, Queued)
        )
    }
}

/// A transcription job.
///
/// Pausing stops the worker: that is the only way to give the memory back, and
/// no OS-independent way exists to suspend native inference. Stored segments
/// are kept, and resuming continues from the end of the last one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Job {
    pub id: String,
    pub model_id: String,
    pub input_path: String,
    /// Audio length once known.
    #[specta(type = Option<f64>)]
    pub duration_ms: Option<u64>,
    pub priority: Priority,
    /// Position within a priority; lower runs first. Drag to reorder sets this.
    #[specta(type = f64)]
    pub rank: i64,
    pub state: JobState,
    /// Set while `Paused`.
    pub pause_origin: Option<PauseOrigin>,
    /// Why a queued job is not running yet (not persisted).
    pub wait_reason: Option<String>,
    /// Whole-job progress in \[0, 1].
    pub progress: f32,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub translate: bool,
    #[serde(default)]
    pub word_timestamps: bool,
    #[specta(type = f64)]
    pub created_at_ms: u64,
    #[specta(type = Option<f64>)]
    pub started_at_ms: Option<u64>,
    #[specta(type = Option<f64>)]
    pub finished_at_ms: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub retries: u32,
    /// Measured real-time factor, once finished.
    pub rtf: Option<f32>,
}

impl Job {
    /// Apply a state transition if legal; returns false (and does nothing) if
    /// the transition is illegal.
    pub fn try_transition(&mut self, to: JobState) -> bool {
        if self.state.can_transition(to) {
            self.state = to;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_orders_high_first() {
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
        let mut v = [Priority::Low, Priority::High, Priority::Normal];
        v.sort();
        assert_eq!(v, [Priority::Low, Priority::Normal, Priority::High]);
    }

    #[test]
    fn legal_transitions_table() {
        use JobState::*;
        let legal = [
            (Queued, Starting),
            (Starting, Running),
            (Running, Completed),
            (Running, Paused),
            (Starting, Paused),
            (Paused, Queued),
            (Running, Cancelled),
            (Failed, Queued),
            (Cancelled, Queued),
        ];
        for (a, b) in legal {
            assert!(a.can_transition(b), "{a:?} -> {b:?} should be legal");
        }
    }

    #[test]
    fn illegal_transitions_rejected() {
        use JobState::*;
        let illegal = [
            (Completed, Running),
            (Completed, Queued),
            (Queued, Running), // must pass through Starting
            (Paused, Running), // a resumed job gets a new worker via Queued
            (Queued, Completed),
            (Cancelled, Running),
            (Running, Starting),
        ];
        for (a, b) in illegal {
            assert!(!a.can_transition(b), "{a:?} -> {b:?} should be illegal");
        }
    }

    #[test]
    fn try_transition_mutates_only_when_legal() {
        let mut job = Job {
            id: "j".into(),
            model_id: "whisper-tiny".into(),
            input_path: "/a.wav".into(),
            duration_ms: None,
            priority: Priority::Normal,
            rank: 1,
            state: JobState::Queued,
            pause_origin: None,
            wait_reason: None,
            progress: 0.0,
            language: None,
            translate: false,
            word_timestamps: false,
            created_at_ms: 0,
            started_at_ms: None,
            finished_at_ms: None,
            error_code: None,
            error_message: None,
            retries: 0,
            rtf: None,
        };
        assert!(job.try_transition(JobState::Starting));
        assert_eq!(job.state, JobState::Starting);
        assert!(!job.try_transition(JobState::Completed)); // Starting -> Completed illegal
        assert_eq!(job.state, JobState::Starting);
    }

    #[test]
    fn stored_values_roundtrip() {
        use JobState::*;
        for s in [
            Queued, Starting, Running, Paused, Completed, Failed, Cancelled,
        ] {
            assert_eq!(JobState::parse(s.as_str()), Some(s));
        }
        assert_eq!(JobState::parse("nonsense"), None);
        for p in [Priority::Low, Priority::Normal, Priority::High] {
            assert_eq!(Priority::parse(p.as_str()), p);
        }
        for o in [PauseOrigin::User, PauseOrigin::Governor] {
            assert_eq!(PauseOrigin::parse(o.as_str()), Some(o));
        }
    }

    #[test]
    fn active_and_terminal_classification() {
        assert!(JobState::Running.is_active());
        assert!(JobState::Starting.is_active());
        assert!(!JobState::Queued.is_active());
        assert!(JobState::Completed.is_terminal());
        assert!(JobState::Failed.is_terminal());
        assert!(!JobState::Running.is_terminal());
    }
}
