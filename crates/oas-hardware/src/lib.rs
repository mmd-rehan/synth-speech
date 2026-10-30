//! Machine profiling: CPU, RAM, GPU, disk, OS, thermal, and live usage.
//!
//! Placeholder crate scaffolded in M00; the real implementation lands in M01.
//! It links against `oas-core` so the workspace layering is exercised from the start.

#![forbid(unsafe_code)]

/// Re-export of the shared result type so downstream crates import it from here.
pub use oas_core::Result;

#[cfg(test)]
mod tests {
    #[test]
    fn links_against_core() {
        // Proves the crate compiles and is wired to oas-core.
        let err = oas_core::OasError::OutOfMemory;
        assert_eq!(err.code(), "OUT_OF_MEMORY");
    }
}
