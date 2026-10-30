//! Identifier newtypes. Using distinct types (not bare strings) stops us from
//! ever passing a model id where a job id is expected.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique id for a transcription job. Generated when a job is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(Uuid);

impl JobId {
    /// Create a fresh random id.
    pub fn new() -> Self {
        JobId(Uuid::new_v4())
    }

    /// The underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for JobId {
    type Err = crate::OasError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s)
            .map(JobId)
            .map_err(|e| crate::OasError::InvalidInput {
                reason: format!("not a valid job id: {e}"),
            })
    }
}

/// Stable id for a catalog model, e.g. `whisper-base-q5_0`. Chosen by the
/// catalog, not random, so it can be referenced in links and settings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    /// Wrap a non-empty id string.
    pub fn new(id: impl Into<String>) -> crate::Result<Self> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(crate::OasError::InvalidInput {
                reason: "model id must not be empty".into(),
            });
        }
        Ok(ModelId(id))
    }

    /// Borrow the id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_id_roundtrips_through_string() {
        let id = JobId::new();
        let parsed: JobId = id.to_string().parse().expect("parse back");
        assert_eq!(id, parsed);
    }

    #[test]
    fn job_ids_are_unique() {
        assert_ne!(JobId::new(), JobId::new());
    }

    #[test]
    fn bad_job_id_is_invalid_input() {
        let err = "not-a-uuid".parse::<JobId>().unwrap_err();
        assert_eq!(err.code(), "INVALID_INPUT");
    }

    #[test]
    fn empty_model_id_is_rejected() {
        assert!(ModelId::new("  ").is_err());
        assert_eq!(
            ModelId::new("whisper-base").unwrap().as_str(),
            "whisper-base"
        );
    }
}
