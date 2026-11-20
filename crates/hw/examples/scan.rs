//! Prints everything `hw` detects on this PC: `cargo run -p hw --example scan`.

const MIB: f64 = 1024.0 * 1024.0;
const GIB: f64 = MIB * 1024.0;

fn main() -> Result<(), hw::Error> {
    let gpus = hw::gpus()?;
    let usage = hw::vram_usage()?;
    for (i, g) in gpus.iter().enumerate() {
        let driver = g.driver.map_or("?".to_string(), |d| d.to_string());
        println!(
            "GPU {i}: {} ({:?}, {:04X}:{:04X}) luid {} | VRAM {:.1} GiB, shared {:.1} GiB | driver {driver}",
            g.name, g.vendor, g.vendor_id, g.device_id, g.luid,
            g.dedicated_bytes as f64 / GIB, g.shared_bytes as f64 / GIB,
        );
        match usage.adapter(g.luid) {
            Some(a) => println!(
                "  in use: dedicated {:.0} MiB, shared {:.0} MiB | reserve (no own pid) {} MiB",
                a.dedicated_bytes as f64 / MIB,
                a.shared_bytes as f64 / MIB,
                usage.reserve_mib(g.luid, None, hw::DEFAULT_MARGIN_MIB).unwrap_or(0),
            ),
            None => println!("  in use: no counter instance"),
        }
        if usage.adapter(g.luid).is_some() && usage.adapter_dedicated(g.luid).is_none() {
            println!("  dedicated counter implausible (Windows reports ~0): treated as unknown");
        }
        let mut top: Vec<_> = usage.processes.iter().filter(|p| p.luid == g.luid && p.dedicated_bytes > 0).collect();
        top.sort_by_key(|p| std::cmp::Reverse(p.dedicated_bytes));
        for p in top.iter().take(5) {
            println!(
                "    pid {:>6}: dedicated {:>6.0} MiB, shared {:>5.0} MiB",
                p.pid,
                p.dedicated_bytes as f64 / MIB,
                p.shared_bytes as f64 / MIB
            );
        }
        if let Some(w) = hw::driver_warning(g) {
            println!("  WARNING: {w}");
        }
    }
    let rocm = hw::rocm_runtime()?;
    println!("ROCm runtime: {}", rocm.as_ref().map_or("none".into(), |p| p.display().to_string()));
    let s = hw::backend_suggestion(&gpus, rocm.is_some());
    println!("Backend: {:?} (gpu {:?}) - {}", s.backend, s.gpu, s.reason);

    let sys = hw::system()?;
    println!(
        "RAM {:.1} GiB, available {:.1} GiB | commit {:.1} / {:.1} GiB ({:.1} GiB free)",
        sys.ram_total_bytes as f64 / GIB,
        sys.ram_available_bytes as f64 / GIB,
        sys.commit_used_bytes as f64 / GIB,
        sys.commit_limit_bytes as f64 / GIB,
        sys.commit_free_bytes() as f64 / GIB,
    );
    if let Some(w) = sys.commit_warning() {
        println!("  WARNING: {w}");
    }
    println!(
        "CPU {} | {} cores / {} threads | AVX2 {} | AVX-512F {}",
        sys.cpu_name, sys.physical_cores, sys.logical_cores, sys.avx2, sys.avx512f
    );
    Ok(())
}
