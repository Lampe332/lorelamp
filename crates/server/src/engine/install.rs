//! Official llama.cpp builds from GitHub: the release list (only `b…` tags with Windows zips)
//! and the install of one zip into `<data>/engines/<tag>-<variant>/`: download with SHA-256
//! check against the asset's `digest`, unpack, self-test, then the folder gets its final name.
//! Anything that fails leaves no folder behind.
//!
//! Self-test: `--version` for every build except Vulkan, `--list-devices` only for the CPU and
//! ROCm builds (the ROCm one with the ROCm runtime first on PATH, and it must list `ROCm0`; the
//! official zip silently falls back to the CPU otherwise, docs/benchmarks.md M0.6). A Vulkan
//! build is never started here: Vulkan crashed the dev PC (CLAUDE.md rule 6).

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use rp_core::llama_args;
use rp_models::{CancellationToken, Expected, Progress, Retry};

use super::probe;

pub const GITHUB_API: &str = "https://api.github.com";
const RELEASES: &str = "/repos/ggml-org/llama.cpp/releases?per_page=5";
/// Largest unpacked size of one zip (the ROCm build unpacks to ~1.5 GB).
const MAX_UNPACKED: u64 = 8 * 1024 * 1024 * 1024;
/// Free space asked for before a download: the zip plus what it unpacks to.
const SPACE_FACTOR: u64 = 6;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Asset {
    pub name: String,
    /// `cpu`, `vulkan`, `rocm-10.0`, `cuda-12.4`: part of the folder name.
    pub variant: String,
    /// `cpu`, `vulkan`, `rocm`, `cuda`.
    pub backend: String,
    pub size: u64,
    /// Hex SHA-256 from the asset's `digest`.
    pub sha256: String,
    pub url: String,
    /// CUDA builds: the zip with the CUDA runtime DLLs that goes into the same folder.
    pub runtime: Option<Box<Asset>>,
}

impl Asset {
    /// Bytes to download, including the runtime zip.
    pub fn total_size(&self) -> u64 {
        self.size + self.runtime.as_ref().map_or(0, |r| r.size)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Release {
    pub tag: String,
    pub published_at: String,
    pub assets: Vec<Asset>,
}

/// `x64` or `arm64`: the asset names' word for this program's CPU.
pub fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

fn is_build_tag(tag: &str) -> bool {
    tag.strip_prefix('b').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `cpu` / `vulkan` / `rocm-…` / `hip-…` / `cuda-…` with its backend; other builds (SYCL,
/// OpenVINO, Adreno) are not offered.
fn backend_of(variant: &str) -> Option<&'static str> {
    match variant {
        "cpu" => Some("cpu"),
        "vulkan" => Some("vulkan"),
        v if v.starts_with("rocm-") || v.starts_with("hip-") || v == "hip" => Some("rocm"),
        v if v.starts_with("cuda-") => Some("cuda"),
        _ => None,
    }
}

fn sha256_of(asset: &Value) -> Option<String> {
    let hex = asset["digest"].as_str()?.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
}

fn plain_asset(a: &Value, variant: &str, backend: &str) -> Option<Asset> {
    Some(Asset {
        name: a["name"].as_str()?.to_string(),
        variant: variant.to_string(),
        backend: backend.to_string(),
        size: a["size"].as_u64()?,
        sha256: sha256_of(a)?,
        url: a["browser_download_url"].as_str()?.to_string(),
        runtime: None,
    })
}

/// The releases answer of the GitHub API, reduced to build tags with Windows zips for `arch`.
/// Assets without a SHA-256 `digest` are left out (nothing is installed unchecked).
pub fn parse_releases(json: &str, arch: &str) -> Result<Vec<Release>, String> {
    let all: Vec<Value> = serde_json::from_str(json).map_err(|e| format!("release list: {e}"))?;
    let suffix = format!("-{arch}.zip");
    let mut out = Vec::new();
    for r in &all {
        let Some(tag) = r["tag_name"].as_str().filter(|t| is_build_tag(t)) else { continue };
        if r["draft"].as_bool() == Some(true) {
            continue;
        }
        let empty = Vec::new();
        let raw = r["assets"].as_array().unwrap_or(&empty);
        let prefix = format!("llama-{tag}-bin-win-");
        let runtime = |version: &str| {
            let name = format!("cudart-llama-bin-win-cuda-{version}{suffix}");
            raw.iter().find(|a| a["name"].as_str() == Some(name.as_str())).and_then(|a| plain_asset(a, "cudart", "cuda"))
        };
        let mut assets: Vec<Asset> = raw
            .iter()
            .filter_map(|a| {
                let variant = a["name"].as_str()?.strip_prefix(&prefix)?.strip_suffix(&suffix)?;
                let backend = backend_of(variant)?;
                let mut asset = plain_asset(a, variant, backend)?;
                if backend == "cuda" {
                    asset.runtime = runtime(variant.trim_start_matches("cuda-")).map(Box::new);
                }
                Some(asset)
            })
            .collect();
        if assets.is_empty() {
            continue;
        }
        let order = |a: &Asset| ["cpu", "rocm", "cuda", "vulkan"].iter().position(|b| *b == a.backend).unwrap_or(9);
        assets.sort_by(|a, b| order(a).cmp(&order(b)).then_with(|| a.variant.cmp(&b.variant)));
        out.push(Release { tag: tag.to_string(), published_at: r["published_at"].as_str().unwrap_or_default().to_string(), assets });
    }
    Ok(out)
}

/// The newest releases from `api_base` (normally [`GITHUB_API`]).
pub async fn fetch_releases(client: &reqwest::Client, api_base: &str) -> Result<Vec<Release>, String> {
    let resp = client
        .get(format!("{api_base}{RELEASES}"))
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub answered {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| format!("GitHub: {e}"))?;
    parse_releases(&text, arch())
}

/// Where an install stands, for the progress events.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Download,
    Unpack,
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

/// Why an install failed: a code for the UI and a detail for the log.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub code: &'static str,
    pub detail: String,
}

fn fail(code: &'static str, detail: impl std::fmt::Display) -> Failure {
    Failure { code, detail: detail.to_string() }
}

/// What the self-test found; stored with the engine.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SelfTest {
    pub ok: bool,
    /// What was run: `list-devices`, `version`; empty for a Vulkan build.
    pub ran: Vec<&'static str>,
    /// GPUs from `--list-devices` (`ROCm0: AMD Radeon RX 7900 XTX (24560 MiB, …)`).
    pub devices: Vec<String>,
    pub version: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub dir: PathBuf,
    pub exe: PathBuf,
    pub extra_path: Vec<PathBuf>,
    pub selftest: SelfTest,
}

/// The folder a build goes to: `<engines>/<tag>-<variant>`.
pub fn target_dir(engines: &Path, tag: &str, asset: &Asset) -> PathBuf {
    engines.join(format!("{tag}-{}", asset.variant))
}

fn safe_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 80 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_') && !s.starts_with('.')
}

/// Downloads, checks, unpacks and tests `asset` of release `tag`. `rocm_runtime`: the folder the
/// ROCm build needs on PATH (required for it). On success the build is in [`target_dir`]; on any
/// failure nothing of it is left.
pub async fn install(
    client: &reqwest::Client,
    engines: &Path,
    tag: &str,
    asset: &Asset,
    rocm_runtime: Option<&Path>,
    progress: impl FnMut(Step) + Send,
) -> Result<Installed, Failure> {
    install_with(client, engines, tag, asset, rocm_runtime, progress, self_test).await
}

type TestFn = fn(&Path, &str, &[PathBuf]) -> Result<SelfTest, Failure>;

async fn install_with(
    client: &reqwest::Client,
    engines: &Path,
    tag: &str,
    asset: &Asset,
    rocm_runtime: Option<&Path>,
    mut progress: impl FnMut(Step) + Send,
    test: TestFn,
) -> Result<Installed, Failure> {
    if !is_build_tag(tag) || !safe_name(&asset.variant) || !safe_name(&asset.name) {
        return Err(fail("bad_asset", format!("{tag} / {}", asset.name)));
    }
    let extra_path: Vec<PathBuf> = match (asset.backend.as_str(), rocm_runtime) {
        ("rocm", Some(r)) => vec![r.to_path_buf()],
        ("rocm", None) => return Err(fail("rocm_runtime_missing", "no ROCm runtime found")),
        _ => Vec::new(),
    };
    let dir = target_dir(engines, tag, asset);
    if dir.exists() {
        return Err(fail("engine_exists", dir.display()));
    }
    std::fs::create_dir_all(engines).map_err(|e| fail("storage", e))?;
    let free = rp_models::free_space(engines).map_err(|e| fail("storage", e))?;
    if free < asset.total_size().saturating_mul(SPACE_FACTOR) {
        return Err(fail("no_space", format!("{free} bytes free")));
    }
    let staging = engines.join(format!(".{tag}-{}.part", asset.variant));
    let downloads = engines.join(".download");
    let result = async {
        let _ = std::fs::remove_dir_all(&staging);
        let mut zips = vec![asset];
        zips.extend(asset.runtime.as_deref());
        let total = asset.total_size();
        let mut before = 0u64;
        for z in zips {
            let dest = downloads.join(&z.name);
            let expected = Expected { size: Some(z.size), sha256: Some(z.sha256.clone()) };
            let got = rp_models::download_file(client, &z.url, &dest, &expected, Retry::default(), &CancellationToken::new(), |p: Progress| {
                progress(Step { phase: Phase::Download, done: before + p.downloaded, total })
            })
            .await;
            match got {
                Ok(()) => {}
                Err(rp_models::Error::HashMismatch { .. } | rp_models::Error::SizeMismatch { .. }) => {
                    return Err(fail("hash_mismatch", z.name.clone()))
                }
                Err(rp_models::Error::NoSpace { .. }) => return Err(fail("no_space", z.name.clone())),
                Err(e) => return Err(fail("download_failed", e)),
            }
            before += z.size;
            progress(Step { phase: Phase::Unpack, done: 0, total: 1 });
            let (zip, into) = (dest.clone(), staging.clone());
            let unpacked = tokio::task::spawn_blocking(move || unpack(&zip, &into)).await.map_err(|e| fail("storage", e))?;
            let _ = std::fs::remove_file(&dest);
            unpacked?;
        }
        let exe_rel = find_server(&staging).ok_or_else(|| fail("bad_zip", "no llama-server.exe in the zip"))?;
        progress(Step { phase: Phase::Test, done: 0, total: 1 });
        let (exe, backend, extra) = (staging.join(&exe_rel), asset.backend.clone(), extra_path.clone());
        let mut selftest = tokio::task::spawn_blocking(move || test(&exe, &backend, &extra)).await.map_err(|e| fail("storage", e))??;
        if selftest.version.is_none() {
            selftest.version = Some(tag.to_string()); // the Vulkan build is not run: its tag says it
        }
        std::fs::rename(&staging, &dir).map_err(|e| fail("storage", e))?;
        Ok(Installed { exe: dir.join(&exe_rel), dir: dir.clone(), extra_path: extra_path.clone(), selftest })
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&dir);
        for z in std::iter::once(asset).chain(asset.runtime.as_deref()) {
            let _ = std::fs::remove_file(downloads.join(&z.name));
        }
    }
    let _ = std::fs::remove_dir(&downloads); // only when empty
    result
}

/// Unpacks `zip` into `into` (created). Entries that would land outside are refused.
fn unpack(zip: &Path, into: &Path) -> Result<(), Failure> {
    let file = std::fs::File::open(zip).map_err(|e| fail("storage", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| fail("bad_zip", e))?;
    let mut written = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| fail("bad_zip", e))?;
        let rel = entry.enclosed_name().ok_or_else(|| fail("bad_zip", format!("unsafe path {}", entry.name())))?;
        let path = into.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| fail("storage", e))?;
            continue;
        }
        written += entry.size();
        if written > MAX_UNPACKED {
            return Err(fail("bad_zip", "unpacks to more than 8 GB"));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| fail("storage", e))?;
        }
        let mut out = std::fs::File::create(&path).map_err(|e| fail("storage", e))?;
        let size = entry.size();
        std::io::copy(&mut (&mut entry).take(size), &mut out).map_err(|e| fail("bad_zip", e))?;
    }
    Ok(())
}

/// `llama-server.exe` relative to `dir`: at the top or one folder down.
fn find_server(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    if dir.join(name).is_file() {
        return Some(PathBuf::from(name));
    }
    std::fs::read_dir(dir).ok()?.flatten().filter(|e| e.path().is_dir()).map(|e| PathBuf::from(e.file_name()).join(name)).find(|rel| dir.join(rel).is_file())
}

/// GPU lines of `--list-devices` (`  ROCm0: AMD Radeon …`).
pub fn parse_devices(text: &str) -> Option<Vec<String>> {
    let mut lines = text.lines().map(str::trim).skip_while(|l| !l.starts_with("Available devices"));
    lines.next()?;
    Some(lines.filter(|l| !l.is_empty() && *l != "(none)" && l.contains(':')).map(str::to_string).collect())
}

/// Runs the self-test of one build (see the module comment for what runs when).
pub fn self_test(exe: &Path, backend: &str, extra_path: &[PathBuf]) -> Result<SelfTest, Failure> {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    if backend == "vulkan" {
        let dir = exe.parent().unwrap_or(Path::new("."));
        if !dir.join(if cfg!(windows) { "ggml-vulkan.dll" } else { "libggml-vulkan.so" }).is_file() {
            return Err(fail("bad_zip", "no ggml-vulkan in the Vulkan build"));
        }
        return Ok(SelfTest { ok: true, ran: Vec::new(), devices: Vec::new(), version: None, at });
    }
    let mut ran = Vec::new();
    let mut devices = Vec::new();
    if backend == "cpu" || backend == "rocm" {
        let out = probe::run(exe, extra_path, "--list-devices").map_err(|e| fail("selftest_failed", e))?;
        devices = parse_devices(&out).ok_or_else(|| fail("selftest_failed", "--list-devices printed no device list"))?;
        ran.push("list-devices");
        if backend == "rocm" && !devices.iter().any(|d| d.starts_with("ROCm")) {
            return Err(fail("rocm_no_gpu", "the ROCm build sees no GPU (it would run on the CPU)"));
        }
    }
    let out = probe::run(exe, extra_path, "--version").map_err(|e| fail("selftest_failed", e))?;
    let version = llama_args::parse_version(&out).ok_or_else(|| fail("selftest_failed", "--version printed no build number"))?;
    ran.push("version");
    Ok(SelfTest { ok: true, ran, devices, version: Some(version), at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    /// A real answer of `GET /repos/ggml-org/llama.cpp/releases?per_page=5` (14.11.2026).
    const FIXTURE: &str = include_str!("../../tests/fixtures/llama-releases.json");

    #[test]
    fn parses_the_real_release_list() {
        let r = parse_releases(FIXTURE, "x64").unwrap();
        assert_eq!(r.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(), ["b12387", "b12386", "b12385", "b12383", "b12381"]);
        let b = &r[0];
        let names: Vec<_> = b.assets.iter().map(|a| (a.variant.as_str(), a.backend.as_str())).collect();
        assert_eq!(names, [("cpu", "cpu"), ("rocm-10.0", "rocm"), ("cuda-12.4", "cuda"), ("cuda-13.5", "cuda"), ("vulkan", "vulkan")]);
        let cpu = &b.assets[0];
        assert_eq!((cpu.name.as_str(), cpu.size), ("llama-b12387-bin-win-cpu-x64.zip", 25_124_944));
        assert!(cpu.sha256.starts_with("ced8482efac76") && cpu.sha256.len() == 64);
        assert!(cpu.url.ends_with("/b12387/llama-b12387-bin-win-cpu-x64.zip"));
        let rocm = &b.assets[1];
        assert_eq!((rocm.size, rocm.runtime.is_none()), (280_412_960, true));
        assert!(rocm.sha256.starts_with("ccc0c39f87ca1"));
        let cuda = &b.assets[2];
        let rt = cuda.runtime.as_ref().unwrap();
        assert_eq!((rt.name.as_str(), rt.size), ("cudart-llama-bin-win-cuda-12.4-x64.zip", 391_443_627));
        assert_eq!(cuda.total_size(), 319_407_424 + 391_443_627);
        // arm64 gets its own zips; no ROCm build for it.
        let arm = parse_releases(FIXTURE, "arm64").unwrap();
        assert!(arm[0].assets.iter().all(|a| a.name.ends_with("-arm64.zip") && a.backend != "rocm"));
    }

    #[test]
    fn only_build_tags_and_checked_assets() {
        let json = r#"[
          {"tag_name":"v1.0.0","draft":false,"assets":[{"name":"llama-v1.0.0-bin-win-cpu-x64.zip","size":1,"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","browser_download_url":"u"}]},
          {"tag_name":"b2","draft":true,"assets":[{"name":"llama-b2-bin-win-cpu-x64.zip","size":1,"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","browser_download_url":"u"}]},
          {"tag_name":"b3","draft":false,"assets":[{"name":"llama-b3-bin-win-cpu-x64.zip","size":1,"digest":null,"browser_download_url":"u"},
                                                   {"name":"llama-b3-bin-win-sycl-x64.zip","size":1,"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","browser_download_url":"u"}]},
          {"tag_name":"b4","draft":false,"assets":[{"name":"llama-b4-bin-win-vulkan-x64.zip","size":7,"digest":"sha256:ABCDEF0000000000000000000000000000000000000000000000000000000000","browser_download_url":"u"}]}
        ]"#;
        let r = parse_releases(json, "x64").unwrap();
        assert_eq!(r.len(), 1, "v-tags, drafts, unchecked and unknown builds are left out");
        assert_eq!((r[0].tag.as_str(), r[0].assets[0].sha256.as_str()), ("b4", "abcdef0000000000000000000000000000000000000000000000000000000000"));
        assert!(parse_releases("{}", "x64").is_err());
    }

    #[test]
    fn device_list() {
        assert_eq!(parse_devices("load_backend: x\nAvailable devices:\n  (none)\n"), Some(vec![]));
        assert_eq!(
            parse_devices("Available devices:\n  ROCm0: AMD Radeon RX 7900 XTX (24560 MiB, 24411 MiB free)\n"),
            Some(vec!["ROCm0: AMD Radeon RX 7900 XTX (24560 MiB, 24411 MiB free)".to_string()])
        );
        assert_eq!(parse_devices("error: unknown argument"), None);
    }

    fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// A real HTTP server on a free port that serves `body` at `/f.zip`.
    async fn serve(body: Vec<u8>) -> String {
        let app = axum::Router::new().route("/f.zip", get(move || async move { body.clone() }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/f.zip")
    }

    fn asset(url: &str, data: &[u8], sha256: Option<String>) -> Asset {
        Asset {
            name: "llama-b7-bin-win-cpu-x64.zip".into(),
            variant: "cpu".into(),
            backend: "cpu".into(),
            size: data.len() as u64,
            sha256: sha256.unwrap_or_else(|| format!("{:x}", Sha256::digest(data))),
            url: url.into(),
            runtime: None,
        }
    }

    fn engines_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rp-engines-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default()
    }

    fn passes(exe: &Path, _: &str, _: &[PathBuf]) -> Result<SelfTest, Failure> {
        assert!(exe.is_file(), "the test runs on the unpacked program");
        Ok(SelfTest { ok: true, ran: vec!["version"], devices: vec![], version: Some("b7".into()), at: 1 })
    }

    #[tokio::test]
    async fn download_checks_unpacks_and_names_the_folder() {
        let data = zip_bytes(&[("llama-server.exe", b"MZ test"), ("ggml-cpu.dll", b"dll"), ("sub/readme.txt", b"hi")]);
        let url = serve(data.clone()).await;
        let engines = engines_dir("ok");
        let client = rp_models::download::http_client();
        let mut steps = Vec::new();
        let got = install_with(&client, &engines, "b7", &asset(&url, &data, None), None, |s| steps.push(s.phase), passes).await.unwrap();
        assert_eq!(got.dir, engines.join("b7-cpu"));
        assert_eq!(std::fs::read(&got.exe).unwrap(), b"MZ test");
        assert_eq!(std::fs::read(got.dir.join("sub").join("readme.txt")).unwrap(), b"hi");
        assert_eq!(steps.first(), Some(&Phase::Download));
        assert!(steps.contains(&Phase::Unpack) && steps.last() == Some(&Phase::Test));
        assert_eq!(leftovers(&engines), ["b7-cpu"], "no zip, no staging folder");
        // A second install of the same build is refused and leaves the first alone.
        let again = install_with(&client, &engines, "b7", &asset(&url, &data, None), None, |_| {}, passes).await;
        assert_eq!(again.unwrap_err().code, "engine_exists");
        assert!(got.exe.is_file());
        let _ = std::fs::remove_dir_all(engines);
    }

    #[tokio::test]
    async fn wrong_digest_leaves_nothing() {
        let data = zip_bytes(&[("llama-server.exe", b"MZ")]);
        let url = serve(data.clone()).await;
        let engines = engines_dir("digest");
        let client = rp_models::download::http_client();
        let bad = asset(&url, &data, Some("0".repeat(64)));
        let err = install_with(&client, &engines, "b7", &bad, None, |_| {}, passes).await.unwrap_err();
        assert_eq!(err.code, "hash_mismatch");
        assert!(leftovers(&engines).is_empty(), "{:?}", leftovers(&engines));
        let _ = std::fs::remove_dir_all(engines);
    }

    #[tokio::test]
    async fn failed_selftest_rolls_back() {
        // Not a program: the real self-test cannot start it.
        let data = zip_bytes(&[("llama-server.exe", b"not a program")]);
        let url = serve(data.clone()).await;
        let engines = engines_dir("selftest");
        let client = rp_models::download::http_client();
        let err = install_with(&client, &engines, "b7", &asset(&url, &data, None), None, |_| {}, self_test).await.unwrap_err();
        assert_eq!(err.code, "selftest_failed");
        assert!(leftovers(&engines).is_empty(), "{:?}", leftovers(&engines));
        // A zip without llama-server is refused too.
        let data = zip_bytes(&[("readme.txt", b"x")]);
        let url = serve(data.clone()).await;
        let err = install_with(&client, &engines, "b7", &asset(&url, &data, None), None, |_| {}, passes).await.unwrap_err();
        assert_eq!(err.code, "bad_zip");
        assert!(leftovers(&engines).is_empty());
        let _ = std::fs::remove_dir_all(engines);
    }

    #[tokio::test]
    async fn rocm_needs_a_runtime_and_zip_slip_is_refused() {
        let engines = engines_dir("rocm");
        let client = rp_models::download::http_client();
        let mut a = asset("http://127.0.0.1:1/x.zip", b"x", None);
        a.backend = "rocm".into();
        a.variant = "rocm-10.0".into();
        let err = install_with(&client, &engines, "b7", &a, None, |_| {}, passes).await.unwrap_err();
        assert_eq!(err.code, "rocm_runtime_missing");
        let err = install_with(&client, &engines, "../b7", &asset("u", b"x", None), None, |_| {}, passes).await.unwrap_err();
        assert_eq!(err.code, "bad_asset");

        let data = zip_bytes(&[("../evil.exe", b"x")]);
        let url = serve(data.clone()).await;
        let err = install_with(&client, &engines, "b7", &asset(&url, &data, None), None, |_| {}, passes).await.unwrap_err();
        assert_eq!(err.code, "bad_zip");
        assert!(!engines.parent().unwrap().join("evil.exe").exists());
        assert!(leftovers(&engines).is_empty());
        let _ = std::fs::remove_dir_all(engines);
    }

    /// Real: lists the GitHub releases, installs the newest CPU zip into a temp folder and runs its
    /// self-test. `cargo test -p rp-server real_cpu_engine -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network: GitHub release list and a ~25 MB download"]
    async fn real_cpu_engine_from_github() {
        let client = rp_models::download::http_client();
        let releases = fetch_releases(&client, GITHUB_API).await.unwrap();
        let r = &releases[0];
        let cpu = r.assets.iter().find(|a| a.backend == "cpu").expect("a CPU zip");
        println!("release {} asset {} ({} bytes)", r.tag, cpu.name, cpu.size);
        let engines = engines_dir("real");
        let got = install(&client, &engines, &r.tag, cpu, None, |_| {}).await.unwrap();
        println!("installed {} selftest {:?}", got.exe.display(), got.selftest);
        assert_eq!(got.selftest.version.as_deref(), Some(r.tag.as_str()));
        assert_eq!(got.selftest.ran, ["list-devices", "version"]);
        let _ = std::fs::remove_dir_all(engines);
    }
}
