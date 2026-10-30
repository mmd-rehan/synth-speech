//! `oas-core` holds the domain vocabulary shared across Open Audio Studio:
//! stable error codes, identifier newtypes, path layout, and logging setup.
//!
//! This crate performs no network or heavy filesystem I/O and depends on no
//! other internal crate. Everything above it in the architecture may depend on
//! it; it depends on nothing of ours.

#![forbid(unsafe_code)]

pub mod error;
pub mod ids;
pub mod paths;
pub mod telemetry;

pub use error::{OasError, Result};
pub use ids::{JobId, ModelId};
pub use paths::Paths;
