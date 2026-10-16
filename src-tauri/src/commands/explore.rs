//! Charts, followed artists + new releases, search suggestions/history,
//! lyrics translation, cover colors, Spotify profile import, playlist sync.

use std::collections::HashSet;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::commands::discover::fetch_remote;
use crate::error::{AppError, AppResult};
use crate::explore::{self, Charts, Translation};
use crate::spotify::{self, UserPlaylist};
use crate::state::{emit_library, AppState};
use crate::store::{now_ms, FollowedArtist};
use crate::{artist, secrets};

// ---------------------------------------------------------------------------
// Search suggestions / history
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn search_suggestions(query: String) -> AppResult<Vec<String>> {
    explore::suggestions(query).await
}

#[tauri::command]
pub fn search_history(st: State<'_, AppState>) -> Vec<String> {
    st.store.settings().search_history
}

#[tauri::command]
pub fn clear_search_history(st: State<'_, AppState>, query: Option<String>) -> AppResult<()> {
    st.store.update_settings(|s| match &query {
        Some(q) => s.search_history.retain(|h| h != q),
        None => s.search_history.clear(),
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Charts
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct Country {
    code: &'static str,
    name: &'static str,
}

#[tauri::command]
pub fn chart_countries() -> Vec<Country> {
    explore::CHART_COUNTRIES.iter().map(|(code, name)| Country { code, name }).collect()
}

#[tauri::command]
pub async fn get_charts(st: State<'_, AppState>, country: Option<String>) -> AppResult<Charts> {
    let country = match country {
        Some(c) => {
            if st.store.settings().charts_country != c {
                let _ = st.store.update_settings(|s| s.charts_country = c.clone());
            }
            c
        }
        None => st.store.settings().charts_country,
    };
    explore::charts(country).await
}

// ---------------------------------------------------------------------------
// Followed artists + new releases
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_followed(st: State<'_, AppState>) -> Vec<FollowedArtist> {
    st.store.library.lock().unwrap().followed.clone()
}

fn same_name(a: &str, b: &str) -> bool {
    crate::search::norm_text(a) == crate::search::norm_text(b)
}

#[tauri::command]
pub fn follow_artist(app: AppHandle, st: State<'_, AppState>, name: String, url: Option<String>, thumbnail: Option<String>) -> AppResult<bool> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Invalid("artist name".into()));
    }
    let url = url.filter(|u| u.contains("youtube.com/"));
    let following = st.store.update_library(|lib| {
        if let Some(pos) = lib.followed.iter().position(|a| same_name(&a.name, &name)) {
            lib.followed.remove(pos);
            Ok(false)
        } else {
            lib.followed.insert(0, FollowedArtist { name, url, thumbnail, seen: Vec::new(), followed_at: now_ms() });
            Ok(true)
        }
    })?;
    *FEED.lock().unwrap() = None;
    emit_library(&app, "followed");
    Ok(following)
}

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub artist: String,
    pub title: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub new: bool,
}

/// Releases per followed artist, cached for a few hours.
static FEED: Mutex<Option<(std::time::Instant, Vec<Release>)>> = Mutex::new(None);
const PER_ARTIST: usize = 4;

#[tauri::command]
pub async fn new_releases(app: AppHandle, st: State<'_, AppState>, force: Option<bool>) -> AppResult<Vec<Release>> {
    let followed = st.store.library.lock().unwrap().followed.clone();
    if !force.unwrap_or(false) {
        if let Some((t, feed)) = FEED.lock().unwrap().as_ref() {
            if t.elapsed().as_secs() < 3 * 3600 {
                return Ok(mark_new(feed.clone(), &followed));
            }
        }
    }
    // yt-dlp's own semaphore keeps this at 3 concurrent requests.
    let tasks: Vec<_> = followed
        .iter()
        .take(60)
        .map(|a| {
            let (name, url) = (a.name.clone(), a.url.clone());
            tauri::async_runtime::spawn(async move { (name.clone(), artist::albums(&name, url).await) })
        })
        .collect();
    let mut feed = Vec::new();
    for t in tasks {
        let Ok((name, res)) = t.await else { continue };
        if let Ok(cards) = res {
            for c in cards.into_iter().take(PER_ARTIST) {
                feed.push(Release { artist: name.clone(), title: c.title, url: c.url, thumbnail: c.thumbnail, new: false });
            }
        }
    }
    *FEED.lock().unwrap() = Some((std::time::Instant::now(), feed.clone()));
    let feed = mark_new(feed, &followed);
    // First fetch for a newly followed artist: nothing is "new" yet.
    let _ = st.store.update_library(|lib| {
        for a in lib.followed.iter_mut().filter(|a| a.seen.is_empty()) {
            a.seen = feed.iter().filter(|r| same_name(&r.artist, &a.name)).map(|r| r.url.clone()).collect();
        }
        Ok(())
    });
    emit_library(&app, "followed");
    Ok(mark_new(feed, &st.store.library.lock().unwrap().followed))
}

fn mark_new(mut feed: Vec<Release>, followed: &[FollowedArtist]) -> Vec<Release> {
    for r in feed.iter_mut() {
        r.new = followed.iter().find(|a| same_name(&a.name, &r.artist)).map(|a| !a.seen.is_empty() && !a.seen.contains(&r.url)).unwrap_or(false);
    }
    feed
}

#[tauri::command]
pub fn mark_releases_seen(app: AppHandle, st: State<'_, AppState>) -> AppResult<()> {
    let feed = FEED.lock().unwrap().as_ref().map(|(_, f)| f.clone()).unwrap_or_default();
    st.store.update_library(|lib| {
        for a in lib.followed.iter_mut() {
            let mut seen: HashSet<String> = a.seen.drain(..).collect();
            seen.extend(feed.iter().filter(|r| same_name(&r.artist, &a.name)).map(|r| r.url.clone()));
            a.seen = seen.into_iter().collect();
        }
        Ok(())
    })?;
    emit_library(&app, "followed");
    Ok(())
}

// ---------------------------------------------------------------------------
// Lyrics translation, cover color
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn translate_lyrics(lines: Vec<String>, target: Option<String>) -> AppResult<Translation> {
    explore::translate(lines, target.unwrap_or_else(|| "en".into())).await
}

#[tauri::command]
pub async fn cover_color(url: String) -> AppResult<Option<String>> {
    explore::cover_color(url).await
}

// ---------------------------------------------------------------------------
// Spotify profile import + playlist sync
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn spotify_user_playlists(st: State<'_, AppState>, url: String) -> AppResult<Vec<UserPlaylist>> {
    let user = spotify::user_id_from_url(&url).ok_or_else(|| AppError::Invalid("paste a Spotify profile link (open.spotify.com/user/…)".into()))?;
    let id = st.store.settings().spotify_client_id;
    let secret = secrets::get(secrets::SPOTIFY_SECRET).unwrap_or_default();
    spotify::user_playlists(user, id, secret).await
}

/// Re-download an imported playlist from its source link, replacing its songs.
#[tauri::command]
pub async fn sync_playlist(app: AppHandle, st: State<'_, AppState>, id: String) -> AppResult<usize> {
    let src = st
        .store
        .library
        .lock()
        .unwrap()
        .playlists
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::NotFound("playlist".into()))?
        .source_url
        .clone()
        .ok_or_else(|| AppError::Invalid("this playlist wasn't imported from a link".into()))?;
    let remote = fetch_remote(&st, src).await?;
    let n = remote.tracks.len();
    st.store.update_library(|lib| {
        let p = lib.playlists.iter_mut().find(|p| p.id == id).ok_or_else(|| AppError::NotFound("playlist".into()))?;
        p.tracks = remote.tracks;
        p.updated = now_ms();
        Ok(())
    })?;
    emit_library(&app, "playlists");
    Ok(n)
}
