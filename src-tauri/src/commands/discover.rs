use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::artist::{self, AlbumCard, ArtistPage};
use crate::commands::playback::{play_track_inner, set_queue_and_play};
use crate::error::{AppError, AppResult};
use crate::lyrics::{self, Lyrics};
use crate::models::{Platform, RemotePlaylist, Track};
use crate::search::{self, SearchResponse};
use crate::state::AppState;
use crate::{spotdl, spotify, ytdlp};

#[tauri::command]
pub async fn search_tracks(app: AppHandle, st: State<'_, AppState>, query: String, source: String, limit: Option<usize>) -> AppResult<SearchResponse> {
    let settings = st.store.settings();
    let limit = limit.unwrap_or(settings.search_limit);
    if settings.last_source != source {
        let _ = st.store.update_settings(|s| s.last_source = source.clone());
    }
    // Spotify search works without spotDL now (web search, spotDL as fallback).
    let spotify_ok = settings.spotify_backend != "off";
    let spotify = spotify_ok && (source == "spotify" || settings.include_spotify_in_all);
    let q = query.trim();
    if !q.is_empty() && !q.contains("://") {
        let q = q.to_string();
        let _ = st.store.update_settings(|s| {
            s.search_history.retain(|h| !h.eq_ignore_ascii_case(&q));
            s.search_history.insert(0, q);
            s.search_history.truncate(30);
        });
    }
    let gen = st.search_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let mut res = search::search(query, source, limit, settings.platform_order, st.cache.clone(), settings.include_youtube_in_all, spotify).await?;
    // Spotify-only results: use known matches, and find the rest now so a click
    // only needs the stream (the user is still reading the results).
    res.tracks = crate::matcher::with_matches(res.tracks);
    crate::matcher::prepare(&app, res.tracks.iter().take(12).cloned().collect());

    // YouTube Music entries with guessed artist/duration get completed in the
    // background; the frontend patches its rows when the event arrives.
    let urls = search::needs_completion(&res.tracks);
    if !urls.is_empty() {
        let cache = st.cache.clone();
        tauri::async_runtime::spawn(async move {
            let current = |app: &AppHandle| tauri::Manager::state::<AppState>(app).search_gen.load(std::sync::atomic::Ordering::SeqCst) == gen;
            // Give the user a moment: if they search again, skip this work entirely.
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            if !current(&app) {
                return;
            }
            let (a, b) = urls.split_at(urls.len().div_ceil(2));
            let (a, b) = (a.to_vec(), b.to_vec());
            let ta = tauri::async_runtime::spawn(async move { ytdlp::batch_meta(&a).await });
            let tb = tauri::async_runtime::spawn(async move { ytdlp::batch_meta(&b).await });
            let mut metas = Vec::new();
            for t in [ta, tb] {
                if let Ok(Ok(m)) = t.await {
                    metas.extend(m);
                }
            }
            if !metas.is_empty() {
                search::apply_metadata(&cache, &metas);
                if current(&app) {
                    let _ = app.emit("metadata-updated", metas);
                }
            }
        });
    }
    Ok(res)
}

fn is_spotify(url: &str) -> bool {
    Platform::from_url(url) == Some(Platform::Spotify)
}

#[tauri::command]
pub async fn load_playlist(app: tauri::AppHandle, st: State<'_, AppState>, url: String) -> AppResult<RemotePlaylist> {
    let mut pl = fetch_remote(&st, url).await?;
    // Spotify songs: use known matches now, find the rest in the background.
    pl.tracks = crate::matcher::with_matches(pl.tracks);
    crate::matcher::prepare(&app, pl.tracks.clone());
    Ok(pl)
}

/// Any supported playlist/album/track link → tracks.
pub async fn fetch_remote(st: &AppState, url: String) -> AppResult<RemotePlaylist> {
    let url = url.trim().to_string();
    if is_spotify(&url) {
        return load_spotify(&st.store.settings().spotify_backend, &url).await;
    }
    ytdlp::validate_url(&url).map_err(|_| AppError::Invalid("paste a full http(s) link".into()))?;
    // Native APIs first (full metadata, ~2× faster); yt-dlp below as fallback.
    if url.contains("soundcloud.com/") && url.contains("/sets/") {
        if let Ok((title, tracks)) = crate::soundcloud::playlist(&url).await {
            return Ok(RemotePlaylist { title, url, platform: Some(Platform::Soundcloud), tracks });
        }
    }
    if (url.contains("youtube.com/") || url.contains("youtu.be/")) && url.contains("list=") && !url.contains("list=RD") {
        if let Some(bid) = crate::ytm::browse_id_from_url(&url) {
            if let Ok(c) = crate::ytm::collection(&bid).await {
                let music = url.contains("music.youtube.com");
                let mut tracks = c.tracks;
                if !music {
                    // A YouTube link plays from YouTube (same videos, other URLs).
                    for t in tracks.iter_mut() {
                        for v in t.versions.iter_mut() {
                            v.platform = Platform::Youtube;
                            v.url = format!("https://www.youtube.com/watch?v={}", v.id);
                        }
                    }
                }
                return Ok(RemotePlaylist { title: c.title, url, platform: Some(if music { Platform::Ytmusic } else { Platform::Youtube }), tracks });
            }
        }
    }
    let json = ytdlp::flat(&url, Some(1000)).await?;
    let platform = Platform::from_url(&url);
    let title = json.get("title").and_then(Value::as_str).unwrap_or("Playlist").to_string();
    let tracks: Vec<Track> = match json.get("entries").and_then(Value::as_array) {
        Some(entries) => entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let eurl = e.get("url").or_else(|| e.get("webpage_url")).and_then(Value::as_str).unwrap_or("");
                let p = match (platform, Platform::from_url(eurl)) {
                    (Some(Platform::Ytmusic), _) => Platform::Ytmusic,
                    (_, Some(p)) if p != Platform::Local => p,
                    (Some(p), _) => p,
                    _ => Platform::Youtube,
                };
                search::parse_entry(e, p, i)
            })
            .map(Track::from_version)
            .collect(),
        // A single item: treat it as a one-track playlist.
        None => search::parse_entry(&json, platform.unwrap_or(Platform::Youtube), 0)
            .map(|mut v| {
                if let Some(w) = json.get("webpage_url").and_then(Value::as_str) {
                    v.url = w.to_string();
                }
                vec![Track::from_version(v)]
            })
            .unwrap_or_default(),
    };
    if tracks.is_empty() {
        return Err(AppError::EmptyResults);
    }
    Ok(RemotePlaylist { title, url, platform, tracks })
}

/// Spotify link → tracks, via the embed page and/or spotDL.
async fn load_spotify(backend: &str, url: &str) -> AppResult<RemotePlaylist> {
    let use_spotdl = match backend {
        "off" => return Err(AppError::Unsupported("Spotify support is turned off (Settings → Spotify)".into())),
        "spotdl" => true,
        "auto" => spotdl::available().await,
        _ => false,
    };
    // Auto: the embed page is instant; spotDL is the fallback (it can be slow when rate-limited).
    if backend == "auto" {
        match spotify::load(url).await {
            Ok(pl) => return Ok(pl),
            Err(e) if !use_spotdl => return Err(e),
            Err(_) => {}
        }
    }
    if use_spotdl {
        let versions = spotdl::save(url).await?;
        if versions.is_empty() {
            return Err(AppError::EmptyResults);
        }
        let title = if versions.len() == 1 { versions[0].title.clone() } else { "Spotify".into() };
        let mut pl = RemotePlaylist {
            title,
            url: url.to_string(),
            platform: Some(Platform::Spotify),
            tracks: versions.into_iter().map(|v| search::build_track(vec![v], 0.0)).collect(),
        };
        if pl.tracks.len() > 1 {
            if let Ok(meta) = spotify::load(url).await {
                pl.title = meta.title;
            }
        }
        return Ok(pl);
    }
    spotify::load(url).await
}

#[tauri::command]
pub async fn extract_stream_url(url: String) -> AppResult<ytdlp::StreamInfo> {
    ytdlp::extract_stream(url.trim()).await
}

/// Tracks with any saved Spotify matches filled in (an opened link's view).
#[tauri::command]
pub fn apply_matches(tracks: Vec<Track>) -> Vec<Track> {
    crate::matcher::with_matches(tracks)
}

/// Paste-and-play: a track URL plays now, a playlist URL replaces the queue.
#[tauri::command]
pub async fn play_url(app: AppHandle, st: State<'_, AppState>, url: String) -> AppResult<RemotePlaylist> {
    let pl = load_playlist(app.clone(), st.clone(), url).await?;
    if pl.tracks.len() == 1 {
        play_track_inner(&app, &st, pl.tracks[0].clone(), None);
    } else {
        set_queue_and_play(&app, &st, pl.tracks.clone(), 0);
    }
    Ok(pl)
}

#[tauri::command]
pub async fn get_lyrics(st: State<'_, AppState>, artist: String, title: String, album: Option<String>, duration: Option<f64>) -> AppResult<Option<Lyrics>> {
    if !st.store.settings().lyrics {
        return Ok(None);
    }
    lyrics::get(artist, title, album, duration).await
}

/// Fill in missing artist/duration/artwork for up to 12 items (2 processes).
#[tauri::command]
pub async fn complete_metadata(st: State<'_, AppState>, urls: Vec<String>) -> AppResult<Vec<ytdlp::Meta>> {
    let urls: Vec<String> = urls.into_iter().filter(|u| u.starts_with("https://")).take(12).collect();
    let (a, b) = urls.split_at(urls.len().div_ceil(2));
    let (a, b) = (a.to_vec(), b.to_vec());
    let ta = tauri::async_runtime::spawn(async move { ytdlp::batch_meta(&a).await });
    let tb = tauri::async_runtime::spawn(async move { ytdlp::batch_meta(&b).await });
    let mut metas = Vec::new();
    for t in [ta, tb] {
        if let Ok(Ok(m)) = t.await {
            metas.extend(m);
        }
    }
    search::apply_metadata(&st.cache, &metas);
    Ok(metas)
}

#[tauri::command]
pub async fn artist_page(name: String, soundcloud_url: Option<String>) -> AppResult<ArtistPage> {
    artist::artist(name.trim(), soundcloud_url).await
}

#[tauri::command]
pub async fn artist_albums(name: String, channel_url: Option<String>) -> AppResult<Vec<AlbumCard>> {
    artist::albums(name.trim(), channel_url).await
}

#[tauri::command]
pub async fn album_cover(url: String) -> AppResult<Option<String>> {
    artist::album_cover(url.trim()).await
}

#[tauri::command]
pub async fn album_page(url: String) -> AppResult<RemotePlaylist> {
    ytdlp::validate_url(url.trim())?;
    artist::album(url.trim()).await
}
