//! Serializable, user-facing error type shared by every IPC command.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum AppError {
    #[error("yt-dlp was not found. Install a recent yt-dlp and make sure it is on your PATH.")]
    YtDlpMissing,
    #[error("yt-dlp failed: {0}")]
    YtDlp(String),
    #[error("No results found.")]
    EmptyResults,
    #[error("This track is unavailable: {0}")]
    Unavailable(String),
    #[error("This content is DRM-protected and cannot be streamed: {0}")]
    Drm(String),
    #[error("This content is not available in your region.")]
    RegionLocked,
    #[error("Network problem: {0}")]
    Network(String),
    #[error("Timed out: {0}")]
    Timeout(String),
    #[error("Could not extract a playable stream: {0}")]
    Extraction(String),
    #[error("Unsupported URL or source: {0}")]
    Unsupported(String),
    #[error("Audio device problem: {0}")]
    AudioDevice(String),
    #[error("Could not decode audio: {0}")]
    Decode(String),
    #[error("Plugin error: {0}")]
    Plugin(String),
    #[error("Theme error: {0}")]
    Theme(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Invalid input: {0}")]
    Invalid(String),
    #[error("Storage error: {0}")]
    Storage(String),
    /// The data changed since the caller looked (sync retries with fresh data).
    #[error("Changed meanwhile: {0}")]
    Conflict(String),
    #[error("Cancelled")]
    Cancelled,
}

impl AppError {
    /// Whether retrying the same operation could plausibly succeed.
    pub fn is_transient(&self) -> bool {
        matches!(self, AppError::Network(_) | AppError::Timeout(_))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Storage(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Invalid(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Turn yt-dlp's stderr into a typed error.
pub fn classify_ytdlp_stderr(stderr: &str) -> AppError {
    let s = stderr.to_lowercase();
    let last = stderr
        .lines()
        .rev()
        .find(|l| l.contains("ERROR"))
        .unwrap_or_else(|| stderr.lines().last().unwrap_or("unknown error"))
        .trim()
        .trim_start_matches("ERROR:")
        .trim()
        .chars()
        .take(300)
        .collect::<String>();
    if s.contains("drm") {
        AppError::Drm(last)
    } else if s.contains("available in your country") || s.contains("geo restrict") || s.contains("geo-restrict") || s.contains("not available from your location") {
        AppError::RegionLocked
    } else if s.contains("unsupported url") {
        AppError::Unsupported(last)
    } else if s.contains("video unavailable")
        || s.contains("private video")
        || s.contains("has been removed")
        || s.contains("sign in to confirm your age")
        || s.contains("members-only")
        || s.contains("this track is not available")
        || s.contains("http error 404")
        || s.contains("requested format is not available")
    {
        AppError::Unavailable(last)
    } else if s.contains("timed out")
        || s.contains("timeout")
        || s.contains("temporary failure")
        || s.contains("connection reset")
        || s.contains("connection refused")
        || s.contains("name or service not known")
        || s.contains("network is unreachable")
        || s.contains("getaddrinfo")
        || s.contains("http error 5")
        || s.contains("http error 429")
        || s.contains("unable to download")
    {
        AppError::Network(last)
    } else {
        AppError::YtDlp(last)
    }
}
