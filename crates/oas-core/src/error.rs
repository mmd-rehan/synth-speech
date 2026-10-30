//! The single error type for Open Audio Studio.
//!
//! Every variant carries a **stable machine code** (never change an existing
//! string — the UI and logs key off it), a human-readable message, and an
//! optional "what to do next" hint shown to the user.

use std::fmt;

/// Convenience alias used throughout the workspace.
pub type Result<T, E = OasError> = std::result::Result<T, E>;

/// All recoverable and reportable errors in the app.
///
/// Workers must never panic silently: panics are caught and converted into an
/// [`OasError`] before being reported over the worker protocol.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OasError {
    /// A model was requested that is not present on disk.
    #[error("The model \"{model_id}\" is not installed.")]
    ModelNotInstalled { model_id: String },

    /// The system (or a per-job budget) ran out of memory.
    #[error("Not enough memory to run this job.")]
    OutOfMemory,

    /// Audio could not be decoded to PCM.
    #[error("Could not read the audio: {reason}")]
    DecodeFailed { reason: String },

    /// A GPU backend failed to initialise; the caller may fall back to CPU.
    #[error("The GPU could not be initialised: {reason}")]
    GpuInitFailed { reason: String },

    /// A referenced file does not exist.
    #[error("File not found: {path}")]
    FileNotFound { path: String },

    /// Caller-supplied input was invalid.
    #[error("Invalid input: {reason}")]
    InvalidInput { reason: String },

    /// The worker <-> core JSONL protocol was violated.
    #[error("Worker protocol error: {reason}")]
    Protocol { reason: String },

    /// A network operation failed (downloads, update checks only).
    #[error("Network error: {reason}")]
    Network { reason: String },

    /// A downloaded file's SHA-256 did not match the catalog.
    #[error("The download was corrupted (checksum did not match).")]
    HashMismatch { expected: String, actual: String },

    /// Not enough free disk space for a download.
    #[error("Not enough disk space: need {needed_bytes} bytes, {available_bytes} free.")]
    InsufficientDisk {
        needed_bytes: u64,
        available_bytes: u64,
    },

    /// An action needing the network was attempted while offline mode is on.
    #[error("Offline mode is on, so this needs to be done with the network enabled.")]
    OfflineMode,

    /// An underlying I/O error.
    #[error("I/O error: {reason}")]
    Io { reason: String },

    /// A bug or unexpected condition. Should be rare and always logged.
    #[error("Internal error: {reason}")]
    Internal { reason: String },
}

impl OasError {
    /// Stable machine code for logs, metrics, and UI branching.
    ///
    /// These strings are part of the app's contract: do not rename existing
    /// ones. Add new variants instead.
    pub fn code(&self) -> &'static str {
        match self {
            OasError::ModelNotInstalled { .. } => "MODEL_NOT_INSTALLED",
            OasError::OutOfMemory => "OUT_OF_MEMORY",
            OasError::DecodeFailed { .. } => "DECODE_FAILED",
            OasError::GpuInitFailed { .. } => "GPU_INIT_FAILED",
            OasError::FileNotFound { .. } => "FILE_NOT_FOUND",
            OasError::InvalidInput { .. } => "INVALID_INPUT",
            OasError::Protocol { .. } => "PROTOCOL_ERROR",
            OasError::Network { .. } => "NETWORK_ERROR",
            OasError::HashMismatch { .. } => "HASH_MISMATCH",
            OasError::InsufficientDisk { .. } => "INSUFFICIENT_DISK",
            OasError::OfflineMode => "OFFLINE_MODE",
            OasError::Io { .. } => "IO_ERROR",
            OasError::Internal { .. } => "INTERNAL_ERROR",
        }
    }

    /// A short, user-facing message. Safe to show in the UI.
    pub fn user_message(&self) -> String {
        self.to_string()
    }

    /// A concrete suggestion for what the user can do, when one applies.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            OasError::ModelNotInstalled { .. } => {
                Some("Download the model from the Models screen, then try again.")
            }
            OasError::OutOfMemory => {
                Some("Close other apps, pick a smaller model, or run fewer jobs at once.")
            }
            OasError::GpuInitFailed { .. } => {
                Some("The job can still run on the CPU. Check your graphics drivers.")
            }
            OasError::DecodeFailed { .. } => {
                Some("Try a different file or convert it to WAV, MP3, or FLAC.")
            }
            OasError::Network { .. } => {
                Some("Check your internet connection, or enable offline mode to work without it.")
            }
            OasError::HashMismatch { .. } => {
                Some("The file was corrupted in transit. Delete it and download again.")
            }
            OasError::InsufficientDisk { .. } => {
                Some("Free up disk space or pick a smaller model.")
            }
            OasError::OfflineMode => {
                Some("Turn off offline mode in Settings to download or check for updates.")
            }
            _ => None,
        }
    }

    /// Whether retrying the same operation might succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            OasError::Network { .. } | OasError::Io { .. } | OasError::HashMismatch { .. }
        )
    }
}

/// Lets `?` convert `std::io::Error` into an `OasError` without `unwrap`.
impl From<std::io::Error> for OasError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            OasError::FileNotFound {
                path: e.to_string(),
            }
        } else {
            OasError::Io {
                reason: e.to_string(),
            }
        }
    }
}

/// A serialisable, UI-friendly view of an error: code + message + hint.
///
/// This is the shape crossed over IPC; the full `OasError` stays in Rust.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
    pub retryable: bool,
}

impl fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl From<&OasError> for ErrorInfo {
    fn from(e: &OasError) -> Self {
        ErrorInfo {
            code: e.code().to_string(),
            message: e.user_message(),
            hint: e.hint().map(str::to_string),
            retryable: e.is_retryable(),
        }
    }
}

impl From<OasError> for ErrorInfo {
    fn from(e: OasError) -> Self {
        ErrorInfo::from(&e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_exhaustive() {
        // If a variant is added, add it here too, so the codes stay a
        // deliberate, reviewed contract.
        let samples = [
            (
                OasError::ModelNotInstalled {
                    model_id: "x".into(),
                },
                "MODEL_NOT_INSTALLED",
            ),
            (OasError::OutOfMemory, "OUT_OF_MEMORY"),
            (
                OasError::DecodeFailed { reason: "x".into() },
                "DECODE_FAILED",
            ),
            (
                OasError::GpuInitFailed { reason: "x".into() },
                "GPU_INIT_FAILED",
            ),
            (
                OasError::FileNotFound { path: "x".into() },
                "FILE_NOT_FOUND",
            ),
            (
                OasError::InvalidInput { reason: "x".into() },
                "INVALID_INPUT",
            ),
            (OasError::Protocol { reason: "x".into() }, "PROTOCOL_ERROR"),
            (OasError::Network { reason: "x".into() }, "NETWORK_ERROR"),
            (
                OasError::HashMismatch {
                    expected: "a".into(),
                    actual: "b".into(),
                },
                "HASH_MISMATCH",
            ),
            (
                OasError::InsufficientDisk {
                    needed_bytes: 2,
                    available_bytes: 1,
                },
                "INSUFFICIENT_DISK",
            ),
            (OasError::OfflineMode, "OFFLINE_MODE"),
            (OasError::Io { reason: "x".into() }, "IO_ERROR"),
            (OasError::Internal { reason: "x".into() }, "INTERNAL_ERROR"),
        ];
        for (err, code) in samples {
            assert_eq!(err.code(), code);
        }
    }

    #[test]
    fn error_info_roundtrips_through_json() {
        let err = OasError::OutOfMemory;
        let info = ErrorInfo::from(&err);
        let json = serde_json::to_string(&info).expect("serialise");
        let back: ErrorInfo = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(info, back);
        assert_eq!(back.code, "OUT_OF_MEMORY");
        assert!(back.hint.is_some());
    }

    #[test]
    fn io_not_found_maps_to_file_not_found() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let err: OasError = io.into();
        assert_eq!(err.code(), "FILE_NOT_FOUND");
    }

    #[test]
    fn network_errors_are_retryable() {
        assert!(OasError::Network { reason: "x".into() }.is_retryable());
        assert!(!OasError::OutOfMemory.is_retryable());
    }
}
