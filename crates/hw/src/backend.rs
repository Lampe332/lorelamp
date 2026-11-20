//! Which llama.cpp build fits the detected PC. Pure rule, only a suggestion.

use crate::{Gpu, Vendor};

/// A GPU below this much dedicated memory (integrated graphics) is not worth
/// offloading to; the CPU build is suggested instead.
pub const MIN_DEDICATED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// CUDA 13 build: NVIDIA driver 580 or newer (Turing and newer only).
    Cuda13,
    /// CUDA 12 build: NVIDIA driver 525 to 579, or older cards.
    Cuda12,
    /// HIP/ROCm build: AMD with an installed ROCm/HIP runtime.
    Rocm,
    Vulkan,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSuggestion {
    pub backend: Backend,
    /// Index into the GPU list the suggestion is for; `None` for CPU.
    pub gpu: Option<usize>,
    /// Stable code of the rule that decided (`no_gpu`, `nvidia_cuda13`, `nvidia_cuda12`,
    /// `nvidia_driver_old`, `nvidia_driver_unknown`, `amd_rocm`, `amd_no_rocm`, `intel`,
    /// `other_vendor`).
    pub code: &'static str,
    /// Short German reason for the UI.
    pub reason: String,
}

/// NVIDIA PCI device ids below this are cards before Turing (GTX 10 and older, Volta): CUDA 13
/// no longer builds kernels for them. 0 means "not known".
const NVIDIA_FIRST_TURING_ID: u32 = 0x1E00;

/// Picks the GPU with the most dedicated memory and suggests a build:
/// NVIDIA by driver release (>= 580 CUDA 13, but CUDA 12 for a card before Turing; 525..=579 CUDA
/// 12; an unknown release CUDA 12, which runs from driver 525; older than 525 Vulkan), AMD ROCm if
/// `rocm_runtime` else Vulkan, Intel Vulkan (SYCL named as the alternative), anything else CPU.
pub fn backend_suggestion(gpus: &[Gpu], rocm_runtime: bool) -> BackendSuggestion {
    let best = gpus
        .iter()
        .enumerate()
        .filter(|(_, g)| g.dedicated_bytes >= MIN_DEDICATED_BYTES)
        .max_by_key(|(_, g)| g.dedicated_bytes);
    let Some((i, g)) = best else {
        return BackendSuggestion {
            backend: Backend::Cpu,
            gpu: None,
            code: "no_gpu",
            reason: "Keine Grafikkarte mit eigenem Speicher gefunden.".into(),
        };
    };
    let (backend, code, reason) = match g.vendor {
        Vendor::Nvidia => match g.driver.and_then(|d| d.nvidia_release()) {
            Some((major, _)) if major >= 580 && g.device_id != 0 && g.device_id < NVIDIA_FIRST_TURING_ID => (
                Backend::Cuda12,
                "nvidia_cuda12",
                format!("NVIDIA-Treiber {major}, Karte vor RTX 20 / GTX 16: CUDA 12."),
            ),
            Some((major, _)) if major >= 580 => (
                Backend::Cuda13,
                "nvidia_cuda13",
                format!("NVIDIA-Treiber {major}: CUDA 13 (ältere Karten vor RTX 20 brauchen CUDA 12)."),
            ),
            Some((major, _)) if major >= 525 => {
                (Backend::Cuda12, "nvidia_cuda12", format!("NVIDIA-Treiber {major}: CUDA 12."))
            }
            Some((major, _)) => (
                Backend::Vulkan,
                "nvidia_driver_old",
                format!("NVIDIA-Treiber {major} ist zu alt für CUDA: Vulkan."),
            ),
            None => (
                Backend::Cuda12,
                "nvidia_driver_unknown",
                "NVIDIA-Treiberversion unbekannt: CUDA 12 (braucht Treiber 525 oder neuer).".into(),
            ),
        },
        Vendor::Amd if rocm_runtime => (Backend::Rocm, "amd_rocm", "AMD-Grafikkarte mit installierter ROCm-Laufzeit: ROCm.".into()),
        Vendor::Amd => (
            Backend::Vulkan,
            "amd_no_rocm",
            "AMD-Grafikkarte ohne ROCm-Laufzeit: Vulkan wäre möglich, die App schaltet aber nicht selbst um.".into(),
        ),
        Vendor::Intel => (Backend::Vulkan, "intel", "Intel-Grafik: Vulkan (ein SYCL-Build wäre die Alternative).".into()),
        Vendor::Other(_) => (Backend::Cpu, "other_vendor", "Unbekannter Grafikkarten-Hersteller: CPU.".into()),
    };
    BackendSuggestion { backend, gpu: Some(i), code, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DriverVersion, Luid};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn gpu(vendor: Vendor, gib: u64, driver: &str) -> Gpu {
        Gpu {
            name: "test".into(),
            vendor,
            vendor_id: 0,
            device_id: 0,
            luid: Luid { high: 0, low: 1 },
            dedicated_bytes: gib * GIB,
            shared_bytes: 16 * GIB,
            driver: DriverVersion::parse(driver),
        }
    }

    fn pick(gpus: &[Gpu], rocm: bool) -> Backend {
        backend_suggestion(gpus, rocm).backend
    }

    #[test]
    fn nvidia_by_driver() {
        assert_eq!(pick(&[gpu(Vendor::Nvidia, 12, "32.0.15.8097")], false), Backend::Cuda13);
        assert_eq!(pick(&[gpu(Vendor::Nvidia, 12, "32.0.15.8000")], false), Backend::Cuda13);
        assert_eq!(pick(&[gpu(Vendor::Nvidia, 12, "31.0.15.7999")], false), Backend::Cuda12);
        assert_eq!(pick(&[gpu(Vendor::Nvidia, 12, "31.0.15.2500")], false), Backend::Cuda12);
        assert_eq!(pick(&[gpu(Vendor::Nvidia, 12, "30.0.15.2499")], false), Backend::Vulkan);
        // Unknown release: CUDA 12 (an NVIDIA card gets CUDA, not Vulkan, unless the driver is known old).
        assert_eq!(pick(&[gpu(Vendor::Nvidia, 12, "")], false), Backend::Cuda12);
    }

    #[test]
    fn nvidia_before_turing_stays_on_cuda_12() {
        let mut pascal = gpu(Vendor::Nvidia, 8, "32.0.15.8097");
        pascal.device_id = 0x1B80; // GTX 1080
        let s = backend_suggestion(&[pascal.clone()], false);
        assert_eq!((s.backend, s.code), (Backend::Cuda12, "nvidia_cuda12"));
        let mut turing = pascal;
        turing.device_id = 0x2182; // GTX 1660 Ti
        assert_eq!(pick(&[turing], false), Backend::Cuda13);
    }

    #[test]
    fn every_rule_has_its_code() {
        let code = |g: &[Gpu], rocm| backend_suggestion(g, rocm).code;
        assert_eq!(code(&[], false), "no_gpu");
        assert_eq!(code(&[gpu(Vendor::Nvidia, 12, "32.0.15.8097")], false), "nvidia_cuda13");
        assert_eq!(code(&[gpu(Vendor::Nvidia, 12, "31.0.15.5222")], false), "nvidia_cuda12");
        assert_eq!(code(&[gpu(Vendor::Nvidia, 12, "30.0.15.2499")], false), "nvidia_driver_old");
        assert_eq!(code(&[gpu(Vendor::Nvidia, 12, "")], false), "nvidia_driver_unknown");
        assert_eq!(code(&[gpu(Vendor::Amd, 24, "")], true), "amd_rocm");
        assert_eq!(code(&[gpu(Vendor::Amd, 24, "")], false), "amd_no_rocm");
        assert_eq!(code(&[gpu(Vendor::Intel, 8, "")], false), "intel");
        assert_eq!(code(&[gpu(Vendor::Other(7), 8, "")], false), "other_vendor");
    }

    #[test]
    fn amd_by_runtime() {
        let g = [gpu(Vendor::Amd, 24, "32.0.31036.15")];
        assert_eq!(pick(&g, true), Backend::Rocm);
        assert_eq!(pick(&g, false), Backend::Vulkan);
    }

    #[test]
    fn intel_other_and_none() {
        assert_eq!(pick(&[gpu(Vendor::Intel, 16, "")], true), Backend::Vulkan);
        assert_eq!(pick(&[gpu(Vendor::Other(0x1234), 8, "")], true), Backend::Cpu);
        assert_eq!(pick(&[], true), Backend::Cpu);
    }

    #[test]
    fn largest_dedicated_gpu_wins_and_igpu_is_skipped() {
        let g = [gpu(Vendor::Intel, 0, ""), gpu(Vendor::Nvidia, 8, "32.0.15.8097"), gpu(Vendor::Amd, 24, "")];
        let s = backend_suggestion(&g, false);
        assert_eq!((s.backend, s.gpu), (Backend::Vulkan, Some(2)));
        // Only an iGPU with little dedicated memory: CPU.
        let s = backend_suggestion(&[gpu(Vendor::Amd, 1, "")], true);
        assert_eq!((s.backend, s.gpu), (Backend::Cpu, None));
    }
}
