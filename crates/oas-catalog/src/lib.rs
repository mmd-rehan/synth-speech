//! The model catalog: strongly-typed entries plus a validating JSON loader.
//!
//! File sizes and SHA-256 hashes in the shipped catalog are real values taken
//! from the model host and checked by `scripts/verify-catalog`. The loader
//! rejects malformed catalogs with a clear [`OasError`].

#![forbid(unsafe_code)]

use oas_core::{OasError, Result};
use serde::{Deserialize, Serialize};
use specta::Type;

/// Schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// One reference measurement of a model's real-time factor on a named machine
/// profile. Used by the fit engine's speed estimate. These are provisional
/// estimates until calibrated against real runs (M03+).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RtfReference {
    /// A profile key such as `apple_m1_metal` or `ryzen_8c_avx2_cpu`.
    pub profile: String,
    /// Real-time factor: processing time / audio duration (lower is faster).
    pub rtf: f32,
}

/// A single downloadable model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ModelEntry {
    /// Stable catalog id, e.g. `whisper-large-v3-turbo-q5_0`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Model family, e.g. `whisper`.
    pub family: String,
    /// Variant within the family, e.g. `large-v3-turbo`.
    pub variant: String,
    /// Quantisation tag (e.g. `q5_0`), or `None` for full precision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quant: Option<String>,
    /// Whether this is an English-only model.
    #[serde(default)]
    pub english_only: bool,
    /// Download URL.
    pub url: String,
    /// Optional mirror URLs.
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// Real file size in bytes (verified).
    #[specta(type = f64)]
    pub weights_bytes: u64,
    /// Real lowercase hex SHA-256 of the file (verified).
    pub sha256: String,
    /// Non-weight working memory at runtime (activations, KV cache, etc.).
    #[specta(type = f64)]
    pub runtime_overhead_bytes: u64,
    /// Extra working set per minute of audio.
    #[specta(type = f64)]
    pub per_minute_audio_bytes: u64,
    /// Whether the model can run on a GPU accelerator.
    pub gpu_capable: bool,
    /// Minimum dedicated VRAM for a GPU run.
    #[specta(type = f64)]
    pub min_vram_bytes: u64,
    /// Quality tier, 1 (lowest) to 5 (highest).
    pub quality_tier: u8,
    /// License identifier (e.g. `MIT`).
    pub license: String,
    /// Reference RTF measurements per machine profile.
    #[serde(default)]
    pub rtf_reference: Vec<RtfReference>,
}

/// The full catalog document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Catalog {
    pub schema_version: u32,
    pub models: Vec<ModelEntry>,
}

impl Catalog {
    /// Parse and validate a catalog from JSON text.
    pub fn from_json(text: &str) -> Result<Self> {
        let catalog: Catalog = serde_json::from_str(text).map_err(|e| OasError::InvalidInput {
            reason: format!("catalog is not valid JSON: {e}"),
        })?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Look up a model by id.
    pub fn get(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(OasError::InvalidInput {
                reason: format!(
                    "catalog schema version {} is not supported (expected {})",
                    self.schema_version, SCHEMA_VERSION
                ),
            });
        }
        if self.models.is_empty() {
            return Err(OasError::InvalidInput {
                reason: "catalog contains no models".into(),
            });
        }
        let mut seen = std::collections::HashSet::new();
        for m in &self.models {
            let where_ = || format!("model \"{}\"", m.id);
            if m.id.trim().is_empty() {
                return Err(invalid("a model has an empty id"));
            }
            if !seen.insert(m.id.as_str()) {
                return Err(invalid(format!("duplicate model id \"{}\"", m.id)));
            }
            if m.url.trim().is_empty() {
                return Err(invalid(format!("{} has an empty url", where_())));
            }
            if !is_sha256(&m.sha256) {
                return Err(invalid(format!(
                    "{} has an invalid sha256 (must be 64 hex chars)",
                    where_()
                )));
            }
            if m.weights_bytes == 0 {
                return Err(invalid(format!("{} has zero weights_bytes", where_())));
            }
            if !(1..=5).contains(&m.quality_tier) {
                return Err(invalid(format!(
                    "{} has quality_tier {} (must be 1..=5)",
                    where_(),
                    m.quality_tier
                )));
            }
        }
        Ok(())
    }
}

fn invalid(reason: impl Into<String>) -> OasError {
    OasError::InvalidInput {
        reason: reason.into(),
    }
}

/// True for a 64-character lowercase/uppercase hex string.
fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
        "schema_version": 1,
        "models": [{
            "id": "whisper-tiny",
            "name": "Whisper Tiny",
            "family": "whisper",
            "variant": "tiny",
            "url": "https://example.com/ggml-tiny.bin",
            "weights_bytes": 77691713,
            "sha256": "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            "runtime_overhead_bytes": 250000000,
            "per_minute_audio_bytes": 2000000,
            "gpu_capable": true,
            "min_vram_bytes": 400000000,
            "quality_tier": 1,
            "license": "MIT"
        }]
    }"#;

    #[test]
    fn loads_a_valid_catalog() {
        let c = Catalog::from_json(GOOD).unwrap();
        assert_eq!(c.models.len(), 1);
        assert_eq!(c.get("whisper-tiny").unwrap().quality_tier, 1);
        assert!(c.get("missing").is_none());
    }

    #[test]
    fn rejects_bad_json() {
        assert_eq!(
            Catalog::from_json("{not json").unwrap_err().code(),
            "INVALID_INPUT"
        );
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let bad = GOOD.replace("\"schema_version\": 1", "\"schema_version\": 99");
        let err = Catalog::from_json(&bad).unwrap_err();
        assert!(err.user_message().contains("schema version"));
    }

    #[test]
    fn rejects_empty_models() {
        let bad = r#"{"schema_version":1,"models":[]}"#;
        assert!(Catalog::from_json(bad)
            .unwrap_err()
            .user_message()
            .contains("no models"));
    }

    #[test]
    fn rejects_bad_sha256() {
        let bad = GOOD.replace(
            "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            "xyz",
        );
        assert!(Catalog::from_json(&bad)
            .unwrap_err()
            .user_message()
            .contains("sha256"));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let dup = r#"{"schema_version":1,"models":[
            {"id":"a","name":"A","family":"whisper","variant":"tiny","url":"u",
             "weights_bytes":1,"sha256":"be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
             "runtime_overhead_bytes":1,"per_minute_audio_bytes":1,"gpu_capable":true,
             "min_vram_bytes":1,"quality_tier":1,"license":"MIT"},
            {"id":"a","name":"A","family":"whisper","variant":"tiny","url":"u",
             "weights_bytes":1,"sha256":"be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
             "runtime_overhead_bytes":1,"per_minute_audio_bytes":1,"gpu_capable":true,
             "min_vram_bytes":1,"quality_tier":1,"license":"MIT"}
        ]}"#;
        assert!(Catalog::from_json(dup)
            .unwrap_err()
            .user_message()
            .contains("duplicate"));
    }

    #[test]
    fn rejects_bad_quality_tier() {
        let bad = GOOD.replace("\"quality_tier\": 1", "\"quality_tier\": 9");
        assert!(Catalog::from_json(&bad)
            .unwrap_err()
            .user_message()
            .contains("quality_tier"));
    }

    #[test]
    fn shipped_catalog_is_valid() {
        // The real catalog file must always parse and validate.
        let text = include_str!("../../../catalog/models.v1.json");
        let catalog = Catalog::from_json(text).expect("shipped catalog is valid");
        assert_eq!(catalog.schema_version, SCHEMA_VERSION);
        assert!(catalog.models.len() >= 12);
        // The recommended default must exist.
        assert!(catalog.get("whisper-large-v3-turbo").is_some());
    }
}
