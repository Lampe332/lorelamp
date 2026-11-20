//! Compares the two Windows sources of adapter VRAM usage for every GPU:
//! the WDDM performance counters (PDH, what `hw::vram_usage` reads) and the
//! kernel's segment statistics (`D3DKMTQueryStatistics`, per memory segment).
//! `cargo run -p hw --example vram_probe`

#[cfg(windows)]
fn main() -> Result<(), hw::Error> {
    use windows::Wdk::Graphics::Direct3D::{
        D3DKMTQueryStatistics, D3DKMT_QUERYSTATISTICS, D3DKMT_QUERYSTATISTICS_ADAPTER, D3DKMT_QUERYSTATISTICS_SEGMENT,
    };
    use windows::Win32::Foundation::LUID;

    const MIB: f64 = 1024.0 * 1024.0;
    let pid: Option<u32> = std::env::args().nth(1).and_then(|a| a.parse().ok());
    let usage = hw::vram_usage()?;
    for g in hw::gpus()? {
        let luid = LUID { LowPart: g.luid.low, HighPart: g.luid.high as i32 };
        let mut q = D3DKMT_QUERYSTATISTICS { Type: D3DKMT_QUERYSTATISTICS_ADAPTER, AdapterLuid: luid, ..Default::default() };
        // SAFETY: plain in/out struct; Type selects the AdapterInformation result.
        let st = unsafe { D3DKMTQueryStatistics(&q) };
        // SAFETY: AdapterInformation is the active union member for this Type.
        let segments = unsafe { q.QueryResult.AdapterInformation.NbSegments };
        println!("GPU {} luid {} | D3DKMT adapter status 0x{:08X}, {segments} segments", g.name, g.luid, st.0);
        let pdh = usage.adapter(g.luid);
        println!(
            "  PDH:    dedicated {:>7.0} MiB, shared {:>7.0} MiB",
            pdh.map_or(-1.0, |a| a.dedicated_bytes as f64 / MIB),
            pdh.map_or(-1.0, |a| a.shared_bytes as f64 / MIB)
        );
        let (mut local_c, mut local_r) = (0u64, 0u64);
        for id in 0..segments {
            q = D3DKMT_QUERYSTATISTICS { Type: D3DKMT_QUERYSTATISTICS_SEGMENT, AdapterLuid: luid, ..Default::default() };
            q.Anonymous.QuerySegment.SegmentId = id;
            // SAFETY: as above, SegmentInformation is the active member.
            let st = unsafe { D3DKMTQueryStatistics(&q) };
            let s = unsafe { q.QueryResult.SegmentInformation };
            println!(
                "  seg {id}: status 0x{:08X} aperture {} limit {:>7.0} committed {:>7.0} resident {:>7.0} MiB",
                st.0,
                s.Aperture,
                s.CommitLimit as f64 / MIB,
                s.BytesCommitted as f64 / MIB,
                s.BytesResident as f64 / MIB
            );
            if st.0 == 0 && s.Aperture == 0 {
                local_c += s.BytesCommitted;
                local_r += s.BytesResident;
            }
        }
        println!("  D3DKMT: local committed {:>7.0} MiB, resident {:>7.0} MiB", local_c as f64 / MIB, local_r as f64 / MIB);
        if let Some(pid) = pid {
            use windows::Wdk::Graphics::Direct3D::D3DKMT_QUERYSTATISTICS_PROCESS_SEGMENT;
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION};
            // SAFETY: plain handle open, closed below.
            let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) }) else {
                println!("  pid {pid}: OpenProcess failed");
                continue;
            };
            let p = usage.process(pid, g.luid);
            println!(
                "  pid {pid} PDH: dedicated {:>7.0} MiB, shared {:>7.0} MiB",
                p.map_or(-1.0, |p| p.dedicated_bytes as f64 / MIB),
                p.map_or(-1.0, |p| p.shared_bytes as f64 / MIB)
            );
            for id in 0..segments {
                q = D3DKMT_QUERYSTATISTICS {
                    Type: D3DKMT_QUERYSTATISTICS_PROCESS_SEGMENT,
                    AdapterLuid: luid,
                    hProcess: h,
                    ..Default::default()
                };
                q.Anonymous.QueryProcessSegment.SegmentId = id;
                // SAFETY: as above, ProcessSegmentInformation is the active member.
                let st = unsafe { D3DKMTQueryStatistics(&q) };
                let s = unsafe { q.QueryResult.ProcessSegmentInformation };
                println!("    seg {id}: status 0x{:08X} committed {:>7.0} MiB", st.0, s.BytesCommitted as f64 / MIB);
            }
            // SAFETY: handle from OpenProcess above.
            let _ = unsafe { CloseHandle(h) };
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
