//! Resumable download into a `.part` file with SHA-256 check before the final rename.
//!
//! Resume uses `Range: bytes=<n>-` plus `If-Range` with the ETag of the first attempt
//! (kept next to the part file as `<name>.part.etag`). Redirects are followed by hand so
//! the range headers go to every hop (Hugging Face redirects downloads to a CDN). A server
//! that answers 200 instead of 206 starts the file over. 429 and 5xx are retried with
//! exponential backoff (or `Retry-After`).
//!
//! The size and hash the caller expects (catalog, tree listing) can be out of date. Hugging Face
//! names the current ones on the redirect (`X-Linked-Size`, `X-Linked-ETag` = SHA-256 of the
//! file); when they agree with what the CDN sends, they count instead.

use crate::{Error, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, LOCATION, RANGE, RETRY_AFTER};
use reqwest::{Client, Response, Url};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

const MAX_REDIRECTS: usize = 10;
/// Longest wait for the next bytes of an answer (headers included). The Xet CDN of Hugging Face
/// took 221 s to answer a `Range` request at an uncached offset of a 16.7 GB file (M0.7).
pub const READ_TIMEOUT: Duration = Duration::from_secs(300);
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// What the finished file must look like. Unknown values are not checked.
#[derive(Debug, Clone, Default)]
pub struct Expected {
    pub size: Option<u64>,
    /// Hex SHA-256 (as in `lfs.oid` of the Hugging Face tree).
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    /// Bytes in the part file, including those from earlier attempts.
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Average speed of this call (bytes fetched now, not resumed ones).
    pub bytes_per_sec: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Retry {
    /// Retries after the first attempt, counted since the last received bytes.
    pub attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Retry { attempts: 6, base_delay: Duration::from_secs(1), max_delay: Duration::from_secs(30) }
    }
}

impl Retry {
    fn delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(d) = retry_after {
            return d.min(Duration::from_secs(60));
        }
        let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
        self.base_delay.saturating_mul(factor).min(self.max_delay)
    }
}

/// HTTP client for all requests of this crate: redirects are followed by hand.
pub fn http_client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Lorelamp/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("static client configuration")
}

fn transient(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

fn retry_after(resp: &Response) -> Option<Duration> {
    let v = resp.headers().get(RETRY_AFTER)?.to_str().ok()?;
    v.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// What Hugging Face says about the file behind a redirect.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Linked {
    /// `X-Linked-Size`: size of the file (not of the redirect answer).
    pub size: Option<u64>,
    /// `X-Linked-ETag`: the SHA-256 (`lfs.oid`) of the file.
    pub sha256: Option<String>,
}

impl Linked {
    fn read(&mut self, h: &HeaderMap) {
        let text = |name: &str| h.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
        if let Some(size) = text("x-linked-size").and_then(|v| v.parse().ok()) {
            self.size = Some(size);
        }
        let tag = text("x-linked-etag").map(|v| v.trim_start_matches("W/").trim_matches('"').to_ascii_lowercase());
        if let Some(sha) = tag.filter(|t| t.len() == 64 && t.bytes().all(|c| c.is_ascii_hexdigit())) {
            self.sha256 = Some(sha);
        }
    }
}

/// GET that follows redirects and sends `headers` again on every hop.
async fn send_following(client: &Client, url: &str, headers: &HeaderMap) -> Result<(Response, Linked)> {
    let mut url = Url::parse(url).map_err(|e| Error::Json(format!("bad URL {url}: {e}")))?;
    let mut linked = Linked::default();
    for _ in 0..=MAX_REDIRECTS {
        let resp = client.get(url.clone()).headers(headers.clone()).send().await?;
        linked.read(resp.headers());
        if resp.status().is_redirection() {
            if let Some(loc) = resp.headers().get(LOCATION).and_then(|l| l.to_str().ok()) {
                url = url.join(loc).map_err(|e| Error::Json(format!("bad redirect {loc}: {e}")))?;
                continue;
            }
        }
        return Ok((resp, linked));
    }
    Err(Error::TooManyRedirects)
}

/// GET with redirects and retries on connection errors, 429 and 5xx. Any other status is
/// returned to the caller, as is the last transient one when the retries are used up.
pub(crate) async fn get_with_retry(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    retry: Retry,
    cancel: Option<&CancellationToken>,
) -> Result<Response> {
    get_linked(client, url, headers, retry, cancel).await.map(|(resp, _)| resp)
}

/// [`get_with_retry`] that also returns what the redirects said about the file.
async fn get_linked(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    retry: Retry,
    cancel: Option<&CancellationToken>,
) -> Result<(Response, Linked)> {
    let never = CancellationToken::new();
    let cancel = cancel.unwrap_or(&never);
    let mut attempt = 0;
    loop {
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            r = send_following(client, url, headers) => r,
        };
        let wait = match &result {
            Ok((resp, _)) if transient(resp.status().as_u16()) => retry.delay(attempt, retry_after(resp)),
            Err(Error::Http(_)) => retry.delay(attempt, None),
            _ => return result,
        };
        if attempt >= retry.attempts {
            return result;
        }
        attempt += 1;
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// Free bytes for the current user on the drive that holds `dir`.
#[cfg(windows)]
pub fn free_space(dir: &Path) -> std::io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call; the out
    // pointer is a valid u64, the two optional out pointers are null.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(free)
}

/// Free bytes for the current user on the drive that holds `dir`.
#[cfg(unix)]
pub fn free_space(dir: &Path) -> std::io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `s` a writable statvfs.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(s.f_bavail as u64 * s.f_frsize as u64)
}

fn check_space(dir: &Path, needed: u64) -> Result<()> {
    // If the drive cannot be asked, the write itself will report a full disk.
    if let Ok(available) = free_space(dir) {
        if needed > available {
            return Err(Error::NoSpace { needed, available });
        }
    }
    Ok(())
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// `bytes 100-299/300` → (100, Some(300)).
fn content_range(h: &HeaderMap) -> Option<(u64, Option<u64>)> {
    let v = h.get(CONTENT_RANGE)?.to_str().ok()?.strip_prefix("bytes ")?;
    let (range, total) = v.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    Some((start, total.trim().parse().ok()))
}

fn content_length(h: &HeaderMap) -> Option<u64> {
    h.get(CONTENT_LENGTH)?.to_str().ok()?.parse().ok()
}

/// Hash of the bytes already in the part file (off the async threads).
async fn hash_prefix(part: PathBuf) -> Result<Sha256> {
    tokio::task::spawn_blocking(move || -> Result<Sha256> {
        let mut h = Sha256::new();
        let mut f = match std::fs::File::open(&part) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(h),
            Err(e) => return Err(e.into()),
        };
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                return Ok(h);
            }
            h.update(&buf[..n]);
        }
    })
    .await
    .map_err(|e| Error::Io(std::io::Error::other(e)))?
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct State {
    part: PathBuf,
    tag: PathBuf,
    have: u64,
    etag: Option<String>,
    hasher: Sha256,
}

impl State {
    /// Throws away what was downloaded so far.
    async fn reset(&mut self) -> Result<()> {
        tokio::fs::File::create(&self.part).await?;
        let _ = tokio::fs::remove_file(&self.tag).await;
        self.have = 0;
        self.etag = None;
        self.hasher = Sha256::new();
        Ok(())
    }
}

/// Downloads `url` to `dest`. The data goes to `dest` + `.part` first; only after size and
/// SHA-256 match `expected` is it renamed to `dest`. On cancel the part file stays and a
/// later call resumes it. A file whose hash does not match is deleted.
pub async fn download_file<F>(
    client: &Client,
    url: &str,
    dest: &Path,
    expected: &Expected,
    retry: Retry,
    cancel: &CancellationToken,
    mut progress: F,
) -> Result<()>
where
    F: FnMut(Progress) + Send,
{
    let dir = dest.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(dir).await?;
    let part = with_suffix(dest, ".part");
    let tag = with_suffix(dest, ".part.etag");
    let mut expected = expected.clone();
    let mut st = State {
        have: tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0),
        etag: tokio::fs::read_to_string(&tag).await.ok().filter(|s| !s.trim().is_empty()),
        part,
        tag,
        hasher: Sha256::new(),
    };
    let mut total = expected.size;
    if st.have == 0 || total.is_some_and(|t| st.have > t) {
        st.reset().await?;
    } else {
        st.hasher = hash_prefix(st.part.clone()).await?;
    }
    if let Some(t) = total {
        check_space(dir, t - st.have)?;
    }

    let started = Instant::now();
    let start_bytes = st.have;
    let mut last_emit: Option<Instant> = None;
    let mut emit = |have: u64, total: Option<u64>, force: bool| {
        if force || last_emit.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
            let secs = started.elapsed().as_secs_f64().max(1e-3);
            progress(Progress { downloaded: have, total, bytes_per_sec: (have - start_bytes) as f64 / secs });
            last_emit = Some(Instant::now());
        }
    };

    let mut attempt = 0u32;
    while total != Some(st.have) {
        let mut headers = HeaderMap::new();
        if st.have > 0 {
            headers.insert(RANGE, HeaderValue::from_str(&format!("bytes={}-", st.have)).expect("ascii"));
            if let Some(v) = st.etag.as_deref().and_then(|e| HeaderValue::from_str(e.trim()).ok()) {
                headers.insert(IF_RANGE, v);
            }
        }
        let (resp, linked) = get_linked(client, url, &headers, retry, Some(cancel)).await?;
        match resp.status().as_u16() {
            206 => match content_range(resp.headers()) {
                Some((start, full)) if start == st.have => {
                    if full.is_some() {
                        total = full;
                    }
                }
                _ => {
                    // Range answered for the wrong offset: start over.
                    st.reset().await?;
                    attempt += 1;
                    if attempt > retry.attempts {
                        return Err(Error::Status { status: 206, url: url.to_string() });
                    }
                    continue;
                }
            },
            200 => {
                if st.have > 0 {
                    // Range ignored or the file changed (If-Range): begin anew.
                    st.reset().await?;
                }
                if let Some(len) = content_length(resp.headers()) {
                    total = Some(len);
                }
            }
            416 => {
                st.reset().await?;
                attempt += 1;
                if attempt > retry.attempts {
                    return Err(Error::Status { status: 416, url: url.to_string() });
                }
                continue;
            }
            401 | 403 => return Err(Error::Gated(url.to_string())),
            404 => return Err(Error::NotFound(url.to_string())),
            status => return Err(Error::Status { status, url: url.to_string() }),
        }
        if let Some(t) = total {
            if linked.size == Some(t) {
                // Hugging Face names the file as it is now: an out-of-date size or hash from
                // the caller gives way, the bytes are still checked against the Hub's SHA-256.
                expected.size = Some(t);
                if linked.sha256.is_some() {
                    expected.sha256 = linked.sha256.clone();
                }
            }
            if let Some(e) = expected.size.filter(|&e| e != t) {
                return Err(Error::SizeMismatch { expected: e, actual: t });
            }
            check_space(dir, t - st.have)?;
        }
        let new_tag = resp.headers().get(ETAG).and_then(|v| v.to_str().ok()).map(str::to_string);
        if let Some(t) = new_tag.filter(|t| !t.starts_with("W/")) {
            if st.etag.as_deref() != Some(t.as_str()) {
                tokio::fs::write(&st.tag, &t).await?;
                st.etag = Some(t);
            }
        }

        let file = tokio::fs::OpenOptions::new().append(true).create(true).open(&st.part).await?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        let mut body = resp.bytes_stream();
        let mut broken = false;
        loop {
            let chunk = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    out.flush().await?;
                    emit(st.have, total, true);
                    return Err(Error::Cancelled);
                }
                c = body.next() => c,
            };
            match chunk {
                Some(Ok(bytes)) => {
                    if total.is_some_and(|t| st.have + bytes.len() as u64 > t) {
                        out.flush().await?;
                        return Err(Error::SizeMismatch { expected: total.unwrap_or(0), actual: st.have + bytes.len() as u64 });
                    }
                    out.write_all(&bytes).await?;
                    st.hasher.update(&bytes);
                    st.have += bytes.len() as u64;
                    attempt = 0;
                    emit(st.have, total, false);
                }
                Some(Err(_)) => {
                    broken = true;
                    break;
                }
                None => break,
            }
        }
        out.flush().await?;
        if total.is_none() && !broken {
            // No length announced: the end of the body is the end of the file.
            total = Some(st.have);
        }
        if total != Some(st.have) {
            // Connection ended early: continue from where it stopped.
            if attempt >= retry.attempts {
                return Err(Error::Stalled { expected: total.unwrap_or(0), actual: st.have });
            }
            attempt += 1;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                _ = tokio::time::sleep(retry.delay(attempt - 1, None)) => {}
            }
        }
    }
    emit(st.have, total, true);

    let actual = hex(&st.hasher.finalize());
    if let Some(exp) = &expected.sha256 {
        if !exp.eq_ignore_ascii_case(&actual) {
            let _ = tokio::fs::remove_file(&st.part).await;
            let _ = tokio::fs::remove_file(&st.tag).await;
            return Err(Error::HashMismatch { expected: exp.clone(), actual });
        }
    }
    tokio::fs::rename(&st.part, dest).await?;
    let _ = tokio::fs::remove_file(&st.tag).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_range() {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 100-299/300"));
        assert_eq!(content_range(&h), Some((100, Some(300))));
        h.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 5-9/*"));
        assert_eq!(content_range(&h), Some((5, None)));
        h.insert(CONTENT_RANGE, HeaderValue::from_static("items 1-2/3"));
        assert_eq!(content_range(&h), None);
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let r = Retry { attempts: 5, base_delay: Duration::from_secs(1), max_delay: Duration::from_secs(10) };
        assert_eq!(r.delay(0, None), Duration::from_secs(1));
        assert_eq!(r.delay(2, None), Duration::from_secs(4));
        assert_eq!(r.delay(30, None), Duration::from_secs(10));
        assert_eq!(r.delay(0, Some(Duration::from_secs(3))), Duration::from_secs(3));
        assert_eq!(r.delay(0, Some(Duration::from_secs(999))), Duration::from_secs(60));
    }

    #[test]
    fn free_space_of_temp_dir() {
        assert!(free_space(&std::env::temp_dir()).unwrap() > 0);
    }
}
