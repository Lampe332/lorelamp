//! "PC prüfen": one snapshot of the hardware (crate `hw`) with the suggested llama.cpp build.
//! Only a suggestion: nothing is switched by it.

use serde_json::{json, Value};

pub(crate) fn vendor(v: hw::Vendor) -> &'static str {
    match v {
        hw::Vendor::Amd => "amd",
        hw::Vendor::Nvidia => "nvidia",
        hw::Vendor::Intel => "intel",
        hw::Vendor::Other(_) => "other",
    }
}

/// A stored scan as `build` answers it today: a scan stored before the driver notice had a code
/// carries a German `driver_warning`; its `driver_issue` is read again from vendor and driver.
pub fn upgrade(scan: &mut Value) {
    let Some(gpus) = scan.get_mut("gpus").and_then(Value::as_array_mut) else { return };
    for g in gpus {
        let Some(g) = g.as_object_mut() else { continue };
        if g.remove("driver_warning").is_none() && g.contains_key("driver_issue") {
            continue;
        }
        let v = match g.get("vendor").and_then(Value::as_str) {
            Some("amd") => hw::Vendor::Amd,
            Some("nvidia") => hw::Vendor::Nvidia,
            Some("intel") => hw::Vendor::Intel,
            _ => hw::Vendor::Other(0),
        };
        let driver = g.get("driver").and_then(Value::as_str).and_then(hw::DriverVersion::parse);
        g.insert("driver_issue".into(), json!(hw::driver_issue(v, driver).map(|r| r.code)));
    }
}

/// `(suggested, cuda version)` as the API names them.
pub(crate) fn backend_name(b: hw::Backend) -> (&'static str, Option<&'static str>) {
    match b {
        hw::Backend::Cuda13 => ("cuda", Some("13")),
        hw::Backend::Cuda12 => ("cuda", Some("12")),
        hw::Backend::Rocm => ("rocm", None),
        hw::Backend::Vulkan => ("vulkan", None),
        hw::Backend::Cpu => ("cpu", None),
    }
}

/// The scan as the API returns it. `own_pid`: the model server this app runs; its VRAM does not
/// count as "used by other programs". Byte values throughout.
pub fn build(gpus: &[hw::Gpu], usage: Option<&hw::VramUsage>, sys: &hw::SystemInfo, rocm: Option<&std::path::Path>, own_pid: Option<u32>, at: i64) -> Value {
    let suggestion = hw::backend_suggestion(gpus, rocm.is_some());
    let (suggested, cuda) = backend_name(suggestion.backend);
    let gpus: Vec<Value> = gpus
        .iter()
        .map(|g| {
            let others = usage.and_then(|u| {
                let total = u.adapter_dedicated(g.luid)?;
                let own = own_pid.and_then(|p| u.process(p, g.luid)).map_or(0, |p| p.dedicated_bytes);
                Some(total.saturating_sub(own))
            });
            json!({
                "name": g.name,
                "vendor": vendor(g.vendor),
                "vram_total": g.dedicated_bytes,
                "vram_used_by_others": others,
                "driver": g.driver.map(|d| d.to_string()),
                "driver_issue": hw::driver_issue(g.vendor, g.driver).map(|r| r.code),
            })
        })
        .collect();
    json!({
        "gpus": gpus,
        "ram_total": sys.ram_total_bytes,
        "ram_free": sys.ram_available_bytes,
        "commit": { "limit": sys.commit_limit_bytes, "used": sys.commit_used_bytes, "warning": sys.commit_warning() },
        "cpu": { "name": sys.cpu_name, "cores": sys.physical_cores, "threads": sys.logical_cores, "avx2": sys.avx2 },
        "backend": {
            "suggested": suggested,
            "cuda": cuda,
            "gpu_index": suggestion.gpu,
            "reason_code": suggestion.code,
            "reason": suggestion.reason,
            "rocm_runtime_found": rocm.is_some(),
            "rocm_runtime_path": rocm.map(|p| p.display().to_string()),
        },
        "scanned_at": at,
    })
}

/// The GPU of this PC (`largest_gpu` of the adapters). Read once; a failed read (DXGI) is not
/// kept, the next call reads again.
pub fn main_gpu() -> Option<&'static hw::Gpu> {
    static GPU: std::sync::OnceLock<Option<hw::Gpu>> = std::sync::OnceLock::new();
    if let Some(known) = GPU.get() {
        return known.as_ref();
    }
    let gpus = hw::gpus().ok()?;
    GPU.get_or_init(|| largest_gpu(gpus)).as_ref()
}

/// The largest adapter with real graphics memory (at least `hw::MIN_DEDICATED_BYTES`, as for the
/// backend suggestion): the one llama.cpp uses first. Integrated graphics with a little memory
/// carved out of the RAM is no GPU to put a model on.
pub fn largest_gpu(gpus: Vec<hw::Gpu>) -> Option<hw::Gpu> {
    gpus.into_iter().filter(|g| g.dedicated_bytes >= hw::MIN_DEDICATED_BYTES).max_by_key(|g| g.dedicated_bytes)
}

/// The GPU a model server computes on, from the PC's GPU (`main_gpu`), the engine's backend
/// (`engines.backend`) and the profile's load arguments: a GPU, a build that is not the CPU build,
/// and `-ngl` not 0. None: it all runs on the CPU.
pub fn computing_gpu<'a>(gpu: Option<&'a hw::Gpu>, backend: &str, args: &[String]) -> Option<&'a hw::Gpu> {
    gpu.filter(|_| backend != "cpu" && rp_core::bench::gpu_layers(args) != Some(0))
}

/// (VRAM of the main GPU, free of it for our model server, free RAM): free = total minus what
/// everything except our own model server holds (switching profiles replaces that one).
pub fn memory_now(own_pid: Option<u32>) -> (Option<u64>, Option<u64>, Option<u64>) {
    let ram = hw::system().ok().map(|s| s.ram_available_bytes);
    let Some(gpu) = main_gpu() else { return (None, None, ram) };
    let free = hw::vram_usage().ok().and_then(|u| {
        let used = u.adapter_dedicated(gpu.luid)?;
        let own = own_pid.and_then(|pid| u.process(pid, gpu.luid)).map_or(0, |p| p.dedicated_bytes);
        Some(gpu.dedicated_bytes.saturating_sub(used.saturating_sub(own)))
    });
    (Some(gpu.dedicated_bytes), free, ram)
}

/// Reads the hardware now (blocking: DXGI, PDH, file checks).
pub fn scan(own_pid: Option<u32>) -> hw::Result<Value> {
    let gpus = hw::gpus()?;
    // Without the counters (switched off in Windows) the rest of the scan still helps.
    let usage = hw::vram_usage().ok();
    let sys = hw::system()?;
    let rocm = hw::rocm_runtime()?;
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    Ok(build(&gpus, usage.as_ref(), &sys, rocm.as_deref(), own_pid, at))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn amd(driver: &str) -> hw::Gpu {
        hw::Gpu {
            name: "AMD Radeon RX 7900 XTX".into(),
            vendor: hw::Vendor::Amd,
            vendor_id: 0x1002,
            device_id: 0x744c,
            luid: hw::Luid { high: 0, low: 0x179fb },
            dedicated_bytes: 24 * GIB,
            shared_bytes: 16 * GIB,
            driver: hw::DriverVersion::parse(driver),
        }
    }

    fn sys() -> hw::SystemInfo {
        hw::SystemInfo {
            ram_total_bytes: 32 * GIB,
            ram_available_bytes: 14 * GIB,
            commit_limit_bytes: 100 * GIB,
            commit_used_bytes: 33 * GIB,
            cpu_name: "Test CPU".into(),
            physical_cores: 24,
            logical_cores: 32,
            avx2: true,
            avx512f: false,
        }
    }

    #[test]
    fn scan_shape_rocm_and_the_driver_warning() {
        let l = "luid_0x00000000_0x000179FB_phys_0".to_string();
        let usage = hw::VramUsage::from_samples(&hw::Samples {
            adapter_dedicated: vec![(l.clone(), 9 * GIB)],
            process_dedicated: vec![(format!("pid_77_{l}"), 6 * GIB)],
            ..Default::default()
        });
        let rocm = std::path::Path::new("C:\\rocm\\bin");
        let v = build(&[amd("32.0.31036.15")], Some(&usage), &sys(), Some(rocm), Some(77), 9);
        let g = &v["gpus"][0];
        assert_eq!((g["vendor"].as_str(), g["vram_total"].as_u64()), (Some("amd"), Some(24 * GIB)));
        assert_eq!(g["vram_used_by_others"].as_u64(), Some(3 * GIB), "the app's own model server is not counted");
        assert_eq!(g["driver"], "32.0.31036.15");
        assert_eq!(g["driver_issue"], "amd_adrenalin_26_5");
        assert!(g.get("driver_warning").is_none());
        let b = &v["backend"];
        assert_eq!((b["suggested"].as_str(), b["reason_code"].as_str(), b["rocm_runtime_found"].as_bool()), (Some("rocm"), Some("amd_rocm"), Some(true)));
        assert_eq!(b["rocm_runtime_path"], "C:\\rocm\\bin");
        assert_eq!((v["cpu"]["cores"].as_u64(), v["cpu"]["threads"].as_u64(), v["ram_total"].as_u64()), (Some(24), Some(32), Some(32 * GIB)));
        assert_eq!(v["scanned_at"], 9);

        // No runtime: Vulkan is only suggested; without counters the VRAM use is unknown.
        let v = build(&[amd("32.0.21000.1")], None, &sys(), None, None, 9);
        assert_eq!((v["backend"]["suggested"].as_str(), v["backend"]["reason_code"].as_str()), (Some("vulkan"), Some("amd_no_rocm")));
        assert_eq!((v["gpus"][0]["vram_used_by_others"].is_null(), v["gpus"][0]["driver_issue"].is_null()), (true, true));
        assert_eq!(v["backend"]["rocm_runtime_path"], Value::Null);
    }

    #[test]
    fn a_stored_scan_gets_the_driver_code_instead_of_the_old_german_text() {
        let mut old = json!({ "gpus": [
            { "vendor": "amd", "driver": "32.0.31036.15", "driver_warning": "Grafiktreiber 32.0.31036.15 (…)" },
            { "vendor": "nvidia", "driver": "32.0.31036.15", "driver_warning": null },
            { "vendor": "amd", "driver": null },
        ] });
        upgrade(&mut old);
        assert_eq!(old["gpus"], json!([
            { "vendor": "amd", "driver": "32.0.31036.15", "driver_issue": "amd_adrenalin_26_5" },
            { "vendor": "nvidia", "driver": "32.0.31036.15", "driver_issue": null },
            { "vendor": "amd", "driver": null, "driver_issue": null },
        ]));
        // A new scan stays as it is.
        let mut new = old.clone();
        upgrade(&mut new);
        assert_eq!(new, old);
    }

    /// Intel UHD-like integrated graphics: a few MiB "dedicated", the rest shared system RAM.
    fn igpu() -> hw::Gpu {
        hw::Gpu {
            name: "Intel(R) UHD Graphics 770".into(),
            vendor: hw::Vendor::Intel,
            vendor_id: 0x8086,
            device_id: 0x4680,
            luid: hw::Luid { high: 0, low: 0x1234 },
            dedicated_bytes: 128 * 1024 * 1024,
            shared_bytes: 16 * GIB,
            driver: None,
        }
    }

    /// H2: only a GPU with real graphics memory, a build that is not the CPU build and layers on the
    /// GPU (`-ngl` not 0) make a model server compute on the GPU; everything else is a CPU run.
    #[test]
    fn only_real_graphics_memory_with_a_gpu_build_computes_on_the_gpu() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let none = args(&[]);
        let only_igpu = largest_gpu(vec![igpu()]);
        assert!(computing_gpu(only_igpu.as_ref(), "vulkan", &none).is_none(), "integrated graphics alone is no GPU");
        // An AMD APU with 512 MiB carved out of the RAM: still integrated.
        let apu = hw::Gpu { name: "AMD Radeon(TM) Graphics".into(), vendor: hw::Vendor::Amd, dedicated_bytes: 512 * 1024 * 1024, ..igpu() };
        assert!(computing_gpu(largest_gpu(vec![apu]).as_ref(), "vulkan", &none).is_none());
        let both = largest_gpu(vec![igpu(), amd("32.0.31036.15")]);
        assert_eq!(both.as_ref().map(|g| g.dedicated_bytes), Some(24 * GIB));
        assert!(computing_gpu(both.as_ref(), "cpu", &none).is_none(), "the CPU build never uses the card");
        assert!(computing_gpu(both.as_ref(), "rocm", &args(&["-ngl", "0"])).is_none(), "-ngl 0: all on the CPU");
        // The dedicated GPU with a GPU build: unchanged.
        assert_eq!(computing_gpu(both.as_ref(), "rocm", &none).map(|g| g.name.as_str()), Some("AMD Radeon RX 7900 XTX"));
        assert!(computing_gpu(both.as_ref(), "unknown", &args(&["-ngl", "999"])).is_some(), "a static build of unknown kind may use it");
        assert!(largest_gpu(vec![]).is_none());
    }

    #[test]
    fn cuda_carries_its_version() {
        let mut g = amd("32.0.15.8097");
        g.vendor = hw::Vendor::Nvidia;
        let v = build(&[g], None, &sys(), None, None, 0);
        assert_eq!((v["backend"]["suggested"].as_str(), v["backend"]["cuda"].as_str()), (Some("cuda"), Some("13")));
    }
}
