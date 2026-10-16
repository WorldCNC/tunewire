//! spotDL (https://github.com/spotDL/spotify-downloader) as the Spotify
//! counterpart of yt-dlp: real Spotify search plus track/album/playlist
//! metadata via `spotdl save … --save-file`. Audio still comes from a matched
//! YouTube Music / SoundCloud upload (Spotify streams are DRM-protected).

use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, TrackVersion};

fn binary() -> &'static str {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| std::env::var("TUNEWIRE_SPOTDL").unwrap_or_else(|_| "spotdl".into()))
}

fn semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(2))
}

fn command() -> Command {
    let mut cmd = Command::new(binary());
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd
}

/// `Some(version)` when spotDL is installed and runs.
pub async fn version() -> Option<String> {
    let out = tokio::time::timeout(Duration::from_secs(20), command().arg("--version").output()).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn available() -> bool {
    static CACHE: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
    *CACHE.get_or_init(|| async { version().await.is_some() }).await
}

static CREDS: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

/// Use personal Spotify app credentials (much faster than spotDL's shared, rate-limited ones).
pub fn set_credentials(id: &str, secret: &str) {
    *CREDS.lock().unwrap() = (!id.trim().is_empty() && !secret.trim().is_empty()).then(|| (id.trim().to_string(), secret.trim().to_string()));
}

/// Run `spotdl save <query> --save-file tmp` and return the parsed songs.
pub async fn save(query: &str) -> AppResult<Vec<TrackVersion>> {
    let _permit = semaphore().acquire().await.map_err(|_| AppError::Cancelled)?;
    let file = std::env::temp_dir().join(format!("tunewire-{}.spotdl", crate::store::new_id("")));
    let mut cmd = command();
    cmd.args(["save", query, "--save-file"]).arg(&file).args(["--log-level", "ERROR"]);
    if let Some((id, secret)) = CREDS.lock().unwrap().clone() {
        cmd.args(["--client-id", &id, "--client-secret", &secret]);
    }
    let child = cmd
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::Unsupported("spotDL is not installed (pipx install spotdl / uv tool install spotdl)".into())
            } else {
                AppError::Extraction(e.to_string())
            }
        })?;
    let out = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .map_err(|_| AppError::Timeout("spotDL took too long".into()))?
        .map_err(|e| AppError::Extraction(e.to_string()))?;
    let data = std::fs::read_to_string(&file);
    let _ = std::fs::remove_file(&file);
    let data = data.map_err(|_| {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string();
        if err.to_lowercase().contains("rate") || err.contains("429") {
            AppError::Network(format!("Spotify rate limit: {line}"))
        } else {
            AppError::Extraction(format!("spotDL: {line}"))
        }
    })?;
    let songs: Vec<Value> = serde_json::from_str(&data)?;
    Ok(songs.iter().enumerate().filter_map(|(i, s)| parse_song(s, i)).collect())
}

fn parse_song(s: &Value, rank: usize) -> Option<TrackVersion> {
    let title = s.get("name")?.as_str()?.to_string();
    let artist = s
        .get("artists")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
        .filter(|a| !a.is_empty())
        .or_else(|| s.get("artist").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let url = s.get("url").and_then(Value::as_str)?.to_string();
    Some(TrackVersion {
        platform: Platform::Spotify,
        id: s.get("song_id").and_then(Value::as_str).unwrap_or("").to_string(),
        url,
        title,
        artist,
        duration: s.get("duration").and_then(Value::as_f64),
        thumbnail: s.get("cover_url").and_then(Value::as_str).map(str::to_string),
        preview: false,
        partial: false,
        artist_url: None,
        album: s.get("album_name").and_then(Value::as_str).map(str::to_string),
        rank,
    })
}

#[cfg(test)]
mod live_tests {
    #[test]
    #[ignore]
    fn spotdl_search_and_playlist() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let s = rt.block_on(super::save("daft punk one more time")).unwrap();
        println!("search: {:?}", s.iter().map(|v| (&v.title, &v.artist, v.duration)).collect::<Vec<_>>());
        assert!(!s.is_empty());
        let p = rt.block_on(super::save("https://open.spotify.com/album/2noRn2Aes5aoNVsU6iWThc")).unwrap();
        println!("album: {} tracks, first {:?}", p.len(), p.first().map(|v| &v.title));
        assert!(p.len() > 5);
    }
}
