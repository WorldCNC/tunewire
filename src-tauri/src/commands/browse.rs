//! Fast pages from YouTube Music's own API (artist, album, playlist, search
//! extras, moods) and importing libraries from other apps.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::imports::{self, ImportRow, SpotifyLibrary};
use crate::models::Track;
use crate::state::{emit_library, AppState};
use crate::store::Playlist;
use crate::ytm::{self, ArtistInfo, Collection, MoodSection, SearchExtras, Shelf};

#[tauri::command]
pub async fn ytm_search_extras(query: String) -> AppResult<SearchExtras> {
    ytm::search_extras(query.trim()).await
}

/// Artist page by id (fast) or by name (one extra search).
#[tauri::command]
pub async fn ytm_artist(name: Option<String>, browse_id: Option<String>) -> AppResult<ArtistInfo> {
    let (id, thumb) = match browse_id.filter(|b| b.starts_with("UC")) {
        Some(b) => (b, None),
        None => {
            let name = name.unwrap_or_default();
            let primary = name.split([',', '&']).next().unwrap_or(&name).split(" feat").next().unwrap_or("").trim().to_string();
            ytm::find_artist(&primary).await?
        }
    };
    let mut info = ytm::artist(&id).await?;
    info.thumbnail = thumb.or(info.banner.clone());
    Ok(info)
}

/// Album or playlist page: a YouTube Music id or URL; other links go through yt-dlp.
#[tauri::command]
pub async fn ytm_collection(id: Option<String>, url: Option<String>) -> AppResult<Collection> {
    let bid = id.or_else(|| url.as_deref().and_then(ytm::browse_id_from_url));
    if let Some(b) = bid.filter(|b| b.starts_with("MPRE") || b.starts_with("VL")) {
        match ytm::collection(&b).await {
            Ok(c) => return Ok(c),
            Err(e) if url.is_none() => return Err(e),
            Err(_) => {}
        }
    }
    let url = url.ok_or_else(|| AppError::Invalid("nothing to open".into()))?;
    crate::ytdlp::validate_url(url.trim())?;
    let pl = crate::artist::album(url.trim()).await?;
    Ok(Collection {
        title: pl.title,
        kind: "Album".into(),
        artist: pl.tracks.first().map(|t| t.artist.clone()).unwrap_or_default(),
        artist_browse: None,
        year: String::new(),
        cover: pl.tracks.first().and_then(|t| t.thumbnail.clone()),
        description: String::new(),
        browse_id: String::new(),
        tracks: pl.tracks,
    })
}

#[tauri::command]
pub async fn ytm_find_album(artist: String, album: String) -> AppResult<Option<String>> {
    let first = artist.split([',', '&']).next().unwrap_or(&artist).trim().to_string();
    ytm::find_album(&first, album.trim()).await
}

#[tauri::command]
pub async fn ytm_moods() -> AppResult<Vec<MoodSection>> {
    ytm::moods().await
}

#[tauri::command]
pub async fn ytm_mood_page(params: String) -> AppResult<Vec<Shelf>> {
    ytm::mood_page(&params).await
}

/// An artist's own SoundCloud uploads (their profile URL).
#[tauri::command]
pub async fn artist_soundcloud(url: String, name: String) -> AppResult<Vec<Track>> {
    crate::artist::soundcloud_tracks(&url, &name).await
}

// Imports --------------------------------------------------------------------

#[tauri::command]
pub async fn spotify_library_login(app: AppHandle) -> AppResult<SpotifyLibrary> {
    imports::spotify_login(&app).await.inspect_err(|e| crate::report::report_app_error("Spotify library login", e))
}

#[derive(Serialize)]
pub struct SpotifyProfile {
    pub name: String,
    pub playlists: Vec<imports::SpotifyPlaylistInfo>,
}

/// Someone's public playlists from their profile link (needs the login above).
#[tauri::command]
pub async fn spotify_profile_playlists(url: String) -> AppResult<SpotifyProfile> {
    let user = url.trim().trim_start_matches("spotify:user:").split("/user/").last().unwrap_or("").split(['?', '/', '#']).next().unwrap_or("").to_string();
    if user.is_empty() || user.contains(':') {
        return Err(AppError::Invalid("paste a Spotify profile link (open.spotify.com/user/…)".into()));
    }
    let (name, playlists) = imports::spotify_profile(&user).await.inspect_err(|e| crate::report::report_app_error("Spotify profile import", e))?;
    Ok(SpotifyProfile { name, playlists })
}

#[derive(Serialize)]
pub struct ImportSummary {
    pub playlists: usize,
    pub songs: usize,
    pub liked: usize,
    pub missing: usize,
    pub first_id: Option<String>,
}

#[derive(Clone, Serialize)]
struct Step {
    done: usize,
    total: usize,
    label: String,
}

/// Import Liked Songs (into Liked) and the chosen playlists.
#[tauri::command]
pub async fn spotify_library_import(app: AppHandle, st: State<'_, AppState>, liked: bool, playlists: Vec<(String, String)>) -> AppResult<ImportSummary> {
    let total = playlists.len() + usize::from(liked);
    let mut step = 0;
    let mut sum = ImportSummary { playlists: 0, songs: 0, liked: 0, missing: 0, first_id: None };
    let progress = |step: usize, label: &str| {
        let _ = app.emit("import-progress", Step { done: step, total, label: label.to_string() });
    };
    if liked {
        progress(step, "Liked Songs");
        let tracks = imports::spotify_liked().await.inspect_err(|e| crate::report::report_app_error("Spotify liked songs import", e))?;
        let added = st.store.update_library(|lib| {
            let mut n = 0;
            for t in tracks {
                if !lib.favorites.iter().any(|f| f.key == t.key) {
                    lib.favorites.push(t);
                    n += 1;
                }
            }
            Ok(n)
        })?;
        sum.liked = added;
        emit_library(&app, "favorites");
        step += 1;
    }
    for (id, name) in playlists {
        progress(step, &name);
        match imports::spotify_playlist(&id).await {
            Ok(tracks) if !tracks.is_empty() => {
                sum.songs += tracks.len();
                let pl = crate::commands::library::create_playlist(app.clone(), st.clone(), name, Some(tracks), Some(format!("https://open.spotify.com/playlist/{id}")))?;
                sum.first_id.get_or_insert(pl.id);
                sum.playlists += 1;
            }
            Ok(_) => {}
            Err(e) => {
                crate::report::report_app_error("Spotify playlist import", &e);
                sum.missing += 1;
            }
        }
        step += 1;
    }
    progress(total, "Done");
    Ok(sum)
}

#[derive(Serialize)]
pub struct ListImport {
    pub playlist: Playlist,
    pub missing: usize,
}

/// A public Apple Music playlist or album link.
#[tauri::command]
pub async fn import_apple_music(app: AppHandle, st: State<'_, AppState>, url: String) -> AppResult<ListImport> {
    let (title, rows) = imports::apple_music(url.trim()).await?;
    import_list(&app, &st, title, rows, Some(url)).await
}

/// Rows from a CSV file (parsed in the UI).
#[tauri::command]
pub async fn import_rows(app: AppHandle, st: State<'_, AppState>, name: String, rows: Vec<ImportRow>) -> AppResult<ListImport> {
    import_list(&app, &st, name, rows.into_iter().take(10_000).collect(), None).await
}

async fn import_list(app: &AppHandle, st: &State<'_, AppState>, name: String, rows: Vec<ImportRow>, source: Option<String>) -> AppResult<ListImport> {
    if rows.is_empty() {
        return Err(AppError::EmptyResults);
    }
    let (tracks, missing) = imports::rows_to_tracks(app, rows, &name).await;
    if tracks.is_empty() {
        return Err(AppError::Unavailable("none of these songs were found on YouTube Music".into()));
    }
    let playlist = crate::commands::library::create_playlist(app.clone(), st.clone(), name, Some(tracks), source)?;
    Ok(ListImport { playlist, missing })
}
