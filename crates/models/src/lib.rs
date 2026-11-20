//! rp-models: Hugging Face model search and resumable, checksum-verified downloads.

pub mod download;
pub mod hf;

pub use download::{download_file, free_space, Expected, Progress, Retry};
pub use hf::{GgufSummary, HfClient, ModelSummary, RepoFile, RepoInfo};
pub use tokio_util::sync::CancellationToken;

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Http(reqwest::Error),
    Io(std::io::Error),
    /// Unexpected HTTP status.
    Status { status: u16, url: String },
    /// Repo needs an access request at Hugging Face (no token support).
    Gated(String),
    NotFound(String),
    Json(String),
    TooManyRedirects,
    SizeMismatch { expected: u64, actual: u64 },
    /// The connection kept breaking before the file was complete (the `.part` file stays).
    Stalled { expected: u64, actual: u64 },
    HashMismatch { expected: String, actual: String },
    NoSpace { needed: u64, available: u64 },
    /// Stopped by the cancellation token; the `.part` file is kept for resuming.
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Http(e) => write!(f, "network error: {e}"),
            Error::Io(e) => write!(f, "file error: {e}"),
            Error::Status { status, url } => write!(f, "HTTP {status} for {url}"),
            Error::Gated(repo) => write!(f, "{repo} is gated: request access at Hugging Face"),
            Error::NotFound(what) => write!(f, "not found: {what}"),
            Error::Json(m) => write!(f, "unexpected response: {m}"),
            Error::TooManyRedirects => write!(f, "too many redirects"),
            Error::SizeMismatch { expected, actual } => {
                write!(f, "size mismatch: expected {expected} bytes, got {actual}")
            }
            Error::Stalled { expected, actual } => {
                write!(f, "download stalled at {actual} of {expected} bytes")
            }
            Error::HashMismatch { expected, actual } => {
                write!(f, "SHA-256 mismatch: expected {expected}, got {actual}")
            }
            Error::NoSpace { needed, available } => {
                write!(f, "not enough disk space: need {needed} bytes, {available} free")
            }
            Error::Cancelled => write!(f, "download cancelled"),
        }
    }
}

impl std::error::Error for Error {}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
