//! Job model + state machine, the resource governor, the runtime watchdog, the
//! scheduler's pick-next rule, and queue ETA. The governor and watchdog are
//! pure functions over live data so they can be exhaustively tested.

#![forbid(unsafe_code)]

pub mod governor;
pub mod schedule;
pub mod watchdog;

pub use governor::{Budgets, DenyReason, Gauge, GaugeLevel, Governor, JobCost, Reservation};
pub use schedule::{pick_next, queue_eta_secs};
pub use watchdog::{WatchdogAction, WatchdogConfig};

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
                | (Running, Completed)
                | (Running, Failed)
                | (Running, Cancelled)
                | (Running, Paused)
                | (Paused, Queued)
                | (Paused, Running)
                | (Paused, Cancelled)
                | (Paused, Failed)
                // Retry from a finished-but-not-completed state.
                | (Failed, Queued)
                | (Cancelled, Queued)
        )
    }
}

/// A transcription job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Job {
    pub id: String,
    pub model_id: String,
    pub input_path: String,
    pub priority: Priority,
    pub state: JobState,
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
    pub retries: u8,
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
            (Paused, Running),
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
            priority: Priority::Normal,
            state: JobState::Queued,
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
        };
        assert!(job.try_transition(JobState::Starting));
        assert_eq!(job.state, JobState::Starting);
        assert!(!job.try_transition(JobState::Completed)); // Starting -> Completed illegal
        assert_eq!(job.state, JobState::Starting);
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
