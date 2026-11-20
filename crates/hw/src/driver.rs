//! Driver versions and the list of known problem drivers.

use std::fmt;

use crate::{Gpu, Vendor};

/// A Windows driver version `a.b.c.d` (the user-mode driver version that
/// Device Manager and `Win32_VideoController.DriverVersion` show).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DriverVersion(pub [u16; 4]);

impl DriverVersion {
    /// Decodes the packed value DXGI returns from
    /// `IDXGIAdapter::CheckInterfaceSupport` (16 bits per part, high to low).
    pub fn from_umd(raw: i64) -> Self {
        let raw = raw as u64;
        DriverVersion([
            (raw >> 48) as u16,
            (raw >> 32) as u16,
            (raw >> 16) as u16,
            raw as u16,
        ])
    }

    /// Parses `a.b.c.d`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = [0u16; 4];
        let mut it = s.trim().split('.');
        for p in &mut parts {
            *p = it.next()?.parse().ok()?;
        }
        if it.next().is_some() {
            return None;
        }
        Some(DriverVersion(parts))
    }

    /// NVIDIA release number (e.g. `580.97`) from the Windows version
    /// (e.g. `32.0.15.8097`): last digit of part 3 followed by part 4.
    pub fn nvidia_release(&self) -> Option<(u32, u32)> {
        let [_, _, c, d] = self.0;
        if d >= 10_000 {
            return None;
        }
        let n = u32::from(c % 10) * 10_000 + u32::from(d);
        Some((n / 100, n % 100))
    }
}

impl fmt::Display for DriverVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a}.{b}.{c}.{d}")
    }
}

/// A range of driver versions (inclusive) with a known problem.
#[derive(Debug, Clone, Copy)]
pub struct BadDriverRange {
    /// Names the problem for the UI, which puts it in words in the app's language
    /// (`models.pc.driverIssue.<code>`).
    pub code: &'static str,
    pub vendor: Vendor,
    pub from: DriverVersion,
    pub to: DriverVersion,
    /// Marketing name of the affected releases.
    pub release: &'static str,
    pub problem: &'static str,
}

/// Known problem drivers. Keep this list short and sourced.
///
/// AMD Adrenalin 26.5.1 to 26.7.x (Windows driver 32.0.31007.x to 32.0.31036.x):
/// - silent crash while loading a model in llama-server, also RX 7900 XTX with
///   26.5.2; rollback to 26.3.1 fixes it:
///   <https://github.com/ggml-org/llama.cpp/issues/23508>
/// - VRAM evicted to system RAM after ~15 s idle, then `0x116 VIDEO_TDR_FAILURE`;
///   26.3.1 clean, 26.5.1+ affected, 26.7.1 (32.0.31035.1003) still affected:
///   <https://github.com/ggml-org/llama.cpp/discussions/23443>
pub const KNOWN_BAD_DRIVERS: &[BadDriverRange] = &[BadDriverRange {
    code: "amd_adrenalin_26_5",
    vendor: Vendor::Amd,
    from: DriverVersion([32, 0, 31007, 0]),
    to: DriverVersion([32, 0, 31036, u16::MAX]),
    release: "Adrenalin 26.5.1 to 26.7.x",
    problem: "with Vulkan, crashes while loading the model and VRAM evicted while idle \
              (up to a screen reset) were reported. ROCm is not affected",
}];

/// The known problem of this driver, if it is on the list of known problem drivers. Only a
/// hint; nothing is blocked. The UI says it by its `code`.
pub fn driver_issue(vendor: Vendor, driver: Option<DriverVersion>) -> Option<&'static BadDriverRange> {
    let v = driver?;
    KNOWN_BAD_DRIVERS.iter().find(|r| r.vendor == vendor && r.from <= v && v <= r.to)
}

/// [`driver_issue`] as one English line (the scan example on the command line).
pub fn driver_warning(gpu: &Gpu) -> Option<String> {
    let v = gpu.driver?;
    driver_issue(gpu.vendor, gpu.driver).map(|r| format!("graphics driver {v} ({}): {}.", r.release, r.problem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Luid, Vendor};

    fn gpu(vendor: Vendor, driver: &str) -> Gpu {
        Gpu {
            name: "test".into(),
            vendor,
            vendor_id: 0,
            device_id: 0,
            luid: Luid { high: 0, low: 1 },
            dedicated_bytes: 0,
            shared_bytes: 0,
            driver: DriverVersion::parse(driver),
        }
    }

    #[test]
    fn umd_decoding() {
        let raw: i64 = (32 << 48) | (31036 << 16) | 15;
        assert_eq!(DriverVersion::from_umd(raw), DriverVersion([32, 0, 31036, 15]));
        assert_eq!(DriverVersion::from_umd(raw).to_string(), "32.0.31036.15");
    }

    #[test]
    fn parse() {
        assert_eq!(DriverVersion::parse("32.0.15.8097"), Some(DriverVersion([32, 0, 15, 8097])));
        assert_eq!(DriverVersion::parse("1.2.3"), None);
        assert_eq!(DriverVersion::parse("1.2.3.4.5"), None);
        assert_eq!(DriverVersion::parse("1.2.x.4"), None);
    }

    #[test]
    fn nvidia_release() {
        let v = DriverVersion::parse("32.0.15.8097").unwrap();
        assert_eq!(v.nvidia_release(), Some((580, 97)));
        let v = DriverVersion::parse("31.0.15.5222").unwrap();
        assert_eq!(v.nvidia_release(), Some((552, 22)));
    }

    #[test]
    fn amd_bad_range_edges() {
        assert!(driver_warning(&gpu(Vendor::Amd, "32.0.31007.5012")).is_some());
        assert!(driver_warning(&gpu(Vendor::Amd, "32.0.31007.0")).is_some());
        assert!(driver_warning(&gpu(Vendor::Amd, "32.0.31035.1003")).is_some());
        assert!(driver_warning(&gpu(Vendor::Amd, "32.0.31036.15")).is_some());
        assert!(driver_warning(&gpu(Vendor::Amd, "32.0.31006.9999")).is_none());
        assert!(driver_warning(&gpu(Vendor::Amd, "32.0.31037.0")).is_none());
        assert!(driver_warning(&gpu(Vendor::Amd, "31.0.24033.1003")).is_none());
    }

    #[test]
    fn other_vendor_or_unknown_driver_no_warning() {
        assert!(driver_warning(&gpu(Vendor::Nvidia, "32.0.31010.0")).is_none());
        assert!(driver_warning(&gpu(Vendor::Amd, "")).is_none());
    }

    #[test]
    fn warning_names_version() {
        let w = driver_warning(&gpu(Vendor::Amd, "32.0.31036.15")).unwrap();
        assert!(w.contains("32.0.31036.15"));
        assert!(w.contains("26.5.1"));
        assert!(w.contains("Vulkan") && w.contains("ROCm is not affected"));
        assert_eq!(driver_issue(Vendor::Amd, DriverVersion::parse("32.0.31036.15")).map(|r| r.code), Some("amd_adrenalin_26_5"));
        assert!(driver_issue(Vendor::Nvidia, DriverVersion::parse("32.0.31036.15")).is_none());
    }
}
