//! Finding a ROCm runtime for the llama.cpp HIP build. Plain file system checks, no OS calls,
//! so the rule is testable everywhere; the Windows side only collects the candidate folders.
//!
//! The official `win-rocm` zip ships `amdhip64_7.dll` but no rocBLAS/hipBLASLt. Without a
//! runtime folder on PATH it silently runs on the CPU (docs/benchmarks.md, M0.6).

use std::path::{Path, PathBuf};

/// The folder to put first on PATH: a TheRock runtime from a Python `site-packages`
/// (`_rocm_sdk_libraries_*\bin` holding `rocblas.dll`, measured to work in M0.6), else the
/// `bin` of an AMD HIP SDK (`HIP_PATH`) holding `amdhip64*.dll`. Earlier candidates win.
pub fn find_rocm_runtime(site_packages: &[PathBuf], hip_paths: &[PathBuf]) -> Option<PathBuf> {
    site_packages.iter().find_map(|sp| therock_bin(sp)).or_else(|| hip_paths.iter().find_map(|h| hip_sdk_bin(h)))
}

fn names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).collect())
        .unwrap_or_default()
}

fn therock_bin(site_packages: &Path) -> Option<PathBuf> {
    let mut libs: Vec<PathBuf> = std::fs::read_dir(site_packages)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("_rocm_sdk_libraries"))
        .map(|e| e.path().join("bin"))
        .collect();
    libs.sort();
    libs.into_iter().find(|bin| names(bin).iter().any(|n| n == "rocblas.dll"))
}

fn hip_sdk_bin(hip_path: &Path) -> Option<PathBuf> {
    let bin = hip_path.join("bin");
    names(&bin).iter().any(|n| n.starts_with("amdhip64") && n.ends_with(".dll")).then_some(bin)
}

/// `site-packages` folders of the Python installs below `roots` (folders like `Python312`
/// with `Lib\site-packages`, or user sites that are `site-packages` themselves), newest
/// version name first so a fresh install wins over an old one.
pub fn site_packages_below(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        let Ok(it) = std::fs::read_dir(root) else { continue };
        let mut found: Vec<PathBuf> = it
            .flatten()
            .map(|e| e.path())
            .flat_map(|p| [p.join("Lib").join("site-packages"), p.join("site-packages")])
            .filter(|p| p.is_dir())
            .collect();
        found.sort();
        found.reverse();
        out.extend(found);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hw-rocm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"").unwrap();
    }

    #[test]
    fn therock_before_hip_sdk_and_needs_rocblas() {
        let root = tmp("order");
        let py = root.join("Programs").join("Python312");
        let sp = py.join("Lib").join("site-packages");
        let hip = root.join("ROCm").join("6.4");
        touch(&hip.join("bin").join("amdhip64_6.dll"));
        // A TheRock folder without rocBLAS does not count.
        std::fs::create_dir_all(sp.join("_rocm_sdk_libraries_custom").join("bin")).unwrap();
        let sps = site_packages_below(&[root.join("Programs")]);
        assert_eq!(sps, vec![sp.clone()]);
        assert_eq!(find_rocm_runtime(&sps, std::slice::from_ref(&hip)), Some(hip.join("bin")));
        touch(&sp.join("_rocm_sdk_libraries_custom").join("bin").join("rocblas.dll"));
        assert_eq!(find_rocm_runtime(&sps, &[hip]), Some(sp.join("_rocm_sdk_libraries_custom").join("bin")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_found() {
        let root = tmp("none");
        let hip = root.join("hip");
        std::fs::create_dir_all(hip.join("bin")).unwrap();
        touch(&hip.join("bin").join("hipblas.dll")); // no amdhip64*.dll
        assert_eq!(find_rocm_runtime(&site_packages_below(std::slice::from_ref(&root)), &[hip, root.join("missing")]), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn newest_python_first() {
        let root = tmp("newest");
        for v in ["Python311", "Python313"] {
            std::fs::create_dir_all(root.join(v).join("Lib").join("site-packages")).unwrap();
        }
        let sps = site_packages_below(std::slice::from_ref(&root));
        assert!(sps[0].starts_with(root.join("Python313")), "{sps:?}");
        let _ = std::fs::remove_dir_all(root);
    }
}
