//! Hugging Face Hub API (public repos, no token): search, repo info, file list, download.

use crate::download::{download_file, get_with_retry, http_client, Expected, Progress, Retry};
use crate::{Error, Result};
use reqwest::header::HeaderMap;
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub const HF_BASE: &str = "https://huggingface.co";

/// `gated` is `false` or a mode such as `"auto"` / `"manual"`.
fn de_gated<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<String>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Bool(true) => Some("true".into()),
        _ => None,
    })
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct GgufSummary {
    /// Parameter count.
    pub total: Option<u64>,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ModelSummary {
    pub id: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
    #[serde(default, rename = "lastModified")]
    pub last_modified: Option<String>,
    /// `None` = open; otherwise the gate mode.
    #[serde(default, deserialize_with = "de_gated")]
    pub gated: Option<String>,
    #[serde(default)]
    pub gguf: Option<GgufSummary>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RepoInfo {
    pub id: String,
    pub gated: Option<String>,
    pub license: Option<String>,
    pub last_modified: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Deserialize)]
struct RawRepoInfo {
    id: String,
    #[serde(default, deserialize_with = "de_gated")]
    gated: Option<String>,
    #[serde(default, rename = "lastModified")]
    last_modified: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default, rename = "cardData")]
    card_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RepoFile {
    pub path: String,
    pub size: u64,
    /// SHA-256 from `lfs.oid`; `None` for small files stored in git.
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct RawTreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    lfs: Option<RawLfs>,
}

#[derive(Deserialize)]
struct RawLfs {
    oid: String,
    size: Option<u64>,
}

fn repo_info(raw: RawRepoInfo) -> RepoInfo {
    let license = raw
        .card_data
        .as_ref()
        .and_then(|c| c.get("license"))
        .and_then(|l| match l {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(a) => a.first().and_then(|v| v.as_str()).map(str::to_string),
            _ => None,
        })
        .or_else(|| raw.tags.iter().find_map(|t| t.strip_prefix("license:")).map(str::to_string));
    RepoInfo { id: raw.id, gated: raw.gated, license, last_modified: raw.last_modified, tags: raw.tags }
}

#[derive(Debug, Clone)]
pub struct HfClient {
    http: Client,
    base: Url,
    pub retry: Retry,
}

impl HfClient {
    pub fn new() -> Self {
        Self::with_base(HF_BASE).expect("valid default base URL")
    }

    /// Other base URL (tests, mirrors).
    pub fn with_base(base: &str) -> Result<Self> {
        let base = Url::parse(base).map_err(|e| Error::Json(format!("bad base URL {base}: {e}")))?;
        Ok(HfClient { http: http_client(), base, retry: Retry::default() })
    }

    pub fn with_retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }

    /// The base URL (default `https://huggingface.co/`).
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// The HTTP client of this crate (redirects by hand, `Lorelamp/<version>` user agent).
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// Base URL plus path segments; a repo id `org/name` counts as two segments.
    fn url(&self, parts: &[&str]) -> Url {
        let mut u = self.base.clone();
        {
            let mut seg = u.path_segments_mut().expect("http base URL");
            seg.pop_if_empty();
            for p in parts {
                seg.extend(p.split('/').filter(|s| !s.is_empty()));
            }
        }
        u
    }

    async fn get_json<T: DeserializeOwned>(&self, url: Url, what: &str) -> Result<T> {
        let resp = get_with_retry(&self.http, url.as_str(), &HeaderMap::new(), self.retry, None).await?;
        match resp.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(Error::Gated(what.to_string())),
            404 => return Err(Error::NotFound(what.to_string())),
            status => return Err(Error::Status { status, url: url.to_string() }),
        }
        let body = resp.bytes().await?;
        serde_json::from_slice(&body).map_err(|e| Error::Json(format!("{what}: {e}")))
    }

    /// GGUF repos matching `query`, most downloaded first.
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<ModelSummary>> {
        let mut url = self.url(&["api", "models"]);
        url.query_pairs_mut()
            .append_pair("filter", "gguf")
            .append_pair("search", query)
            .append_pair("sort", "downloads")
            .append_pair("direction", "-1")
            .append_pair("limit", &limit.to_string())
            .append_pair("expand[]", "gguf")
            .append_pair("expand[]", "gated")
            .append_pair("expand[]", "downloads")
            .append_pair("expand[]", "likes")
            .append_pair("expand[]", "lastModified");
        self.get_json(url, "search").await
    }

    pub async fn repo_info(&self, repo: &str) -> Result<RepoInfo> {
        let raw: RawRepoInfo = self.get_json(self.url(&["api", "models", repo]), repo).await?;
        Ok(repo_info(raw))
    }

    /// Repo info and all files with size and SHA-256 in ONE request (`?blobs=true`: the
    /// siblings carry `size` and `lfs.sha256`). Used where every request counts (catalog sync,
    /// one request per user action).
    pub async fn repo_files(&self, repo: &str) -> Result<(RepoInfo, Vec<RepoFile>)> {
        #[derive(Deserialize)]
        struct Sibling {
            rfilename: String,
            #[serde(default)]
            size: Option<u64>,
            lfs: Option<SiblingLfs>,
        }
        #[derive(Deserialize)]
        struct SiblingLfs {
            sha256: String,
            size: Option<u64>,
        }
        #[derive(Deserialize)]
        struct Raw {
            #[serde(flatten)]
            info: RawRepoInfo,
            #[serde(default)]
            siblings: Vec<Sibling>,
        }
        let mut url = self.url(&["api", "models", repo]);
        url.query_pairs_mut().append_pair("blobs", "true");
        let raw: Raw = self.get_json(url, repo).await?;
        let files = raw
            .siblings
            .into_iter()
            .map(|s| RepoFile {
                size: s.lfs.as_ref().and_then(|l| l.size).or(s.size).unwrap_or(0),
                sha256: s.lfs.map(|l| l.sha256.to_ascii_lowercase()),
                path: s.rfilename,
            })
            .collect();
        Ok((repo_info(raw.info), files))
    }

    /// All files of the `main` branch with size and SHA-256.
    pub async fn files(&self, repo: &str) -> Result<Vec<RepoFile>> {
        let mut url = self.url(&["api", "models", repo, "tree", "main"]);
        url.query_pairs_mut().append_pair("recursive", "true");
        let raw: Vec<RawTreeEntry> = self.get_json(url, repo).await?;
        Ok(raw
            .into_iter()
            .filter(|e| e.kind == "file")
            .map(|e| RepoFile {
                size: e.lfs.as_ref().and_then(|l| l.size).unwrap_or(e.size),
                sha256: e.lfs.map(|l| l.oid.to_ascii_lowercase()),
                path: e.path,
            })
            .collect())
    }

    /// Download URL of a file on `main`.
    pub fn file_url(&self, repo: &str, path: &str) -> Url {
        self.url(&[repo, "resolve", "main", path])
    }

    /// Downloads one repo file to `dest`, checked against its size and SHA-256.
    pub async fn download<F>(
        &self,
        repo: &str,
        file: &RepoFile,
        dest: &Path,
        cancel: &CancellationToken,
        progress: F,
    ) -> Result<()>
    where
        F: FnMut(Progress) + Send,
    {
        let expected = Expected { size: Some(file.size), sha256: file.sha256.clone() };
        let url = self.file_url(repo, &file.path);
        match download_file(&self.http, url.as_str(), dest, &expected, self.retry, cancel, progress).await {
            Err(Error::Gated(_)) => Err(Error::Gated(repo.to_string())),
            other => other,
        }
    }
}

impl Default for HfClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_urls() {
        let c = HfClient::with_base("http://127.0.0.1:9/").unwrap();
        assert_eq!(
            c.file_url("org/repo", "sub dir/model-Q4_K_M.gguf").as_str(),
            "http://127.0.0.1:9/org/repo/resolve/main/sub%20dir/model-Q4_K_M.gguf"
        );
        assert_eq!(c.url(&["api", "models", "a/b"]).as_str(), "http://127.0.0.1:9/api/models/a/b");
    }

    #[test]
    fn parses_gated_and_summary() {
        let v: Vec<ModelSummary> = serde_json::from_str(
            r#"[{"id":"a/b","downloads":5,"gated":false,"gguf":{"total":100,"architecture":"llama","context_length":4096,"chat_template":"x"}},
                {"id":"c/d","gated":"manual"}]"#,
        )
        .unwrap();
        assert_eq!(v[0].gated, None);
        assert_eq!(v[0].gguf.as_ref().unwrap().total, Some(100));
        assert_eq!(v[1].gated.as_deref(), Some("manual"));
        assert_eq!(v[1].downloads, 0);
    }
}
