//! WDDM GPU memory counters (`GPU Adapter Memory`, `GPU Process Memory`):
//! instance name parsing and the reserve calculation. Pure, no I/O.
//!
//! These counters are the only source on the dev machine (RX 7900 XTX, ROCm/HIP)
//! that sees real allocations; HIP's free-memory query stays constant
//! (docs/benchmarks.md, M0.1).

use crate::Luid;

pub const MIB: u64 = 1024 * 1024;

/// Safety margin added to the reserve when none is given.
pub const DEFAULT_MARGIN_MIB: u64 = 1024;

/// Parsed counter instance name, e.g. `luid_0x00000000_0x000179FB_phys_0`
/// or `pid_1234_luid_0x00000000_0x000179FB_phys_0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instance {
    pub pid: Option<u32>,
    pub luid: Luid,
    pub phys: u32,
}

pub fn parse_instance(name: &str) -> Option<Instance> {
    let (pid, rest) = match name.strip_prefix("pid_") {
        Some(r) => {
            let (pid, rest) = r.split_once('_')?;
            (Some(pid.parse().ok()?), rest)
        }
        None => (None, name),
    };
    let rest = rest.strip_prefix("luid_")?;
    let mut parts = rest.split('_');
    let high = hex32(parts.next()?)?;
    let low = hex32(parts.next()?)?;
    if parts.next()? != "phys" {
        return None;
    }
    let phys = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Instance { pid, luid: Luid { high, low }, phys })
}

fn hex32(s: &str) -> Option<u32> {
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    u32::from_str_radix(h, 16).ok()
}

/// Memory in use on one adapter (all processes and the kernel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterUsage {
    pub luid: Luid,
    pub dedicated_bytes: u64,
    pub shared_bytes: u64,
}

/// Below this, a dedicated value that is smaller than the shared one is not believed.
pub const IMPLAUSIBLE_DEDICATED_BYTES: u64 = 512 * MIB;

impl AdapterUsage {
    /// `false` when Windows' dedicated figure cannot be real: less than 512 MiB and less than the
    /// shared figure. Windows sometimes reports (almost) 0 dedicated for the whole adapter and every
    /// process while the shared values stay correct; `Get-Counter` and `D3DKMTQueryStatistics`
    /// (the kernel's resident/committed bytes) show the same. Seen on the dev PC (RX 7900 XTX,
    /// 2026-11-15, display off): adapter 0 MiB dedicated / 4,440 MiB shared while llama-server held
    /// ~20 GB in VRAM (11 MiB dedicated / 4,355 MiB shared). Trusting it would make the whole card
    /// look free. A working discrete GPU always holds more dedicated than that (desktop surfaces);
    /// an integrated GPU (small carve-out, mostly shared) is also reported as unknown, which is
    /// right: its "free VRAM" is not a dedicated figure.
    pub fn dedicated_plausible(&self) -> bool {
        self.dedicated_bytes >= IMPLAUSIBLE_DEDICATED_BYTES || self.dedicated_bytes >= self.shared_bytes
    }
}

/// Memory one process uses on one adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessUsage {
    pub pid: u32,
    pub luid: Luid,
    pub dedicated_bytes: u64,
    /// Overflow into system memory. If a model server has a large value here,
    /// it did not fit into dedicated VRAM and runs much slower.
    pub shared_bytes: u64,
}

/// One snapshot of the WDDM memory counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VramUsage {
    pub adapters: Vec<AdapterUsage>,
    pub processes: Vec<ProcessUsage>,
}

/// Raw counter samples, `(instance name, bytes)` per counter.
#[derive(Debug, Default)]
pub struct Samples {
    pub adapter_dedicated: Vec<(String, u64)>,
    pub adapter_shared: Vec<(String, u64)>,
    pub process_dedicated: Vec<(String, u64)>,
    pub process_shared: Vec<(String, u64)>,
}

impl VramUsage {
    /// Builds the snapshot; `phys_N` parts of one adapter are summed,
    /// unparsable instance names are skipped.
    pub fn from_samples(s: &Samples) -> Self {
        let mut u = VramUsage::default();
        for (name, v, shared) in tagged(&s.adapter_dedicated, &s.adapter_shared) {
            let Some(i) = parse_instance(name).filter(|i| i.pid.is_none()) else { continue };
            let a = match u.adapters.iter_mut().find(|a| a.luid == i.luid) {
                Some(a) => a,
                None => {
                    u.adapters.push(AdapterUsage { luid: i.luid, dedicated_bytes: 0, shared_bytes: 0 });
                    u.adapters.last_mut().unwrap()
                }
            };
            *if shared { &mut a.shared_bytes } else { &mut a.dedicated_bytes } += v;
        }
        for (name, v, shared) in tagged(&s.process_dedicated, &s.process_shared) {
            let Some(i) = parse_instance(name) else { continue };
            let Some(pid) = i.pid else { continue };
            let p = match u.processes.iter_mut().find(|p| p.pid == pid && p.luid == i.luid) {
                Some(p) => p,
                None => {
                    u.processes.push(ProcessUsage { pid, luid: i.luid, dedicated_bytes: 0, shared_bytes: 0 });
                    u.processes.last_mut().unwrap()
                }
            };
            *if shared { &mut p.shared_bytes } else { &mut p.dedicated_bytes } += v;
        }
        u
    }

    pub fn adapter(&self, luid: Luid) -> Option<&AdapterUsage> {
        self.adapters.iter().find(|a| a.luid == luid)
    }

    /// Dedicated VRAM in use on the adapter, `None` if there is no counter instance or the value
    /// cannot be true (see [`AdapterUsage::dedicated_plausible`]). Use this, not the raw field,
    /// for every decision about free VRAM.
    pub fn adapter_dedicated(&self, luid: Luid) -> Option<u64> {
        self.adapter(luid).filter(|a| a.dedicated_plausible()).map(|a| a.dedicated_bytes)
    }

    pub fn process(&self, pid: u32, luid: Luid) -> Option<&ProcessUsage> {
        self.processes.iter().find(|p| p.pid == pid && p.luid == luid)
    }

    /// Dedicated VRAM held by everything except `own_pid`, plus `margin_mib`,
    /// rounded up to whole MiB. This is the value for llama.cpp `--fit-target`.
    ///
    /// Uses the adapter total minus the own process instead of summing the
    /// per-process values: per-process counters double count shared surfaces
    /// (dwm.exe reports more than the card has), while the adapter total is
    /// what Task Manager shows. `None` if the adapter has no counter instance.
    pub fn reserve_mib(&self, adapter: Luid, own_pid: Option<u32>, margin_mib: u64) -> Option<u64> {
        let total = self.adapter_dedicated(adapter)?;
        let own = own_pid
            .and_then(|pid| self.process(pid, adapter))
            .map_or(0, |p| p.dedicated_bytes);
        Some(total.saturating_sub(own).div_ceil(MIB) + margin_mib)
    }
}

fn tagged<'a>(
    dedicated: &'a [(String, u64)],
    shared: &'a [(String, u64)],
) -> impl Iterator<Item = (&'a str, u64, bool)> {
    dedicated
        .iter()
        .map(|(n, v)| (n.as_str(), *v, false))
        .chain(shared.iter().map(|(n, v)| (n.as_str(), *v, true)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Luid = Luid { high: 0, low: 0x179fb };

    #[test]
    fn parse_adapter_instance() {
        let i = parse_instance("luid_0x00000000_0x000179FB_phys_0").unwrap();
        assert_eq!(i, Instance { pid: None, luid: L, phys: 0 });
    }

    #[test]
    fn parse_process_instance() {
        let i = parse_instance("pid_2776_luid_0x00000000_0x000179fb_phys_1").unwrap();
        assert_eq!(i, Instance { pid: Some(2776), luid: L, phys: 1 });
    }

    #[test]
    fn parse_high_part() {
        let i = parse_instance("luid_0x00000001_0xFFFFFFFF_phys_0").unwrap();
        assert_eq!(i.luid, Luid { high: 1, low: u32::MAX });
    }

    #[test]
    fn parse_rejects_garbage() {
        for bad in [
            "",
            "_Total",
            "luid_0x0_phys_0",
            "luid_00000000_0x000179fb_phys_0",
            "pid_x_luid_0x00000000_0x000179fb_phys_0",
            "luid_0x00000000_0x000179fb_eng_0",
            "luid_0x00000000_0x000179fb_phys_0_eng_3",
            "pid_1_luid_0x00000000_0x0000zz_phys_0",
        ] {
            assert_eq!(parse_instance(bad), None, "{bad}");
        }
    }

    fn s(v: &[(&str, u64)]) -> Vec<(String, u64)> {
        v.iter().map(|(n, b)| (n.to_string(), *b)).collect()
    }

    fn sample() -> VramUsage {
        VramUsage::from_samples(&Samples {
            adapter_dedicated: s(&[
                ("luid_0x00000000_0x000179fb_phys_0", 8000 * MIB),
                ("luid_0x00000000_0x00019a8e_phys_0", 0),
                ("garbage", 5),
            ]),
            adapter_shared: s(&[("luid_0x00000000_0x000179fb_phys_0", 300 * MIB)]),
            process_dedicated: s(&[
                ("pid_10_luid_0x00000000_0x000179fb_phys_0", 2000 * MIB),
                ("pid_10_luid_0x00000000_0x000179fb_phys_1", 500 * MIB),
                ("pid_20_luid_0x00000000_0x000179fb_phys_0", 40_000 * MIB),
            ]),
            process_shared: s(&[("pid_10_luid_0x00000000_0x000179fb_phys_0", 3000 * MIB)]),
        })
    }

    #[test]
    fn samples_are_grouped() {
        let u = sample();
        assert_eq!(u.adapters.len(), 2);
        let a = u.adapter(L).unwrap();
        assert_eq!((a.dedicated_bytes, a.shared_bytes), (8000 * MIB, 300 * MIB));
        let p = u.process(10, L).unwrap();
        assert_eq!((p.dedicated_bytes, p.shared_bytes), (2500 * MIB, 3000 * MIB));
        assert_eq!(u.processes.len(), 2);
    }

    #[test]
    fn reserve_without_own_process() {
        assert_eq!(sample().reserve_mib(L, None, DEFAULT_MARGIN_MIB), Some(9024));
    }

    #[test]
    fn reserve_subtracts_own_process() {
        assert_eq!(sample().reserve_mib(L, Some(10), DEFAULT_MARGIN_MIB), Some(6524));
        // Unknown own pid: nothing subtracted.
        assert_eq!(sample().reserve_mib(L, Some(99), 0), Some(8000));
    }

    #[test]
    fn reserve_ignores_bogus_process_totals_and_saturates() {
        // pid 20 reports more than the adapter (like dwm.exe): never below margin.
        assert_eq!(sample().reserve_mib(L, Some(20), 512), Some(512));
    }

    /// Real counter values of the dev PC with Orion loaded in llama-server: `night` read by
    /// `Get-Counter` during the P6 benchmark (2026-11-15 06:57, dedicated broken), `day` at 07:25.
    fn real(night: bool) -> VramUsage {
        let (ad, ash, pd, psh) = if night { (0, 4440, 11, 4355) } else { (15_879, 4465, 15_402, 4374) };
        VramUsage::from_samples(&Samples {
            adapter_dedicated: s(&[
                ("luid_0x00000000_0x000179fb_phys_0", ad * MIB),
                ("luid_0x00000000_0x00019a8e_phys_0", 0),
            ]),
            adapter_shared: s(&[
                ("luid_0x00000000_0x000179fb_phys_0", ash * MIB),
                ("luid_0x00000000_0x00019a8e_phys_0", 0),
            ]),
            process_dedicated: s(&[
                ("pid_69476_luid_0x00000000_0x000179fb_phys_0", pd * MIB),
                ("pid_12464_luid_0x00000000_0x000179fb_phys_0", 646 * MIB),
            ]),
            process_shared: s(&[("pid_69476_luid_0x00000000_0x000179fb_phys_0", psh * MIB)]),
        })
    }

    #[test]
    fn broken_dedicated_counter_is_unknown_not_free() {
        let u = real(true);
        assert!(!u.adapter(L).unwrap().dedicated_plausible());
        assert_eq!(u.adapter_dedicated(L), None, "0 MiB with 4.4 GB shared must not read as an empty card");
        assert_eq!(u.reserve_mib(L, Some(69476), DEFAULT_MARGIN_MIB), None);
        // Shared still works and is what the overflow alarm reads.
        assert_eq!(u.process(69476, L).unwrap().shared_bytes, 4355 * MIB);
    }

    #[test]
    fn working_dedicated_counter_is_used() {
        let u = real(false);
        assert_eq!(u.adapter_dedicated(L), Some(15_879 * MIB));
        assert_eq!(u.reserve_mib(L, Some(69476), 0), Some(477));
        // The idle second adapter (0 / 0) is a real zero, not a broken counter.
        assert_eq!(u.adapter_dedicated(Luid { high: 0, low: 0x19a8e }), Some(0));
    }

    #[test]
    fn plausibility_edges() {
        let a = |d: u64, sh: u64| AdapterUsage { luid: L, dedicated_bytes: d * MIB, shared_bytes: sh * MIB };
        assert!(a(512, 9000).dedicated_plausible(), "512 MiB and more is always believed");
        assert!(!a(511, 512).dedicated_plausible());
        assert!(a(200, 200).dedicated_plausible(), "small headless card, nothing shared");
        assert!(a(23_500, 8000).dedicated_plausible(), "full card spilling into shared");
    }

    #[test]
    fn reserve_rounds_up_and_unknown_adapter() {
        let u = VramUsage::from_samples(&Samples {
            adapter_dedicated: s(&[("luid_0x00000000_0x00000001_phys_0", MIB + 1)]),
            ..Samples::default()
        });
        assert_eq!(u.reserve_mib(Luid { high: 0, low: 1 }, None, 0), Some(2));
        assert_eq!(u.reserve_mib(L, None, 0), None);
    }
}
