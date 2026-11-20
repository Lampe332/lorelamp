//! Windows implementation: DXGI, PDH (WDDM counters), Win32 system info.

use std::path::PathBuf;

use windows::core::{Interface, HSTRING};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_NO_INSTANCE, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_LARGE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PDH_NO_DATA,
};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};

use crate::{DriverVersion, Error, Gpu, Luid, Result, Samples, SystemInfo, Vendor, VramUsage};

fn os(call: &'static str) -> impl FnOnce(windows::core::Error) -> Error {
    move |e| Error::Os { call, detail: e.message() }
}

fn pdh_err(call: &'static str, status: u32) -> Error {
    Error::Os { call, detail: format!("PDH status 0x{status:08X}") }
}

pub fn gpus() -> Result<Vec<Gpu>> {
    // SAFETY: plain COM factory creation; the interface is released on drop.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(os("CreateDXGIFactory1"))?;
    let mut out = Vec::new();
    for i in 0.. {
        // SAFETY: `factory` is a valid interface; index past the end returns NOT_FOUND.
        let adapter = match unsafe { factory.EnumAdapters1(i) } {
            Ok(a) => a,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(os("EnumAdapters1")(e)),
        };
        // SAFETY: `adapter` is valid for the duration of the call.
        let d = unsafe { adapter.GetDesc1() }.map_err(os("GetDesc1"))?;
        if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        // Driver version: CheckInterfaceSupport(IDXGIDevice) returns the
        // user-mode driver version that the adapter itself loads. It needs no
        // mapping between the DXGI adapter and a registry/WMI device (which
        // would have to go through PnP ids and breaks with two identical
        // cards) and works for every vendor.
        // SAFETY: valid adapter and a static IID.
        let driver = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }
            .ok()
            .map(DriverVersion::from_umd);
        let len = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
        out.push(Gpu {
            name: String::from_utf16_lossy(&d.Description[..len]),
            vendor: Vendor::from_pci_id(d.VendorId),
            vendor_id: d.VendorId,
            device_id: d.DeviceId,
            luid: Luid { high: d.AdapterLuid.HighPart as u32, low: d.AdapterLuid.LowPart },
            dedicated_bytes: d.DedicatedVideoMemory as u64,
            shared_bytes: d.SharedSystemMemory as u64,
            driver,
        });
    }
    Ok(out)
}

/// Closes the PDH query on drop.
struct Query(PDH_HQUERY);

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: the handle came from PdhOpenQueryW and is closed once.
        unsafe { PdhCloseQuery(self.0) };
    }
}

const COUNTERS: [&str; 4] = [
    r"\GPU Adapter Memory(*)\Dedicated Usage",
    r"\GPU Adapter Memory(*)\Shared Usage",
    r"\GPU Process Memory(*)\Dedicated Usage",
    r"\GPU Process Memory(*)\Shared Usage",
];

pub fn vram_usage() -> Result<VramUsage> {
    let mut q = PDH_HQUERY::default();
    // SAFETY: out pointer to a local handle.
    let st = unsafe { PdhOpenQueryW(None, 0, &mut q) };
    if st != 0 {
        return Err(pdh_err("PdhOpenQueryW", st));
    }
    let query = Query(q);
    let mut handles = [PDH_HCOUNTER::default(); 4];
    for (path, h) in COUNTERS.iter().zip(handles.iter_mut()) {
        // English names so this works on every Windows display language.
        // SAFETY: valid query, null-terminated path, out pointer to a local.
        let st = unsafe { PdhAddEnglishCounterW(query.0, &HSTRING::from(*path), 0, h) };
        if st != 0 {
            return Err(pdh_err("PdhAddEnglishCounterW", st));
        }
    }
    // The memory counters are raw gauges: one sample is enough.
    // SAFETY: valid query.
    let st = unsafe { PdhCollectQueryData(query.0) };
    if st != 0 && st != PDH_NO_DATA {
        return Err(pdh_err("PdhCollectQueryData", st));
    }
    let [a, b, c, d] = handles;
    Ok(VramUsage::from_samples(&Samples {
        adapter_dedicated: read_array(a)?,
        adapter_shared: read_array(b)?,
        process_dedicated: read_array(c)?,
        process_shared: read_array(d)?,
    }))
}

/// All instances of one wildcard counter as `(instance name, value)`.
fn read_array(counter: PDH_HCOUNTER) -> Result<Vec<(String, u64)>> {
    loop {
        let (mut size, mut count) = (0u32, 0u32);
        // SAFETY: size query with no buffer.
        let st = unsafe { PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &mut size, &mut count, None) };
        if st == PDH_CSTATUS_NO_INSTANCE || st == PDH_NO_DATA {
            return Ok(Vec::new());
        }
        if st != PDH_MORE_DATA {
            return Err(pdh_err("PdhGetFormattedCounterArrayW", st));
        }
        // The buffer holds the items followed by their name strings; a Vec of
        // items keeps the required alignment.
        let item = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        let mut buf = vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); (size as usize).div_ceil(item)];
        // SAFETY: `buf` is at least `size` bytes and properly aligned.
        let st = unsafe {
            PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &mut size, &mut count, Some(buf.as_mut_ptr()))
        };
        if st == PDH_MORE_DATA {
            continue; // a process started in between; size again
        }
        if st != 0 {
            return Err(pdh_err("PdhGetFormattedCounterArrayW", st));
        }
        let mut out = Vec::with_capacity(count as usize);
        for it in &buf[..count as usize] {
            let status = it.FmtValue.CStatus;
            if status != PDH_CSTATUS_VALID_DATA && status != PDH_CSTATUS_NEW_DATA {
                continue;
            }
            // SAFETY: szName points into `buf`; PDH_FMT_LARGE selects largeValue.
            let (name, value) = unsafe { (it.szName.to_string(), it.FmtValue.Anonymous.largeValue) };
            if let Ok(name) = name {
                out.push((name, value.max(0) as u64));
            }
        }
        return Ok(out);
    }
}

pub fn system() -> Result<SystemInfo> {
    let mut p = PERFORMANCE_INFORMATION {
        cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    // SAFETY: `p` is a correctly sized out struct.
    unsafe { GetPerformanceInfo(&mut p, p.cb) }.map_err(os("GetPerformanceInfo"))?;
    let page = p.PageSize as u64;
    let (physical_cores, logical_cores) = cores()?;
    Ok(SystemInfo {
        ram_total_bytes: p.PhysicalTotal as u64 * page,
        ram_available_bytes: p.PhysicalAvailable as u64 * page,
        commit_limit_bytes: p.CommitLimit as u64 * page,
        commit_used_bytes: p.CommitTotal as u64 * page,
        cpu_name: cpu_name(),
        physical_cores,
        logical_cores,
        avx2: avx("avx2"),
        avx512f: avx("avx512f"),
    })
}

/// (physical, logical) cores over all processor groups.
fn cores() -> Result<(u32, u32)> {
    let mut len = 0u32;
    // SAFETY: size query; fails with ERROR_INSUFFICIENT_BUFFER and sets `len`.
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len) };
    let entry = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
    let mut buf = vec![SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX::default(); (len as usize).div_ceil(entry)];
    // SAFETY: `buf` holds at least `len` bytes, aligned for the struct.
    unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, Some(buf.as_mut_ptr()), &mut len) }
        .map_err(os("GetLogicalProcessorInformationEx"))?;
    let base = buf.as_ptr() as *const u8;
    let (mut physical, mut logical, mut off) = (0u32, 0u32, 0usize);
    while off < len as usize {
        // SAFETY: entries are variable-sized; `Size` gives the stride and every
        // entry starts inside the filled part of `buf`.
        let e = unsafe { &*(base.add(off) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX) };
        // SAFETY: RelationProcessorCore entries use the `Processor` variant.
        let proc_rel = unsafe { &e.Anonymous.Processor };
        physical += 1;
        // A core never spans processor groups, so GroupMask[0] is the only mask.
        logical += proc_rel.GroupMask[0].Mask.count_ones();
        off += e.Size as usize;
    }
    Ok((physical, logical))
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn cpu_name() -> String {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::__cpuid;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::__cpuid;
    #[allow(unused_unsafe)]
    // SAFETY: cpuid is available on every x86-64 CPU; extended leaves are
    // only read after checking the highest supported one.
    let bytes: Vec<u8> = unsafe {
        if __cpuid(0x8000_0000).eax < 0x8000_0004 {
            return String::new();
        }
        (0x8000_0002u32..=0x8000_0004)
            .flat_map(|leaf| {
                let r = __cpuid(leaf);
                [r.eax, r.ebx, r.ecx, r.edx].into_iter().flat_map(u32::to_le_bytes)
            })
            .collect()
    };
    String::from_utf8_lossy(&bytes).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string()
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn cpu_name() -> String {
    String::new()
}

fn avx(feature: &str) -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        match feature {
            "avx2" => std::arch::is_x86_feature_detected!("avx2"),
            "avx512f" => std::arch::is_x86_feature_detected!("avx512f"),
            _ => false,
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        let _ = feature;
        false
    }
}

pub fn rocm_runtime() -> Result<Option<PathBuf>> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    // Where Python installs live: the python.org installer (per user and for all users), the
    // new Python install manager, and the per-user sites of `pip install --user`.
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(local) = env("LOCALAPPDATA") {
        roots.push(local.join("Programs").join("Python"));
        roots.push(local.join("Python"));
    }
    for k in ["ProgramFiles", "ProgramW6432"] {
        if let Some(pf) = env(k) {
            roots.push(pf);
        }
    }
    if let Some(app) = env("APPDATA") {
        roots.push(app.join("Python"));
    }
    roots.dedup();
    let mut sites = crate::site_packages_below(&roots);
    // Every Python on PATH (a venv, conda) as well.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("python.exe").is_file() {
                let sp = dir.join("Lib").join("site-packages");
                if sp.is_dir() && !sites.contains(&sp) {
                    sites.push(sp);
                }
            }
        }
    }
    let mut hip: Vec<PathBuf> = std::env::vars_os()
        .filter(|(k, _)| k.to_string_lossy().to_ascii_uppercase().starts_with("HIP_PATH"))
        .map(|(_, v)| PathBuf::from(v))
        .collect();
    hip.sort();
    hip.dedup();
    Ok(crate::find_rocm_runtime(&sites, &hip))
}
