//! Offline downloads through yt-dlp. Files land in the downloads folder and
//! are recorded in the library; playback prefers a downloaded file over the
//! network whenever one exists for a track.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track, TrackVersion};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    /// Logical track key.
    pub key: String,
    pub track: Track,
    /// The version that was downloaded.
    pub version: TrackVersion,
    pub path: String,
    pub size: u64,
    pub added: u64,
}

fn semaphore() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::new(2))
}

/// Downloaded file for a track, if any version of it was saved and still exists.
pub fn find<'a>(downloads: &'a [Download], track: &Track) -> Option<&'a Download> {
    downloads.iter().filter(|d| d.key == track.key || track.versions.iter().any(|v| v.url == d.version.url)).find(|d| Path::new(&d.path).exists())
}

fn safe_name(s: &str) -> String {
    let cleaned: String = s.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    cleaned.trim().trim_matches('.').chars().take(150).collect()
}

/// Download one version into `dir`; returns the file path.
///
/// yt-dlp resolves the stream; the bytes are fetched by our own ranged /
/// HLS downloader (the same one playback uses), which is more reliable than
/// yt-dlp's downloader against YouTube's throttling.
pub async fn fetch(version: &TrackVersion, dir: &Path) -> AppResult<PathBuf> {
    if matches!(version.platform, Platform::Spotify | Platform::Local) {
        return Err(AppError::Invalid("this version can't be downloaded directly".into()));
    }
    std::fs::create_dir_all(dir)?;
    let _permit = semaphore().acquire().await.map_err(|_| AppError::Cancelled)?;
    let info = crate::ytdlp::extract_stream(&version.url).await?;
    let artist = if version.artist.is_empty() { info.artist.clone().unwrap_or_else(|| "Unknown".into()) } else { version.artist.clone() };
    let title = info.title.clone().unwrap_or_else(|| version.title.clone());
    let ext = info.ext.clone().unwrap_or_else(|| "m4a".into());
    let id = if version.id.is_empty() { crate::store::new_id("") } else { version.id.clone() };
    let dest = dir.join(format!("{} - {} [{}].{}", safe_name(&artist), safe_name(&title), safe_name(&id), ext));
    tauri::async_runtime::spawn_blocking(move || {
        let opened = crate::stream::open(&info)?;
        let tmp = opened.shared.wait_complete(Duration::from_secs(30 * 60))?;
        match &opened.hls {
            // HLS segments may sit out of order in the buffer file.
            Some(idx) => idx.write_ordered(&opened.shared, &dest)?,
            None => {
                std::fs::copy(&tmp, &dest)?;
            }
        }
        Ok(dest)
    })
    .await
    .map_err(|e| AppError::Extraction(e.to_string()))?
}

#[cfg(test)]
mod live_tests {
    #[test]
    #[ignore]
    fn download_and_play_locally() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("tunewire-dl-{}", std::process::id()));
        let v = crate::models::TrackVersion {
            platform: crate::models::Platform::Ytmusic,
            url: "https://music.youtube.com/watch?v=fa5IWHDbftI".into(),
            id: "fa5IWHDbftI".into(),
            title: "One More Time".into(),
            artist: "Daft Punk".into(),
            duration: Some(320.0),
            thumbnail: None,
            preview: false,
            partial: false,
            artist_url: None,
            album: None,
            rank: 0,
            matched_for: None,
            isrc_match: false,
        };
        let t0 = std::time::Instant::now();
        let p = rt.block_on(super::fetch(&v, &dir)).expect("download");
        println!("downloaded {} ({} bytes) in {:?}", p.display(), std::fs::metadata(&p).unwrap().len(), t0.elapsed());
        let opened = crate::stream::open_local(&p).unwrap();
        let mut d = crate::decoder::AudioDecoder::new(opened.reader, opened.hint.as_deref()).unwrap();
        d.try_seek(std::time::Duration::from_secs(200)).unwrap();
        assert!(d.by_ref().take(44100).filter(|s| s.abs() > 1e-3).count() > 10000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
