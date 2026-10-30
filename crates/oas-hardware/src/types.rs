//! Serializable data types describing the machine and its live usage. All of
//! these cross the IPC boundary, so they derive `specta::Type` for TypeScript
//! generation.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Operating system identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OsInfo {
    pub name: String,
    pub version: String,
    /// Architecture, e.g. `aarch64` or `x86_64`.
    pub arch: String,
}

/// CPU SIMD capabilities relevant to inference backends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CpuFeatures {
    pub avx2: bool,
    pub avx512: bool,
    pub neon: bool,
    pub amx: bool,
}

/// Processor details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CpuInfo {
    pub brand: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub base_mhz: u32,
    pub features: CpuFeatures,
}

/// System memory. `is_unified` is true on Apple Silicon, where CPU and GPU
/// share one pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MemInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub is_unified: bool,
}

/// The graphics backend a GPU is exposed through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GpuBackend {
    Metal,
    Vulkan,
    Dx12,
    Cuda,
    Gl,
    Other,
}

/// A graphics adapter that was found. `usable` records whether we can actually
/// run inference on it; if not, `not_usable_reason` explains why (shown in UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GpuInfo {
    pub vendor: String,
    pub name: String,
    pub backend: GpuBackend,
    pub dedicated_vram_bytes: Option<u64>,
    pub shared_bytes: Option<u64>,
    pub driver: Option<String>,
    pub usable: bool,
    pub not_usable_reason: Option<String>,
}

/// Free space on the volume holding the models directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DiskInfo {
    pub free_bytes: u64,
    /// Best effort; `None` when it cannot be determined.
    pub is_ssd: Option<bool>,
}

/// Power source and battery state. Everything is best effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PowerInfo {
    pub on_battery: bool,
    pub battery_pct: Option<u8>,
    pub low_power_mode: bool,
}

/// Thermal pressure. `Unknown` is a valid, non-fatal answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
    Unknown,
}

/// An accelerator the app can *actually* use for inference (not merely one that
/// exists). Derived from the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Accelerator {
    MetalGpu,
    CoreMl,
    Cuda { compute_cap: String },
    Vulkan,
    DirectMl,
    CpuAvx2,
    CpuAvx512,
    CpuNeon,
}

/// The full machine profile, captured at launch and refreshed on demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MachineProfile {
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub memory: MemInfo,
    pub gpus: Vec<GpuInfo>,
    pub disk: DiskInfo,
    pub power: PowerInfo,
    pub thermal: ThermalState,
    pub accel: Vec<Accelerator>,
    /// Unix epoch milliseconds when this profile was captured.
    pub captured_at_ms: u64,
}

/// A point-in-time sample of live resource usage for the charts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct UsageSample {
    pub cpu_total_pct: f32,
    pub cpu_per_core: Vec<f32>,
    pub ram_used_bytes: u64,
    pub ram_available_bytes: u64,
    pub swap_used_bytes: u64,
    pub gpu_util_pct: Option<f32>,
    pub vram_used_bytes: Option<u64>,
    pub app_rss_bytes: Option<u64>,
    pub worker_rss_bytes: u64,
    pub disk_free_bytes: u64,
    pub thermal: ThermalState,
    pub battery_pct: Option<u8>,
    pub captured_at_ms: u64,
}
