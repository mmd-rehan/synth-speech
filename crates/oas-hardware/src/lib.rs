//! `oas-hardware` builds an honest picture of the user's machine: a static
//! [`MachineProfile`] captured at launch, and a stream of [`UsageSample`]s for
//! the live resource view.
//!
//! All collection goes through the [`SystemProbe`] trait so the profile builder
//! and usage sampler can be tested with synthetic machines (an 8 GB laptop, an
//! M1, an RTX box, a GPU-less PC) without touching real hardware.

#![forbid(unsafe_code)]

pub mod probe;
pub mod profile;
pub mod types;
pub mod usage;

pub use probe::{RealProbe, SystemProbe};
pub use profile::build_profile;
pub use types::{
    Accelerator, CpuFeatures, CpuInfo, DiskInfo, GpuBackend, GpuInfo, MachineProfile, MemInfo,
    OsInfo, PowerInfo, ThermalState, UsageSample,
};
pub use usage::UsageHistory;
