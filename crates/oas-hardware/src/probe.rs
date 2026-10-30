//! The [`SystemProbe`] trait abstracts all hardware access, and [`RealProbe`]
//! is the production implementation. Tests use fake probes instead (see
//! `profile.rs`), so none of the profile-building logic depends on real
//! hardware being present.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sysinfo::{Disks, System};

use crate::types::{
    CpuFeatures, CpuInfo, DiskInfo, GpuBackend, GpuInfo, MemInfo, OsInfo, PowerInfo, ThermalState,
    UsageSample,
};

/// Everything the profile builder and usage sampler need from the system.
/// Implemented for real by [`RealProbe`] and for tests by fakes.
pub trait SystemProbe {
    fn os_info(&self) -> OsInfo;
    fn cpu_info(&self) -> CpuInfo;
    fn mem_info(&self) -> MemInfo;
    fn gpus(&self) -> Vec<GpuInfo>;
    fn disk_info(&self, models_path: &Path) -> DiskInfo;
    fn power_info(&self) -> PowerInfo;
    fn thermal_state(&self) -> ThermalState;
    /// Take a live usage sample. Mutable because the underlying library must
    /// refresh its counters.
    fn sample_usage(&mut self, models_path: &Path) -> UsageSample;
}

/// Current Unix time in milliseconds (0 if the clock is before the epoch).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Production probe backed by `sysinfo`, `raw-cpuid`, and `wgpu`.
pub struct RealProbe {
    system: System,
}

impl RealProbe {
    /// Create a probe and prime the system counters.
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        RealProbe { system }
    }
}

impl Default for RealProbe {
    fn default() -> Self {
        Self::new()
    }
}

/// Detect CPU SIMD features for the current architecture. On x86 this reads
/// CPUID; on aarch64 NEON is always present.
fn detect_cpu_features() -> CpuFeatures {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        let cpuid = raw_cpuid::CpuId::new();
        let ef = cpuid.get_extended_feature_info();
        let avx2 = ef.as_ref().map(|f| f.has_avx2()).unwrap_or(false);
        let avx512 = ef.as_ref().map(|f| f.has_avx512f()).unwrap_or(false);
        CpuFeatures {
            avx2,
            avx512,
            neon: false,
            amx: false,
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        CpuFeatures {
            avx2: false,
            avx512: false,
            neon: true,
            // Apple AMX is undocumented; report false rather than guess.
            amx: false,
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
    {
        CpuFeatures::default()
    }
}

/// Map a PCI vendor id to a human name.
fn vendor_name(vendor_id: u32) -> String {
    match vendor_id {
        0x10de => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        0x106b => "Apple",
        0x13b5 => "ARM",
        0x5143 => "Qualcomm",
        _ => "Unknown",
    }
    .to_string()
}

fn map_backend(backend: wgpu::Backend) -> (GpuBackend, bool, Option<String>) {
    match backend {
        wgpu::Backend::Metal => (GpuBackend::Metal, true, None),
        wgpu::Backend::Vulkan => (GpuBackend::Vulkan, true, None),
        wgpu::Backend::Dx12 => (GpuBackend::Dx12, true, None),
        wgpu::Backend::Gl => (
            GpuBackend::Gl,
            false,
            Some("OpenGL is not supported for inference".to_string()),
        ),
        _ => (
            GpuBackend::Other,
            false,
            Some("unsupported graphics backend".to_string()),
        ),
    }
}

/// Enumerate graphics adapters via wgpu. Returns an empty list (not an error)
/// when no usable drivers are present.
fn enumerate_gpus() -> Vec<GpuInfo> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    instance
        .enumerate_adapters(wgpu::Backends::all())
        .into_iter()
        .filter_map(|adapter| {
            let info = adapter.get_info();
            // Skip software rasterisers (e.g. llvmpipe) — they are not real
            // accelerators and would mislead the fit engine.
            if info.device_type == wgpu::DeviceType::Cpu {
                return None;
            }
            let (backend, usable, reason) = map_backend(info.backend);
            let driver = if info.driver_info.is_empty() {
                None
            } else {
                Some(info.driver_info.clone())
            };
            Some(GpuInfo {
                vendor: vendor_name(info.vendor),
                name: info.name,
                backend,
                dedicated_vram_bytes: None,
                shared_bytes: None,
                driver,
                usable,
                not_usable_reason: reason,
            })
        })
        .collect()
}

impl SystemProbe for RealProbe {
    fn os_info(&self) -> OsInfo {
        OsInfo {
            name: System::name().unwrap_or_else(|| "Unknown".to_string()),
            version: System::os_version().unwrap_or_else(|| "Unknown".to_string()),
            arch: System::cpu_arch(),
        }
    }

    fn cpu_info(&self) -> CpuInfo {
        let cpus = self.system.cpus();
        let brand = cpus
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string());
        let base_mhz = cpus.first().map(|c| c.frequency() as u32).unwrap_or(0);
        CpuInfo {
            brand,
            physical_cores: self.system.physical_core_count().unwrap_or(cpus.len()) as u32,
            logical_cores: cpus.len() as u32,
            base_mhz,
            features: detect_cpu_features(),
        }
    }

    fn mem_info(&self) -> MemInfo {
        // Unified memory is the Apple Silicon case: aarch64 macOS.
        let is_unified = cfg!(all(target_os = "macos", target_arch = "aarch64"));
        MemInfo {
            total_bytes: self.system.total_memory(),
            available_bytes: self.system.available_memory(),
            is_unified,
        }
    }

    fn gpus(&self) -> Vec<GpuInfo> {
        enumerate_gpus()
    }

    fn disk_info(&self, models_path: &Path) -> DiskInfo {
        let disks = Disks::new_with_refreshed_list();
        // Pick the disk whose mount point is the longest prefix of the models
        // path — that is the volume the models live on.
        let best = disks
            .list()
            .iter()
            .filter(|d| models_path.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len());
        let free_bytes = best
            .or_else(|| disks.list().first())
            .map(|d| d.available_space())
            .unwrap_or(0);
        DiskInfo {
            free_bytes,
            is_ssd: None,
        }
    }

    fn power_info(&self) -> PowerInfo {
        // sysinfo does not expose battery; treat as desktop/plugged-in until a
        // platform-specific probe is added in a later pass.
        PowerInfo {
            on_battery: false,
            battery_pct: None,
            low_power_mode: false,
        }
    }

    fn thermal_state(&self) -> ThermalState {
        // Platform thermal APIs are added later; unknown is a valid answer.
        ThermalState::Unknown
    }

    fn sample_usage(&mut self, models_path: &Path) -> UsageSample {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();

        let cpu_per_core: Vec<f32> = self.system.cpus().iter().map(|c| c.cpu_usage()).collect();
        let cpu_total_pct = self.system.global_cpu_usage();

        let app_rss_bytes = self
            .system
            .process(sysinfo::get_current_pid().unwrap_or(sysinfo::Pid::from(0)))
            .map(|p| p.memory());

        let disks = Disks::new_with_refreshed_list();
        let disk_free_bytes = disks
            .list()
            .iter()
            .filter(|d| models_path.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .or_else(|| disks.list().first())
            .map(|d| d.available_space())
            .unwrap_or(0);

        UsageSample {
            cpu_total_pct,
            cpu_per_core,
            ram_used_bytes: self.system.used_memory(),
            ram_available_bytes: self.system.available_memory(),
            swap_used_bytes: self.system.used_swap(),
            gpu_util_pct: None,
            vram_used_bytes: None,
            app_rss_bytes,
            worker_rss_bytes: 0,
            disk_free_bytes,
            thermal: ThermalState::Unknown,
            battery_pct: None,
            captured_at_ms: now_ms(),
        }
    }
}
