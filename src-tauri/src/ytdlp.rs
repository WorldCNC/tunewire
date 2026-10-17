//! The only gateway to remote content: every search, playlist and stream lookup
//! goes through a yt-dlp child process. Concurrency is capped by a semaphore and
//! transient failures are retried with exponential backoff.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::error::{classify_ytdlp_stderr, AppError, AppResult};

/// At most this many yt-dlp processes run at once.
const MAX_CONCURRENT: usize = 3;
const MAX_ATTEMPTS: u32 = 3;

fn semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(MAX_CONCURRENT))
}

fn bin_lock() -> &'static std::sync::RwLock<String> {
    static BIN: OnceLock<std::sync::RwLock<String>> = OnceLock::new();
    BIN.get_or_init(|| std::sync::RwLock::new(std::env::var("TUNEWIRE_YTDLP").unwrap_or_else(|_| "yt-dlp".into())))
}

/// The yt-dlp executable in use (system PATH or the bundled copy).
pub fn binary() -> String {
    bin_lock().read().unwrap().clone()
}

/// Switch executables (e.g. to the bundled copy). `TUNEWIRE_YTDLP` wins.
pub fn set_binary(path: &str) {
    if *bin_lock().read().unwrap() != path {
        tauri::async_runtime::spawn(crate::ytdlp_worker::reset());
    }
    if std::env::var("TUNEWIRE_YTDLP").is_err() {
        *bin_lock().write().unwrap() = path.to_string();
    }
}

pub(crate) fn command() -> Command {
    let mut cmd = Command::new(binary());
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Run yt-dlp once with the given args, returning stdout.
async fn run_once(args: &[String], timeout: Duration) -> AppResult<String> {
    let _permit = semaphore().acquire().await.map_err(|_| AppError::Cancelled)?;
    let child = command().args(args).spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::YtDlpMissing
        } else {
            AppError::YtDlp(e.to_string())
        }
    })?;
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| AppError::Timeout("yt-dlp took too long to respond".into()))?
        .map_err(|e| AppError::YtDlp(e.to_string()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() || (!stdout.trim().is_empty() && stdout.trim_start().starts_with('{')) {
        Ok(stdout)
    } else {
        Err(classify_ytdlp_stderr(&String::from_utf8_lossy(&out.stderr)))
    }
}

/// Run yt-dlp with retry + exponential backoff (500ms, 1s, 2s…) on transient errors.
pub async fn run(args: Vec<String>, timeout: Duration) -> AppResult<String> {
    let mut delay = Duration::from_millis(500);
    let mut attempt = 1;
    loop {
        match run_once(&args, timeout).await {
            Ok(s) => return Ok(s),
            Err(e) if e.is_transient() && attempt < MAX_ATTEMPTS => {
                tokio::time::sleep(delay).await;
                delay *= 2;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

pub async fn run_json(args: Vec<String>, timeout: Duration) -> AppResult<Value> {
    let out = run(args, timeout).await?;
    // With --flat-playlist -J there is exactly one JSON document; with -j there
    // may be several lines, and we only ever want the first.
    let first = out.lines().find(|l| l.trim_start().starts_with('{')).unwrap_or("");
    serde_json::from_str(first).map_err(|e| AppError::YtDlp(format!("unexpected output: {e}")))
}

#[derive(Debug, Clone, Serialize)]
pub struct Health {
    pub ok: bool,
    pub version: Option<String>,
    pub message: String,
    pub path: String,
}

pub async fn health_check() -> Health {
    match run_once(&["--version".into()], Duration::from_secs(15)).await {
        Ok(v) => {
            let version = v.trim().to_string();
            // yt-dlp versions are dates (YYYY.MM.DD); warn when it is very old,
            // because site extractors break constantly.
            let year: i32 = version.split('.').next().and_then(|y| y.parse().ok()).unwrap_or(0);
            let message = if year > 0 && year < 2025 {
                format!("yt-dlp {version} is outdated — run `yt-dlp -U` or update via your package manager.")
            } else {
                format!("yt-dlp {version} ready")
            };
            Health { ok: true, version: Some(version), message, path: binary() }
        }
        Err(e) => Health { ok: false, version: None, message: e.to_string(), path: binary() },
    }
}

/// Flat listing for search queries and playlists (no per-entry extraction).
pub async fn flat(target: &str, limit: Option<usize>) -> AppResult<Value> {
    let mut args = vec![
        "--flat-playlist".into(),
        "-J".into(),
        "--no-warnings".into(),
        "--ignore-errors".into(),
    ];
    if let Some(n) = limit {
        args.push("--playlist-end".into());
        args.push(n.to_string());
    }
    args.push("--".into());
    args.push(target.into());
    run_json(args, Duration::from_secs(60)).await
}

/// Completed metadata for one item (see `batch_meta`).
#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub url: String,
    pub artist: Option<String>,
    pub duration: Option<f64>,
    pub album: Option<String>,
    pub artist_url: Option<String>,
    pub thumbnail: Option<String>,
}

impl Meta {
    pub fn apply(&self, v: &mut crate::models::TrackVersion) {
        if let Some(a) = &self.artist {
            v.artist = crate::search::clean_uploader(a.split(',').next().unwrap_or(a).trim());
        }
        v.duration = self.duration.or(v.duration);
        v.album = self.album.clone().or(v.album.take());
        v.artist_url = self.artist_url.clone().or(v.artist_url.take());
        v.thumbnail = v.thumbnail.take().or(self.thumbnail.clone());
        v.partial = false;
    }
}

/// Full metadata for several items in one yt-dlp process (sequential inside).
pub async fn batch_meta(urls: &[String]) -> AppResult<Vec<Meta>> {
    if urls.is_empty() {
        return Ok(Vec::new());
    }
    let mut args: Vec<String> = vec![
        "--skip-download".into(),
        "--no-warnings".into(),
        "--ignore-errors".into(),
        "--no-playlist".into(),
        "--print".into(),
        "%(webpage_url)s\t%(artist,creator,uploader)s\t%(duration)s\t%(album)s\t%(channel_url,uploader_url)s\t%(thumbnail)s".into(),
        "--".into(),
    ];
    args.extend(urls.iter().cloned());
    let out = run(args, Duration::from_secs(120)).await?;
    let none = |s: &str| (!s.is_empty() && s != "NA").then(|| s.to_string());
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 6 {
                return None;
            }
            // yt-dlp may normalize music.youtube.com URLs; match on the video id.
            let id = f[0].split("v=").nth(1).map(|s| s.split('&').next().unwrap_or(s)).unwrap_or(f[0]);
            let url = urls.iter().find(|u| u.contains(id)).cloned().unwrap_or_else(|| f[0].to_string());
            let thumbnail = none(f[5]).map(|t| t.replace("-original.", "-t500x500."));
            Some(Meta { url, artist: none(f[1]), duration: none(f[2]).and_then(|d| d.parse().ok()), album: none(f[3]), artist_url: none(f[4]), thumbnail })
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamInfo {
    pub url: String,
    pub ext: Option<String>,
    pub acodec: Option<String>,
    pub protocol: Option<String>,
    pub duration: Option<f64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub thumbnail: Option<String>,
    pub filesize: Option<u64>,
    #[serde(skip)]
    pub headers: HashMap<String, String>,
}

/// Format preference: AAC/MP3 first (cheap, precise seeking), then Opus/WebM
/// (decoded via libopus), then anything else.
pub const AUDIO_FORMAT: &str = "bestaudio[acodec^=mp4a]/bestaudio[ext=m4a]/bestaudio[acodec=mp3]/bestaudio[acodec=opus]/bestaudio[acodec=flac]/bestaudio[acodec=vorbis]/bestaudio";

/// Equivalent of `yt-dlp -g -f bestaudio --no-playlist <url>`, but via `-j` so
/// we also receive the HTTP headers, codec and duration of the chosen format.
// ---------------------------------------------------------------------------
// Stream URL cache + prefetch: resolving takes 1-2 s, so likely next songs are
// resolved ahead of time (hover, top result, next in queue) and a click only
// waits for the part that's left. Concurrent requests share one yt-dlp run.
// ---------------------------------------------------------------------------

type StreamCell = Arc<tokio::sync::OnceCell<AppResult<StreamInfo>>>;
static STREAMS: std::sync::Mutex<Option<HashMap<String, (std::time::Instant, StreamCell)>>> = std::sync::Mutex::new(None);
/// Stream URLs stay valid for hours; re-resolve well before that.
const STREAM_TTL: Duration = Duration::from_secs(15 * 60);

fn stream_cell(url: &str) -> StreamCell {
    let mut guard = STREAMS.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, (t, c)| t.elapsed() < STREAM_TTL && !matches!(c.get(), Some(Err(_))));
    if map.len() > 200 {
        map.clear();
    }
    map.entry(url.to_string()).or_insert_with(|| (std::time::Instant::now(), Arc::new(tokio::sync::OnceCell::new()))).1.clone()
}

/// Resolve (or reuse) the playable stream for a page URL.
pub async fn stream_info(url: &str) -> AppResult<StreamInfo> {
    let cell = stream_cell(url);
    cell.get_or_init(|| extract_stream(url)).await.clone()
}

/// Drop a cached stream (it failed to open, e.g. expired).
pub fn forget_stream(url: &str) {
    if let Some(map) = STREAMS.lock().unwrap().as_mut() {
        map.remove(url);
    }
}

/// Resolve in the background if a yt-dlp slot is free (never delays real work).
pub fn prefetch_stream(url: String) {
    if semaphore().available_permits() < 2 {
        return;
    }
    let fresh = STREAMS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&url).map(|(t, c)| t.elapsed() < STREAM_TTL && !matches!(c.get(), Some(Err(_)))))
        .unwrap_or(false);
    if fresh {
        return; // already resolved or in flight
    }
    tauri::async_runtime::spawn(async move {
        if let Ok(info) = stream_info(&url).await {
            let _ = tauri::async_runtime::spawn_blocking(move || crate::stream::warm_connection(&info)).await;
        }
    });
}

/// One throwaway YouTube lookup after startup: the worker then holds a
/// ready instance with open connections, so the first real song is faster.
pub async fn warm_youtube() {
    let _ = extract_stream_with_format("https://www.youtube.com/watch?v=jNQXAC9IVRw", AUDIO_FORMAT).await;
}

pub async fn extract_stream(url: &str) -> AppResult<StreamInfo> {
    // SoundCloud's own API answers in ~0.5 s (yt-dlp: ~1.7 s).
    if url.contains("soundcloud.com/") && crate::search::native_apis() {
        if let Ok(info) = crate::soundcloud::stream(url).await {
            return Ok(info);
        }
    }
    extract_stream_with_format(url, AUDIO_FORMAT).await
}

pub async fn extract_stream_with_format(url: &str, format: &str) -> AppResult<StreamInfo> {
    let args = |fast: bool| {
        let mut a: Vec<String> = vec!["-j".into(), "--no-warnings".into(), "--no-playlist".into(), "-f".into(), format.into()];
        if fast {
            // Audio comes from the player API's direct formats; the HLS/DASH
            // manifests (~0.5 s more per song) only add video variants.
            a.extend(["--extractor-args".into(), "youtube:skip=hls,dash".into()]);
        }
        a.extend(["--".into(), url.into()]);
        a
    };
    let youtube = url.contains("youtube.com/") || url.contains("youtu.be/");
    // Fast path: the long-lived worker (no Python startup, warm connections).
    let opts = serde_json::json!({
        "format": format,
        "noplaylist": true,
        "extractor_args": if youtube { serde_json::json!({ "youtube": { "skip": ["hls", "dash"] } }) } else { serde_json::json!({}) },
    });
    let worker = {
        let _permit = semaphore().acquire().await.map_err(|_| AppError::Cancelled)?;
        crate::ytdlp_worker::extract(&binary(), url, opts).await
    };
    let v = match worker {
        Ok(Some(v)) => v,
        Err(e) if !matches!(&e, AppError::Unavailable(m) | AppError::YtDlp(m) if m.to_lowercase().contains("format")) => return Err(e),
        // Worker unavailable here, or the fast options failed: command line.
        _ => match run_json(args(youtube), Duration::from_secs(60)).await {
        Err(AppError::YtDlp(m) | AppError::Unavailable(m) | AppError::Extraction(m)) if youtube && m.to_lowercase().contains("format") => {
            run_json(args(false), Duration::from_secs(60)).await?
        }
        r => r?,
        },
    };

    if v.get("_type").and_then(Value::as_str) == Some("playlist") {
        return Err(AppError::Invalid("this URL is a playlist, not a single track".into()));
    }
    // Single-format selection puts the stream URL at top level; be defensive in
    // case a merged selection slipped through.
    let fmt = v
        .get("requested_formats")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .unwrap_or(&v);
    let stream_url = fmt
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Extraction("yt-dlp returned no stream URL".into()))?
        .to_string();
    validate_url(&stream_url)?;

    let s = |val: &Value, k: &str| val.get(k).and_then(Value::as_str).map(str::to_string);
    let headers = fmt
        .get("http_headers")
        .or_else(|| v.get("http_headers"))
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    Ok(StreamInfo {
        url: stream_url,
        ext: s(fmt, "ext"),
        acodec: s(fmt, "acodec"),
        protocol: s(fmt, "protocol"),
        duration: v.get("duration").and_then(Value::as_f64),
        title: s(&v, "track").or_else(|| s(&v, "title")),
        artist: s(&v, "artist")
            .or_else(|| s(&v, "creator"))
            .or_else(|| s(&v, "uploader"))
            .map(|a| a.split(',').next().unwrap_or(&a).trim().to_string()),
        thumbnail: s(&v, "thumbnail"),
        filesize: fmt
            .get("filesize")
            .or_else(|| fmt.get("filesize_approx"))
            .and_then(Value::as_u64),
        headers,
    })
}

/// Reject anything that is not an absolute http(s) URL before it reaches the
/// network layer / decoder.
pub fn validate_url(u: &str) -> AppResult<()> {
    let ok = (u.starts_with("https://") || u.starts_with("http://"))
        && u.len() > 10
        && !u.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok {
        Ok(())
    } else {
        Err(AppError::Extraction("invalid stream URL".into()))
    }
}

// ---------------------------------------------------------------------------
// Bundled copy: official standalone build from GitHub, checksum-verified.
// ---------------------------------------------------------------------------

fn release_asset() -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        Some("yt-dlp.exe")
    } else if cfg!(target_os = "macos") {
        Some("yt-dlp_macos")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("yt-dlp_linux_aarch64")
    } else if cfg!(target_os = "linux") {
        Some("yt-dlp_linux")
    } else {
        None
    }
}

pub fn bundled_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" })
}

/// Download (or update to) the latest official yt-dlp build into `dir`.
/// Blocking; verifies the SHA-256 against the release's SHA2-256SUMS.
pub fn install_bundled(dir: &std::path::Path) -> AppResult<std::path::PathBuf> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let asset = release_asset().ok_or_else(|| AppError::Unsupported("no official yt-dlp build for this platform".into()))?;
    let agent = crate::http::agent();
    let base = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
    let sums = agent
        .get(&format!("{base}/SHA2-256SUMS"))
        .config()
        .http_status_as_error(true)
        .build()
        .call()
        .map_err(|e| AppError::Network(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| AppError::Network(e.to_string()))?;
    let expected = sums
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            let (h, name) = (it.next()?, it.next()?);
            (name.trim_start_matches('*') == asset).then(|| h.to_lowercase())
        })
        .ok_or_else(|| AppError::Extraction("checksum for this platform not found in the release".into()))?;
    let mut data = Vec::new();
    agent
        .get(&format!("{base}/{asset}"))
        .config()
        .http_status_as_error(true)
        .timeout_recv_body(Some(Duration::from_secs(600)))
        .build()
        .call()
        .map_err(|e| AppError::Network(e.to_string()))?
        .into_body()
        .into_reader()
        .take(200 * 1024 * 1024)
        .read_to_end(&mut data)
        .map_err(|e| AppError::Network(e.to_string()))?;
    let got = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if got != expected {
        return Err(AppError::Extraction("downloaded yt-dlp failed checksum verification".into()));
    }
    std::fs::create_dir_all(dir)?;
    let path = bundled_path(dir);
    let tmp = path.with_extension("download");
    std::fs::write(&tmp, &data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}
