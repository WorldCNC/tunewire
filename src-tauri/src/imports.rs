//! Bringing a library over from another app:
//!   - Spotify: log in once, then pick Liked Songs and any of your playlists
//!     (private ones too). Songs come in as Spotify tracks and the matcher
//!     finds their YouTube Music recordings in the background.
//!   - Apple Music: a playlist or album link (the page lists its songs).
//!   - CSV files (Exportify, TuneMyMusic, Soundiiz…): parsed in the UI.
//!
//! Songs without a Spotify id are matched to YouTube Music right away.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track, TrackVersion};

// ---------------------------------------------------------------------------
// Spotify (Web API with the user's own login)
// ---------------------------------------------------------------------------

static TOKEN: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct SpotifyPlaylistInfo {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub total: usize,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpotifyLibrary {
    pub user: String,
    pub liked: usize,
    pub playlists: Vec<SpotifyPlaylistInfo>,
}

fn token() -> AppResult<String> {
    TOKEN
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(_, at)| at.elapsed() < Duration::from_secs(50 * 60))
        .map(|(t, _)| t.clone())
        .ok_or_else(|| AppError::Unavailable("log in to Spotify first".into()))
}

fn api_blocking(token: &str, url: &str) -> AppResult<Value> {
    let url = if url.starts_with("http") { url.to_string() } else { format!("https://api.spotify.com/v1{url}") };
    let path = url.split('?').next().unwrap_or("").trim_start_matches("https://api.spotify.com").to_string();
    for attempt in 0..4 {
        // The shared agent hands back HTTP errors as responses (not Err), so
        // the status must be checked here: before, an error reply was read as
        // an empty page ("no playlists") instead of being reported.
        let mut r = crate::http::agent().get(&url).header("Authorization", &format!("Bearer {token}")).header("Accept", "application/json").call().map_err(|e| AppError::Network(e.to_string()))?;
        let status = r.status().as_u16();
        let text = r.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let reason = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("").to_string();
        match status {
            200..=299 => return Ok(v),
            429 if attempt < 3 => {
                std::thread::sleep(Duration::from_secs(2 << attempt));
                continue;
            }
            429 => return Err(AppError::Network("Spotify is rate limiting; try again in a minute".into())),
            401 => {
                *TOKEN.lock().unwrap() = None;
                return Err(AppError::Unavailable("the Spotify login expired; log in again".into()));
            }
            403 => return Err(AppError::Extraction(format!("Spotify refused {path}{}", if reason.is_empty() { String::new() } else { format!(": {reason}") }))),
            404 => return Err(AppError::NotFound(format!("Spotify has nothing at {path}"))),
            c if c >= 500 && attempt < 3 => {
                std::thread::sleep(Duration::from_secs(1 << attempt));
                continue;
            }
            // Not "network": these mean the API changed or refused us (worth a report).
            c => return Err(AppError::Extraction(format!("Spotify API {path} returned HTTP {c}{}", if reason.is_empty() { String::new() } else { format!(" ({reason})") }))),
        }
    }
    Err(AppError::Network("Spotify isn't answering; try again in a minute".into()))
}

async fn api(url: String) -> AppResult<Value> {
    let t = token()?;
    tauri::async_runtime::spawn_blocking(move || api_blocking(&t, &url)).await.map_err(|e| AppError::Network(e.to_string()))?
}

fn image(v: &Value) -> Option<String> {
    let imgs = v.get("images").and_then(Value::as_array)?;
    // Smallest image that's still at least 300 px, else the biggest.
    imgs.iter()
        .filter(|i| i.get("width").and_then(Value::as_u64).unwrap_or(640) >= 300)
        .next_back()
        .or_else(|| imgs.first())
        .and_then(|i| i.get("url").and_then(Value::as_str))
        .map(str::to_string)
}

fn spotify_track(t: &Value) -> Option<Track> {
    let id = t.get("id").and_then(Value::as_str)?;
    if t.get("is_local").and_then(Value::as_bool).unwrap_or(false) || t.get("type").and_then(Value::as_str) == Some("episode") {
        return None;
    }
    let title = t.get("name").and_then(Value::as_str)?.to_string();
    let artist = t.get("artists").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    let album = t.pointer("/album/name").and_then(Value::as_str).map(str::to_string);
    let thumbnail = t.get("album").and_then(image);
    let duration = t.get("duration_ms").and_then(Value::as_f64).map(|ms| ms / 1000.0);
    Some(Track::from_version(TrackVersion {
        platform: Platform::Spotify,
        url: format!("https://open.spotify.com/track/{id}"),
        id: id.to_string(),
        title,
        artist,
        duration,
        thumbnail,
        preview: false,
        partial: false,
        artist_url: None,
        album,
        rank: 0,
        matched_for: None,
        isrc_match: false,
    }))
}

/// Opens Spotify's login page in the browser; then lists what can be imported.
pub async fn spotify_login() -> AppResult<SpotifyLibrary> {
    if token().is_err() {
        let tok = crate::spotify_play::oauth(&["user-library-read", "playlist-read-private", "playlist-read-collaborative"]).await?;
        *TOKEN.lock().unwrap() = Some((tok, std::time::Instant::now()));
    }
    let me = api("/me".into()).await?;
    let user = me.get("display_name").and_then(Value::as_str).or_else(|| me.get("id").and_then(Value::as_str)).unwrap_or("you").to_string();
    // Liked Songs and playlists are listed separately: one refused doesn't hide the other.
    let liked = match api("/me/tracks?limit=1".into()).await {
        Ok(v) => v.get("total").and_then(Value::as_u64).unwrap_or(0) as usize,
        Err(e @ AppError::Unavailable(_)) => return Err(e),
        Err(_) => 0,
    };
    let mut playlists = Vec::new();
    let mut next = Some("/me/playlists?limit=50".to_string());
    while let Some(url) = next.take() {
        let page = api(url).await?;
        for p in page.get("items").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = p.get("id").and_then(Value::as_str) else { continue };
            playlists.push(SpotifyPlaylistInfo {
                id: id.to_string(),
                name: p.get("name").and_then(Value::as_str).unwrap_or("Playlist").to_string(),
                owner: p.pointer("/owner/display_name").and_then(Value::as_str).unwrap_or("").to_string(),
                total: p.pointer("/tracks/total").or_else(|| p.pointer("/items/total")).and_then(Value::as_u64).unwrap_or(0) as usize,
                thumbnail: image(p),
            });
        }
        next = page.get("next").and_then(Value::as_str).map(str::to_string);
        if playlists.len() > 1000 {
            break;
        }
    }
    Ok(SpotifyLibrary { user, liked, playlists })
}

/// All saved (liked) songs, newest first.
pub async fn spotify_liked() -> AppResult<Vec<Track>> {
    let mut out = Vec::new();
    let mut next = Some("/me/tracks?limit=50".to_string());
    while let Some(url) = next.take() {
        let page = api(url).await?;
        out.extend(page.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(|i| i.get("item").filter(|v| v.is_object()).or_else(|| i.get("track"))).filter_map(spotify_track));
        next = page.get("next").and_then(Value::as_str).map(str::to_string);
    }
    Ok(out)
}

/// All pages of a playlist listing at `first` (items carry "track" or "item").
async fn playlist_pages(first: String) -> AppResult<Vec<Track>> {
    let mut out = Vec::new();
    let mut next = Some(first);
    while let Some(url) = next.take() {
        let page = api(url).await?;
        out.extend(page.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(|i| i.get("item").filter(|v| v.is_object()).or_else(|| i.get("track"))).filter_map(spotify_track));
        next = page.get("next").and_then(Value::as_str).map(str::to_string);
    }
    Ok(out)
}

/// One playlist's songs (private ones too). Spotify renamed the endpoint
/// (/tracks → /items), so both are tried; then the web player's listing
/// (public playlists, also covers Spotify's own editorial ones).
pub async fn spotify_playlist(id: &str) -> AppResult<Vec<Track>> {
    let mut last = None;
    for path in [format!("/playlists/{id}/items?limit=100"), format!("/playlists/{id}/tracks?limit=100")] {
        match playlist_pages(path).await {
            Ok(t) if !t.is_empty() => return Ok(t),
            Ok(_) => {}
            Err(e @ AppError::Unavailable(_)) => return Err(e), // login expired
            Err(e) => last = Some(e),
        }
    }
    let pid = id.to_string();
    match tauri::async_runtime::spawn_blocking(move || crate::spotify::fetch_playlist_full(&pid).map(|(_, t)| t)).await {
        Ok(Ok(t)) => Ok(t),
        _ => Err(last.unwrap_or(AppError::EmptyResults)),
    }
}

/// Someone's public playlists (a profile link), with the user's login.
pub async fn spotify_profile(user: &str) -> AppResult<(String, Vec<SpotifyPlaylistInfo>)> {
    let profile = api(format!("/users/{user}")).await.ok();
    let name = profile.as_ref().and_then(|p| p.get("display_name").and_then(Value::as_str)).unwrap_or(user).to_string();
    let mut out = Vec::new();
    let mut next = Some(format!("/users/{user}/playlists?limit=50"));
    let mut failed = None;
    while let Some(url) = next.take() {
        match api(url).await {
            Ok(page) => {
                for p in page.get("items").and_then(Value::as_array).into_iter().flatten() {
                    let Some(id) = p.get("id").and_then(Value::as_str) else { continue };
                    out.push(SpotifyPlaylistInfo {
                        id: id.to_string(),
                        name: p.get("name").and_then(Value::as_str).unwrap_or("Playlist").to_string(),
                        owner: p.pointer("/owner/display_name").and_then(Value::as_str).unwrap_or("").to_string(),
                        total: p.pointer("/tracks/total").or_else(|| p.pointer("/items/total")).and_then(Value::as_u64).unwrap_or(0) as usize,
                        thumbnail: image(p),
                    });
                }
                next = page.get("next").and_then(Value::as_str).map(str::to_string);
            }
            Err(e) => failed = Some(e),
        }
    }
    if out.is_empty() {
        // The web player's profile view (works with a user login).
        let t = token()?;
        let u = user.to_string();
        let v = tauri::async_runtime::spawn_blocking(move || -> AppResult<Value> {
            let url = format!("https://spclient.wg.spotify.com/user-profile-view/v3/profile/{u}/playlists?offset=0&limit=200&market=from_token");
            let mut r = crate::http::agent().get(&url).header("Authorization", &format!("Bearer {t}")).header("Accept", "application/json").header("App-Platform", "WebPlayer").call().map_err(|e| AppError::Extraction(format!("Spotify profile view: {e}")))?;
            Ok(serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| AppError::Network(e.to_string()))?)?)
        })
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;
        match v {
            Ok(v) => {
                for p in v.get("public_playlists").and_then(Value::as_array).into_iter().flatten() {
                    let Some(id) = p.get("uri").and_then(Value::as_str).and_then(|u| u.rsplit(':').next()) else { continue };
                    out.push(SpotifyPlaylistInfo {
                        id: id.to_string(),
                        name: p.get("name").and_then(Value::as_str).unwrap_or("Playlist").to_string(),
                        owner: p.get("owner_name").and_then(Value::as_str).unwrap_or("").to_string(),
                        total: p.get("total_tracks").and_then(Value::as_u64).unwrap_or(0) as usize,
                        thumbnail: p.get("image_url").and_then(Value::as_str).map(str::to_string),
                    });
                }
            }
            Err(e) => return Err(failed.unwrap_or(e)),
        }
    }
    Ok((name, out))
}

// ---------------------------------------------------------------------------
// Apple Music
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ImportRow {
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    /// Seconds.
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub spotify_id: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
}

/// Title and songs of a public Apple Music playlist or album page.
pub async fn apple_music(url: &str) -> AppResult<(String, Vec<ImportRow>)> {
    if !url.contains("music.apple.com/") {
        return Err(AppError::Invalid("not an Apple Music link".into()));
    }
    let u = url.to_string();
    let html = tauri::async_runtime::spawn_blocking(move || -> AppResult<String> {
        let mut r = crate::http::agent().get(&u).header("User-Agent", crate::http::USER_AGENT).call().map_err(|e| AppError::Network(e.to_string()))?;
        r.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))
    })
    .await
    .map_err(|e| AppError::Network(e.to_string()))??;
    parse_apple(&html)
}

fn parse_apple(html: &str) -> AppResult<(String, Vec<ImportRow>)> {
    let start_tag = r#"id="serialized-server-data">"#;
    let start = html.find(start_tag).ok_or_else(|| AppError::Extraction("Apple Music page format changed".into()))? + start_tag.len();
    let end = html[start..].find("</script>").ok_or_else(|| AppError::Extraction("Apple Music page format changed".into()))? + start;
    let v: Value = serde_json::from_str(&html[start..end])?;
    let sections = v.pointer("/data/0/data/sections").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut title = v.pointer("/data/0/data/seoData/pageTitle").and_then(Value::as_str).unwrap_or("Apple Music").split(" - ").next().unwrap_or("Apple Music").to_string();
    let mut rows = Vec::new();
    for s in &sections {
        let id = s.get("id").and_then(Value::as_str).unwrap_or("");
        let items = s.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        if id.contains("detail-header") {
            if let Some(t) = items.first().and_then(|i| i.get("title")).and_then(Value::as_str) {
                title = t.to_string();
            }
        }
        if !id.starts_with("track-list") || id.contains("footer") {
            continue;
        }
        for it in &items {
            let Some(name) = it.get("title").and_then(Value::as_str) else { continue };
            let artist = it.get("subtitleLinks").and_then(Value::as_array).map(|a| a.iter().filter_map(|l| l.get("title").and_then(Value::as_str)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let album = it.pointer("/tertiaryLinks/0/title").and_then(Value::as_str).map(|a| a.trim_end_matches(" - Single").trim_end_matches(" - EP").to_string());
            let duration = it.get("duration").and_then(Value::as_f64).map(|ms| ms / 1000.0);
            let cover = it.pointer("/artwork/dictionary/url").and_then(Value::as_str).map(|u| u.replace("{w}x{h}bb.{f}", "300x300bb.jpg"));
            rows.push(ImportRow { title: name.to_string(), artist, album, duration, spotify_id: None, cover });
        }
    }
    if rows.is_empty() {
        return Err(AppError::EmptyResults);
    }
    Ok((title, rows))
}

// ---------------------------------------------------------------------------
// Rows → playable tracks
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
struct Progress {
    done: usize,
    total: usize,
    label: String,
}

async fn match_row(row: &ImportRow) -> Option<Track> {
    if let Some(id) = row.spotify_id.as_ref().filter(|s| s.len() == 22) {
        return Some(Track::from_version(TrackVersion {
            platform: Platform::Spotify,
            url: format!("https://open.spotify.com/track/{id}"),
            id: id.clone(),
            title: row.title.clone(),
            artist: row.artist.clone(),
            duration: row.duration,
            thumbnail: row.cover.clone(),
            preview: false,
            partial: false,
            artist_url: None,
            album: row.album.clone(),
            rank: 0,
            matched_for: None,
            isrc_match: false,
        }));
    }
    let wanted = TrackVersion {
        platform: Platform::Spotify,
        url: String::new(),
        id: String::new(),
        title: row.title.clone(),
        artist: row.artist.clone(),
        duration: row.duration,
        thumbnail: None,
        preview: false,
        partial: false,
        artist_url: None,
        album: row.album.clone(),
        rank: 0,
        matched_for: None,
        isrc_match: false,
    };
    let first_artist = row.artist.split([',', '&']).next().unwrap_or(&row.artist).trim();
    let found = crate::ytm::search_songs(&format!("{first_artist} {}", crate::search::strip_noise(&row.title))).await.ok()?;
    let pick = found
        .iter()
        .take(6)
        .find(|v| crate::search::same_recording(&wanted, v))
        .or_else(|| if row.duration.is_none() { found.iter().take(3).find(|v| crate::search::same_title(&wanted, v)) } else { None })
        .cloned()?;
    let mut v = pick;
    if v.album.is_none() {
        v.album = row.album.clone();
    }
    Some(Track::from_version(v))
}

/// Match rows (8 at a time), keeping their order. Returns tracks and how many weren't found.
pub async fn rows_to_tracks(app: &AppHandle, rows: Vec<ImportRow>, label: &str) -> (Vec<Track>, usize) {
    let total = rows.len();
    let sem = Arc::new(tokio::sync::Semaphore::new(8));
    let done = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<_> = rows
        .into_iter()
        .map(|row| {
            let (sem, done, app, label) = (sem.clone(), done.clone(), app.clone(), label.to_string());
            tauri::async_runtime::spawn(async move {
                let _p = sem.acquire().await;
                let t = match_row(&row).await;
                let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app.emit("import-progress", Progress { done: d, total, label });
                t
            })
        })
        .collect();
    let mut out = Vec::new();
    let mut missing = 0;
    for t in tasks {
        match t.await {
            Ok(Some(t)) => out.push(t),
            _ => missing += 1,
        }
    }
    (out, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_page_parses() {
        let html = r#"<html><script type="application/json" id="serialized-server-data">{"data":[{"data":{"seoData":{"pageTitle":"Mix - Playlist - Apple Music"},"sections":[
            {"id":"playlist-detail-header-section - x","items":[{"title":"My Mix"}]},
            {"id":"track-list - x","items":[{"title":"Song A","duration":200000,"subtitleLinks":[{"title":"Artist A"},{"title":"B"}],"tertiaryLinks":[{"title":"Album - Single"}],"artwork":{"dictionary":{"url":"https://x/{w}x{h}bb.{f}"}}}]},
            {"id":"track-list-footer - x","items":[{"description":"1 song"}]}]}}]}</script>"#;
        let (title, rows) = parse_apple(html).unwrap();
        assert_eq!(title, "My Mix");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].artist, "Artist A, B");
        assert_eq!(rows[0].album.as_deref(), Some("Album"));
        assert_eq!(rows[0].duration, Some(200.0));
        assert_eq!(rows[0].cover.as_deref(), Some("https://x/300x300bb.jpg"));
    }

    #[test]
    #[ignore]
    fn live_apple_import() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let (title, rows) = rt.block_on(apple_music("https://music.apple.com/us/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb233eb5eb")).unwrap();
        println!("{title}: {} rows", rows.len());
        let t0 = std::time::Instant::now();
        let mut found = 0;
        for r in rows.iter().take(16) {
            if let Some(t) = rt.block_on(match_row(r)) {
                found += 1;
                println!("  {} — {}  =>  {} — {}", r.title, r.artist, t.title, t.artist);
            } else {
                println!("  [none] {} — {}", r.title, r.artist);
            }
        }
        println!("{found}/16 in {:?}", t0.elapsed());
        assert!(found >= 13);
    }
}
