//! PC detection and VRAM watch for the model package.
//!
//! Windows only: GPU list via DXGI, VRAM use via the WDDM performance counters
//! (PDH), RAM/commit/CPU via Win32. On other systems every query returns
//! [`Error::Unsupported`]. The rules on top ([`driver_warning`],
//! [`backend_suggestion`], [`VramUsage::reserve_mib`]) are pure functions.

mod backend;
mod counters;
mod driver;
mod net;
mod rocm;
#[cfg(windows)]
mod win;

use std::fmt;
use std::path::PathBuf;

pub use backend::{backend_suggestion, Backend, BackendSuggestion, MIN_DEDICATED_BYTES};
pub use counters::{
    parse_instance, AdapterUsage, Instance, ProcessUsage, Samples, VramUsage, DEFAULT_MARGIN_MIB,
    IMPLAUSIBLE_DEDICATED_BYTES, MIB,
};
pub use driver::{driver_issue, driver_warning,BadDriverRange, DriverVersion, KNOWN_BAD_DRIVERS};
pub use net::{ipv4_addresses, NetAddress};
pub use rocm::{find_rocm_runtime, site_packages_below};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Hardware queries are only implemented for Windows.
    Unsupported,
    /// A Windows API call failed.
    Os { call: &'static str, detail: String },
    /// The WDDM counters have no instance for this adapter.
    AdapterNotFound(Luid),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => f.write_str("hardware detection is only supported on Windows"),
            Error::Os { call, detail } => write!(f, "{call} failed: {detail}"),
            Error::AdapterNotFound(l) => write!(f, "no GPU memory counters for adapter {l}"),
        }
    }
}

impl std::error::Error for Error {}

/// Locally unique adapter id; links DXGI adapters to the WDDM counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Luid {
    pub high: u32,
    pub low: u32,
}

impl fmt::Display for Luid {
    /// Same form as in the counter instance names.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:08X}_0x{:08X}", self.high, self.low)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Amd,
    Nvidia,
    Intel,
    Other(u32),
}

impl Vendor {
    pub fn from_pci_id(id: u32) -> Self {
        match id {
            0x1002 => Vendor::Amd,
            0x10DE => Vendor::Nvidia,
            0x8086 => Vendor::Intel,
            other => Vendor::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub name: String,
    pub vendor: Vendor,
    pub vendor_id: u32,
    pub device_id: u32,
    pub luid: Luid,
    pub dedicated_bytes: u64,
    pub shared_bytes: u64,
    /// `None` if the driver did not report a version.
    pub driver: Option<DriverVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    pub ram_total_bytes: u64,
    pub ram_available_bytes: u64,
    /// Commit limit = RAM + page file; when commit use reaches it, allocations
    /// fail system-wide (on the dev PC this caused flicker and freezes).
    pub commit_limit_bytes: u64,
    pub commit_used_bytes: u64,
    pub cpu_name: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub avx2: bool,
    pub avx512f: bool,
}

/// Warn when less than this much commit is left ...
pub const COMMIT_WARN_FREE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// ... or more than this share of the commit limit is used.
pub const COMMIT_WARN_PERCENT: u64 = 90;

impl SystemInfo {
    pub fn commit_free_bytes(&self) -> u64 {
        self.commit_limit_bytes.saturating_sub(self.commit_used_bytes)
    }

    /// German notice for the UI when virtual memory is nearly full.
    pub fn commit_warning(&self) -> Option<String> {
        let free = self.commit_free_bytes();
        let pct = self.commit_used_bytes.saturating_mul(100) / self.commit_limit_bytes.max(1);
        (free < COMMIT_WARN_FREE_BYTES || pct > COMMIT_WARN_PERCENT).then(|| {
            format!(
                "Virtueller Arbeitsspeicher fast voll ({pct} % belegt, noch {:.1} GB frei). \
                 Programme schließen oder die Auslagerungsdatei vergrößern, sonst kann Windows einfrieren.",
                free as f64 / 1e9
            )
        })
    }
}

/// All hardware GPUs (software adapters left out).
pub fn gpus() -> Result<Vec<Gpu>> {
    imp::gpus()
}

/// One snapshot of the WDDM GPU memory counters.
pub fn vram_usage() -> Result<VramUsage> {
    imp::vram_usage()
}

/// Live reserve for llama.cpp `--fit-target`: dedicated VRAM held by all other
/// processes plus [`DEFAULT_MARGIN_MIB`]. Use [`VramUsage::reserve_mib`] for
/// another margin.
pub fn reserve_mib(adapter: Luid, own_pid: Option<u32>) -> Result<u64> {
    vram_usage()?
        .reserve_mib(adapter, own_pid, DEFAULT_MARGIN_MIB)
        .ok_or(Error::AdapterNotFound(adapter))
}

/// RAM, commit and CPU.
pub fn system() -> Result<SystemInfo> {
    imp::system()
}

/// The folder a llama.cpp HIP build needs first on PATH, if a ROCm runtime is
/// installed: TheRock in a Python `site-packages` (`_rocm_sdk_libraries_*in`)
/// or an AMD HIP SDK (`%HIP_PATH%in`), see [`find_rocm_runtime`]. The HIP
/// runtime inside the Adrenalin driver (System32) does not count: it has no rocBLAS.
pub fn rocm_runtime() -> Result<Option<PathBuf>> {
    imp::rocm_runtime()
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn gpus() -> Result<Vec<Gpu>> {
        Err(Error::Unsupported)
    }
    pub fn vram_usage() -> Result<VramUsage> {
        Err(Error::Unsupported)
    }
    pub fn system() -> Result<SystemInfo> {
        Err(Error::Unsupported)
    }
    pub fn rocm_runtime() -> Result<Option<PathBuf>> {
        Err(Error::Unsupported)
    }
}

#[cfg(windows)]
use win as imp;

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn sys(limit: u64, used: u64) -> SystemInfo {
        SystemInfo {
            ram_total_bytes: 32 * GB,
            ram_available_bytes: 8 * GB,
            commit_limit_bytes: limit,
            commit_used_bytes: used,
            cpu_name: String::new(),
            physical_cores: 8,
            logical_cores: 16,
            avx2: true,
            avx512f: false,
        }
    }

    #[test]
    fn commit_warning_thresholds() {
        assert!(sys(96 * GB, 40 * GB).commit_warning().is_none());
        assert!(sys(96 * GB, 93 * GB).commit_warning().is_some()); // < 4 GB free
        assert!(sys(200 * GB, 185 * GB).commit_warning().is_some()); // > 90 %
        assert!(sys(10 * GB, 12 * GB).commit_warning().is_some()); // over the limit
    }

    #[test]
    fn vendor_ids() {
        assert_eq!(Vendor::from_pci_id(0x1002), Vendor::Amd);
        assert_eq!(Vendor::from_pci_id(0x10de), Vendor::Nvidia);
        assert_eq!(Vendor::from_pci_id(0x8086), Vendor::Intel);
        assert_eq!(Vendor::from_pci_id(0x1414), Vendor::Other(0x1414));
    }

    /// Needs a model loaded on the largest GPU (e.g. the owner's llama-server):
    /// `cargo test -p hw live_dedicated -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a model loaded on the GPU"]
    #[cfg(windows)]
    fn live_dedicated_with_a_model_loaded() {
        let gpu = gpus().unwrap().into_iter().max_by_key(|g| g.dedicated_bytes).expect("a GPU");
        let u = vram_usage().unwrap();
        let a = u.adapter(gpu.luid).expect("counter instance for the DXGI LUID");
        println!("{}: dedicated {} MiB, shared {} MiB", gpu.name, a.dedicated_bytes / MIB, a.shared_bytes / MIB);
        let d = u.adapter_dedicated(gpu.luid).expect("plausible dedicated value");
        assert!(d > GB, "adapter dedicated {d} B with a model loaded");
    }

    #[test]
    fn luid_display_matches_counter_names() {
        let l = Luid { high: 0, low: 0x179fb };
        assert_eq!(l.to_string(), "0x00000000_0x000179FB");
        let name = format!("luid_{l}_phys_0");
        assert_eq!(parse_instance(&name).unwrap().luid, l);
    }
}
