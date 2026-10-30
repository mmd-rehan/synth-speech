//! Assembling a [`MachineProfile`] from a [`SystemProbe`], including deriving
//! the list of accelerators the app can actually use.

use std::path::Path;

use crate::probe::{now_ms, SystemProbe};
use crate::types::{Accelerator, GpuBackend, GpuInfo, MachineProfile};

/// Build the machine profile. `models_path` selects the disk volume to report
/// free space for.
pub fn build_profile<P: SystemProbe>(probe: &P, models_path: &Path) -> MachineProfile {
    let os = probe.os_info();
    let cpu = probe.cpu_info();
    let memory = probe.mem_info();
    let mut gpus = probe.gpus();
    let disk = probe.disk_info(models_path);
    let power = probe.power_info();
    let thermal = probe.thermal_state();

    // On unified-memory machines (Apple Silicon) the GPU shares the system RAM
    // pool, so record that as the GPU's available memory.
    if memory.is_unified {
        for gpu in gpus.iter_mut() {
            if gpu.backend == GpuBackend::Metal && gpu.shared_bytes.is_none() {
                gpu.shared_bytes = Some(memory.total_bytes);
            }
        }
    }

    let accel = derive_accelerators(&os.name, &gpus, &cpu.features);

    MachineProfile {
        os,
        cpu,
        memory,
        gpus,
        disk,
        power,
        thermal,
        accel,
        captured_at_ms: now_ms(),
    }
}

/// Work out which accelerators are usable, most capable first. GPUs come before
/// CPU SIMD; the strongest SIMD level is listed (not every level).
fn derive_accelerators(
    os_name: &str,
    gpus: &[GpuInfo],
    features: &crate::types::CpuFeatures,
) -> Vec<Accelerator> {
    let mut accel = Vec::new();
    let is_macos = os_name.to_lowercase().contains("mac") || os_name.eq_ignore_ascii_case("darwin");

    for gpu in gpus.iter().filter(|g| g.usable) {
        match gpu.backend {
            GpuBackend::Metal => {
                push_unique(&mut accel, Accelerator::MetalGpu);
                // CoreML (Apple Neural Engine / GPU) is available on macOS.
                if is_macos {
                    push_unique(&mut accel, Accelerator::CoreMl);
                }
            }
            GpuBackend::Cuda => push_unique(
                &mut accel,
                Accelerator::Cuda {
                    compute_cap: String::new(),
                },
            ),
            GpuBackend::Dx12 => push_unique(&mut accel, Accelerator::DirectMl),
            GpuBackend::Vulkan => push_unique(&mut accel, Accelerator::Vulkan),
            GpuBackend::Gl | GpuBackend::Other => {}
        }
    }

    // Report the strongest available CPU SIMD level as the CPU fallback.
    if features.avx512 {
        accel.push(Accelerator::CpuAvx512);
    } else if features.avx2 {
        accel.push(Accelerator::CpuAvx2);
    } else if features.neon {
        accel.push(Accelerator::CpuNeon);
    }

    accel
}

fn push_unique(accel: &mut Vec<Accelerator>, item: Accelerator) {
    if !accel.contains(&item) {
        accel.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    /// A fully synthetic machine for deterministic tests.
    struct FakeProbe {
        os: OsInfo,
        cpu: CpuInfo,
        mem: MemInfo,
        gpus: Vec<GpuInfo>,
    }

    impl SystemProbe for FakeProbe {
        fn os_info(&self) -> OsInfo {
            self.os.clone()
        }
        fn cpu_info(&self) -> CpuInfo {
            self.cpu.clone()
        }
        fn mem_info(&self) -> MemInfo {
            self.mem
        }
        fn gpus(&self) -> Vec<GpuInfo> {
            self.gpus.clone()
        }
        fn disk_info(&self, _p: &Path) -> DiskInfo {
            DiskInfo {
                free_bytes: 100_000_000_000,
                is_ssd: Some(true),
            }
        }
        fn power_info(&self) -> PowerInfo {
            PowerInfo {
                on_battery: false,
                battery_pct: None,
                low_power_mode: false,
            }
        }
        fn thermal_state(&self) -> ThermalState {
            ThermalState::Nominal
        }
        fn sample_usage(&mut self, _p: &Path) -> UsageSample {
            UsageSample {
                cpu_total_pct: 0.0,
                cpu_per_core: vec![0.0; self.cpu.logical_cores as usize],
                ram_used_bytes: self.mem.total_bytes - self.mem.available_bytes,
                ram_available_bytes: self.mem.available_bytes,
                swap_used_bytes: 0,
                gpu_util_pct: None,
                vram_used_bytes: None,
                app_rss_bytes: Some(90_000_000),
                worker_rss_bytes: 0,
                disk_free_bytes: 100_000_000_000,
                thermal: ThermalState::Nominal,
                battery_pct: None,
                captured_at_ms: 0,
            }
        }
    }

    fn cpu(brand: &str, cores: u32, f: CpuFeatures) -> CpuInfo {
        CpuInfo {
            brand: brand.into(),
            physical_cores: cores,
            logical_cores: cores * 2,
            base_mhz: 3000,
            features: f,
        }
    }

    fn metal_gpu() -> GpuInfo {
        GpuInfo {
            vendor: "Apple".into(),
            name: "Apple M1".into(),
            backend: GpuBackend::Metal,
            dedicated_vram_bytes: None,
            shared_bytes: None,
            driver: None,
            usable: true,
            not_usable_reason: None,
        }
    }

    fn nvidia_vulkan() -> GpuInfo {
        GpuInfo {
            vendor: "NVIDIA".into(),
            name: "RTX 3060".into(),
            backend: GpuBackend::Vulkan,
            dedicated_vram_bytes: Some(12_000_000_000),
            shared_bytes: None,
            driver: Some("555.0".into()),
            usable: true,
            not_usable_reason: None,
        }
    }

    #[test]
    fn laptop_8gb_avx2_no_gpu() {
        let probe = FakeProbe {
            os: OsInfo {
                name: "Windows".into(),
                version: "11".into(),
                arch: "x86_64".into(),
            },
            cpu: cpu(
                "Intel i5",
                4,
                CpuFeatures {
                    avx2: true,
                    ..Default::default()
                },
            ),
            mem: MemInfo {
                total_bytes: 8 << 30,
                available_bytes: 4 << 30,
                is_unified: false,
            },
            gpus: vec![],
        };
        let p = build_profile(&probe, Path::new("/tmp"));
        assert_eq!(p.accel, vec![Accelerator::CpuAvx2]);
    }

    #[test]
    fn m1_16gb_unified_metal() {
        let probe = FakeProbe {
            os: OsInfo {
                name: "Darwin".into(),
                version: "14".into(),
                arch: "aarch64".into(),
            },
            cpu: cpu(
                "Apple M1",
                8,
                CpuFeatures {
                    neon: true,
                    ..Default::default()
                },
            ),
            mem: MemInfo {
                total_bytes: 16 << 30,
                available_bytes: 10 << 30,
                is_unified: true,
            },
            gpus: vec![metal_gpu()],
        };
        let p = build_profile(&probe, Path::new("/tmp"));
        assert_eq!(
            p.accel,
            vec![
                Accelerator::MetalGpu,
                Accelerator::CoreMl,
                Accelerator::CpuNeon
            ]
        );
        // Unified memory is recorded as the GPU's shared pool.
        assert_eq!(p.gpus[0].shared_bytes, Some(16 << 30));
    }

    #[test]
    fn rtx_box_32gb_vulkan() {
        let probe = FakeProbe {
            os: OsInfo {
                name: "Linux".into(),
                version: "22.04".into(),
                arch: "x86_64".into(),
            },
            cpu: cpu(
                "Ryzen 7",
                8,
                CpuFeatures {
                    avx2: true,
                    avx512: true,
                    ..Default::default()
                },
            ),
            mem: MemInfo {
                total_bytes: 32 << 30,
                available_bytes: 24 << 30,
                is_unified: false,
            },
            gpus: vec![nvidia_vulkan()],
        };
        let p = build_profile(&probe, Path::new("/tmp"));
        // AVX-512 wins over AVX2 as the single reported SIMD level.
        assert_eq!(p.accel, vec![Accelerator::Vulkan, Accelerator::CpuAvx512]);
    }

    #[test]
    fn old_4gb_pc_no_simd_no_gpu() {
        let probe = FakeProbe {
            os: OsInfo {
                name: "Windows".into(),
                version: "10".into(),
                arch: "x86_64".into(),
            },
            cpu: cpu("Old Celeron", 2, CpuFeatures::default()),
            mem: MemInfo {
                total_bytes: 4 << 30,
                available_bytes: 2 << 30,
                is_unified: false,
            },
            gpus: vec![],
        };
        let p = build_profile(&probe, Path::new("/tmp"));
        assert!(p.accel.is_empty());
    }

    #[test]
    fn unusable_gpu_is_excluded_from_accelerators() {
        let mut gl = metal_gpu();
        gl.backend = GpuBackend::Gl;
        gl.usable = false;
        gl.not_usable_reason = Some("OpenGL only".into());
        let probe = FakeProbe {
            os: OsInfo {
                name: "Linux".into(),
                version: "22.04".into(),
                arch: "x86_64".into(),
            },
            cpu: cpu(
                "CPU",
                4,
                CpuFeatures {
                    avx2: true,
                    ..Default::default()
                },
            ),
            mem: MemInfo {
                total_bytes: 8 << 30,
                available_bytes: 4 << 30,
                is_unified: false,
            },
            gpus: vec![gl],
        };
        let p = build_profile(&probe, Path::new("/tmp"));
        assert_eq!(p.accel, vec![Accelerator::CpuAvx2]);
    }
}
