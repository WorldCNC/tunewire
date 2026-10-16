//! Artist and album pages, assembled from yt-dlp listings:
//!   - artist: YouTube Music "artists" search → the artist's channel uploads,
//!     plus SoundCloud tracks when we know the uploader's page;
//!   - albums: YouTube Music "albums" search, each album resolved in parallel;
//!   - album:  a YouTube Music album page (or any playlist URL).

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, RemotePlaylist, Track};
use crate::search::{norm_text, parse_entry};
use crate::ytdlp;

#[derive(Debug, Clone, Serialize)]
pub struct ArtistPage {
    pub name: String,
    pub thumbnail: Option<String>,
    pub url: Option<String>,
    pub songs: Vec<Track>,
    pub soundcloud: Vec<Track>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumCard {
    pub title: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub tracks: usize,
}

fn enc(q: &str) -> String {
    q.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn same_artist(a: &str, b: &str) -> bool {
    let (a, b) = (norm_text(a), norm_text(b));
    !a.is_empty() && (a == b || a.replace(' ', "") == b.replace(' ', "") || b.starts_with(&a) || a.starts_with(&b))
}

fn entries(v: &Value) -> Vec<Value> {
    v.get("entries").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn thumb(v: &Value) -> Option<String> {
    v.get("thumbnails").and_then(Value::as_array).and_then(|a| a.iter().rev().find_map(|t| t.get("url").and_then(Value::as_str))).map(str::to_string)
}

fn tracks_from(v: &Value, platform: Platform) -> Vec<Track> {
    entries(v).iter().enumerate().filter_map(|(i, e)| parse_entry(e, platform, i)).map(Track::from_version).collect()
}

/// Artist page: songs from the YouTube Music channel (+ SoundCloud if known).
pub async fn artist(name: &str, soundcloud_url: Option<String>) -> AppResult<ArtistPage> {
    let search = ytdlp::flat(&format!("https://music.youtube.com/search?q={}#artists", enc(name)), Some(3)).await?;
    let candidates: Vec<String> = entries(&search).iter().filter_map(|e| e.get("url").and_then(Value::as_str).map(str::to_string)).collect();
    let sc = soundcloud_url.filter(|u| u.contains("soundcloud.com"));
    let sc_task = sc.map(|u| tauri::async_runtime::spawn(async move { ytdlp::flat(&format!("{}/tracks", u.trim_end_matches('/')), Some(20)).await }));

    let mut page = ArtistPage { name: name.to_string(), thumbnail: None, url: None, songs: Vec::new(), soundcloud: Vec::new() };
    for url in candidates {
        let Ok(v) = ytdlp::flat(&url, Some(30)).await else { continue };
        let uploader = v.get("uploader").or_else(|| v.get("channel")).and_then(Value::as_str).unwrap_or("");
        if !same_artist(name, &crate::search::clean_uploader(uploader)) {
            continue;
        }
        // Keep the spelling the user clicked when it's the same artist
        // (YouTube writes e.g. "Fred again . ." for "Fred again..").
        let found = crate::search::clean_uploader(uploader);
        page.name = if norm_text(&found).replace(' ', "") == norm_text(name).replace(' ', "") { name.to_string() } else { found };
        page.url = Some(url);
        page.thumbnail = thumb(&v);
        page.songs = tracks_from(&v, Platform::Ytmusic);
        break;
    }
    if let Some(t) = sc_task {
        if let Ok(Ok(v)) = t.await {
            // The user's track listing has no uploader/duration/artwork per
            // entry: they're the artist's own uploads, and the rest is
            // completed in the background (`complete_metadata`).
            page.soundcloud = tracks_from(&v, Platform::Soundcloud);
            for t in page.soundcloud.iter_mut() {
                for v in t.versions.iter_mut() {
                    if v.artist.is_empty() {
                        v.artist = page.name.clone();
                    }
                    if v.duration.is_none() || v.thumbnail.is_none() {
                        v.partial = true;
                    }
                }
                *t = crate::search::build_track(t.versions.clone(), 0.0);
            }
        }
    }
    if page.songs.is_empty() && page.soundcloud.is_empty() {
        return Err(AppError::NotFound(format!("no artist page found for “{name}”")));
    }
    if page.thumbnail.is_none() {
        page.thumbnail = page.songs.iter().chain(&page.soundcloud).find_map(|t| t.thumbnail.clone());
    }
    Ok(page)
}

/// An artist's SoundCloud uploads (metadata gets completed later in the UI).
pub async fn soundcloud_tracks(url: &str, name: &str) -> AppResult<Vec<Track>> {
    if !url.contains("soundcloud.com") {
        return Err(AppError::Invalid("not a SoundCloud page".into()));
    }
    let v = ytdlp::flat(&format!("{}/tracks", url.trim_end_matches('/')), Some(20)).await?;
    let mut tracks = tracks_from(&v, Platform::Soundcloud);
    for t in tracks.iter_mut() {
        for v in t.versions.iter_mut() {
            if v.artist.is_empty() {
                v.artist = name.to_string();
            }
            if v.duration.is_none() || v.thumbnail.is_none() {
                v.partial = true;
            }
        }
        *t = crate::search::build_track(t.versions.clone(), 0.0);
    }
    Ok(tracks)
}

/// Find the artist's official YouTube channel (where the Releases tab lives).
async fn official_channel(name: &str) -> Option<String> {
    let v = ytdlp::flat(&format!("ytsearch8:{name}"), None).await.ok()?;
    let es = entries(&v);
    let pick = |verified_only: bool| {
        es.iter().find_map(|e| {
            let ch = e.get("channel").and_then(Value::as_str).unwrap_or("");
            let verified = e.get("channel_is_verified").and_then(Value::as_bool).unwrap_or(false);
            (same_artist(name, &crate::search::clean_uploader(ch)) && (verified || !verified_only))
                .then(|| e.get("channel_url").and_then(Value::as_str).map(str::to_string))
                .flatten()
        })
    };
    pick(true).or_else(|| pick(false))
}

/// Albums, EPs and singles from the artist's YouTube "Releases" tab.
/// `channel_url` (from a YouTube result) skips the channel lookup.
pub async fn albums(name: &str, channel_url: Option<String>) -> AppResult<Vec<AlbumCard>> {
    let channel = match channel_url.filter(|u| u.contains("youtube.com/")) {
        Some(c) => Some(c),
        None => official_channel(name).await,
    };
    let Some(channel) = channel else { return Ok(Vec::new()) };
    let v = match ytdlp::flat(&format!("{}/releases", channel.trim_end_matches('/')), Some(80)).await {
        Ok(v) => v,
        Err(AppError::YtDlp(_)) | Err(AppError::Unavailable(_)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut seen = std::collections::HashSet::new();
    Ok(entries(&v)
        .iter()
        .filter(|e| seen.insert(norm_text(e.get("title").and_then(Value::as_str).unwrap_or(""))))
        .filter_map(|e| {
            Some(AlbumCard {
                title: e.get("title").and_then(Value::as_str)?.to_string(),
                url: e.get("url").and_then(Value::as_str)?.to_string(),
                thumbnail: thumb(e),
                tracks: 0,
            })
        })
        .collect())
}

/// Cover art of an album/playlist (first track's artwork), cached.
pub async fn album_cover(url: &str) -> AppResult<Option<String>> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(url) {
        return Ok(hit.clone());
    }
    let v = ytdlp::flat(url, Some(1)).await?;
    let cover = thumb(&v).or_else(|| {
        entries(&v).first().and_then(|e| e.get("id").and_then(Value::as_str)).map(|id| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
    });
    cache.lock().unwrap().insert(url.to_string(), cover.clone());
    Ok(cover)
}

/// Album page: tracks of a YouTube Music album (or any playlist URL).
pub async fn album(url: &str) -> AppResult<RemotePlaylist> {
    let v = ytdlp::flat(url, Some(300)).await?;
    let title = v.get("title").and_then(Value::as_str).unwrap_or("Album");
    let title = title.strip_prefix("Album - ").or_else(|| title.strip_prefix("EP - ")).or_else(|| title.strip_prefix("Single - ")).unwrap_or(title).to_string();
    let cover = thumb(&v);
    let platform = Platform::from_url(url).unwrap_or(Platform::Ytmusic);
    let mut tracks = tracks_from(&v, platform);
    for t in tracks.iter_mut() {
        // Album art beats per-video thumbnails on an album page.
        if cover.is_some() {
            t.thumbnail = cover.clone();
        }
        for ver in t.versions.iter_mut() {
            ver.album = Some(title.clone());
        }
    }
    if tracks.is_empty() {
        return Err(AppError::EmptyResults);
    }
    Ok(RemotePlaylist { title, url: url.to_string(), platform: Some(platform), tracks })
}

#[cfg(test)]
mod live_tests {
    #[test]
    #[ignore]
    fn artist_albums_album_lyrics_meta() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let t0 = std::time::Instant::now();
        let a = rt.block_on(super::artist("Daft Punk", Some("https://soundcloud.com/daftpunkofficialmusic".into()))).unwrap();
        println!("artist {} in {:?}: {} songs, {} soundcloud, thumb {}", a.name, t0.elapsed(), a.songs.len(), a.soundcloud.len(), a.thumbnail.is_some());
        assert!(a.songs.len() > 5);
        let t0 = std::time::Instant::now();
        let albums = rt.block_on(super::albums("Daft Punk", None)).unwrap();
        println!("cover {:?}", rt.block_on(super::album_cover(&albums[0].url)).unwrap());
        println!("albums in {:?}: {:?}", t0.elapsed(), albums.iter().map(|x| (&x.title, x.tracks)).collect::<Vec<_>>());
        assert!(!albums.is_empty());
        let al = rt.block_on(super::album(&albums[0].url)).unwrap();
        println!("album {}: {} tracks, first {} — {}", al.title, al.tracks.len(), al.tracks[0].title, al.tracks[0].artist);
        let l = rt.block_on(crate::lyrics::get("Daft Punk".into(), "One More Time".into(), None, Some(320.0))).unwrap();
        let l = l.expect("lyrics");
        println!("lyrics synced={} plain={}", l.synced.is_some(), l.plain.is_some());
        let m = rt.block_on(crate::ytdlp::batch_meta(&["https://music.youtube.com/watch?v=wU26xVT_vBU".to_string()])).unwrap();
        println!("meta {:?}", m);
        assert_eq!(m.len(), 1);
    }
}

