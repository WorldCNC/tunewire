use std::collections::HashMap;

use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::models::Track;
use crate::state::{emit_library, AppState};
use crate::store::{self, HistoryEntry, Playlist};

fn clean_tracks(tracks: Vec<Track>) -> Vec<Track> {
    tracks
        .into_iter()
        .map(|mut t| {
            t.score = 0.0;
            t
        })
        .collect()
}

#[tauri::command(async)]
pub fn list_playlists(st: State<'_, AppState>) -> Vec<Playlist> {
    st.store.library.lock().unwrap().playlists.clone()
}

#[tauri::command(async)]
pub fn create_playlist(app: AppHandle, st: State<'_, AppState>, name: String, tracks: Option<Vec<Track>>, source_url: Option<String>) -> AppResult<Playlist> {
    let name = name.trim().chars().take(120).collect::<String>();
    if name.is_empty() {
        return Err(AppError::Invalid("playlist name cannot be empty".into()));
    }
    let now = store::now_ms();
    let tracks = clean_tracks(tracks.unwrap_or_default());
    let added = tracks.iter().map(|t| (t.key.clone(), now)).collect();
    let pl = Playlist {
        id: store::new_id("pl"),
        name,
        tracks,
        source_url,
        created: now,
        updated: now,
        description: String::new(),
        cover: None,
        pinned: false,
        folder: None,
        added,
        cloud_id: None,
        cloud_role: None,
    };
    let out = pl.clone();
    st.store.update_library(|lib| {
        lib.playlists.push(pl);
        Ok(())
    })?;
    emit_library(&app, "playlists");
    Ok(out)
}

/// Change one playlist. A change that changes nothing (e.g. a sync that
/// finds the same songs) isn't saved or announced, so open pages don't
/// redraw for nothing.
fn with_playlist<T>(app: &AppHandle, st: &AppState, id: &str, f: impl FnOnce(&mut Playlist) -> AppResult<T>) -> AppResult<T> {
    let (r, changed) = st.store.update_library_if(|lib| {
        let pl = lib.playlists.iter_mut().find(|p| p.id == id).ok_or_else(|| AppError::NotFound("playlist".into()))?;
        let before = serde_json::to_vec(&*pl).ok();
        let r = f(pl)?;
        let changed = before != serde_json::to_vec(&*pl).ok();
        if changed {
            pl.updated = store::now_ms();
        }
        Ok((r, changed))
    })?;
    if changed {
        emit_library(app, "playlists");
    }
    Ok(r)
}

#[tauri::command(async)]
pub fn rename_playlist(app: AppHandle, st: State<'_, AppState>, id: String, name: String) -> AppResult<()> {
    let name = name.trim().chars().take(120).collect::<String>();
    if name.is_empty() {
        return Err(AppError::Invalid("playlist name cannot be empty".into()));
    }
    with_playlist(&app, &st, &id, |p| {
        p.name = name;
        Ok(())
    })
}

#[tauri::command(async)]
pub fn delete_playlist(app: AppHandle, st: State<'_, AppState>, id: String) -> AppResult<()> {
    st.store.update_library(|lib| {
        let before = lib.playlists.len();
        lib.playlists.retain(|p| p.id != id);
        if lib.playlists.len() == before {
            return Err(AppError::NotFound("playlist".into()));
        }
        Ok(())
    })?;
    emit_library(&app, "playlists");
    Ok(())
}

#[tauri::command(async)]
pub fn add_to_playlist(app: AppHandle, st: State<'_, AppState>, id: String, tracks: Vec<Track>) -> AppResult<usize> {
    with_playlist(&app, &st, &id, |p| {
        let mut added = 0;
        let now = store::now_ms();
        for t in clean_tracks(tracks) {
            if !p.tracks.iter().any(|x| x.key == t.key) {
                p.added.insert(t.key.clone(), now);
                p.tracks.push(t);
                added += 1;
            }
        }
        Ok(added)
    })
}

/// Positions of `keys` in the playlist now. The page sends the keys it saw
/// next to the positions: if the list changed underneath it (a synced edit
/// just arrived), the songs it meant are found by key, never a neighbour.
fn positions(p: &Playlist, indices: Vec<usize>, keys: Option<Vec<String>>) -> Vec<usize> {
    match keys {
        Some(keys) => {
            let want: std::collections::HashSet<&String> = keys.iter().collect();
            p.tracks.iter().enumerate().filter(|(_, t)| want.contains(&t.key)).map(|(i, _)| i).collect()
        }
        None => indices.into_iter().filter(|&i| i < p.tracks.len()).collect(),
    }
}

#[tauri::command(async)]
pub fn remove_from_playlist(app: AppHandle, st: State<'_, AppState>, id: String, index: usize, key: Option<String>) -> AppResult<()> {
    with_playlist(&app, &st, &id, |p| {
        let index = match &key {
            Some(k) => p.tracks.iter().position(|t| &t.key == k).ok_or_else(|| AppError::NotFound("song".into()))?,
            None => index,
        };
        if index >= p.tracks.len() {
            return Err(AppError::Invalid("index out of range".into()));
        }
        p.tracks.remove(index);
        Ok(())
    })
}

/// Remove several songs at once (multi-select).
#[tauri::command(async)]
pub fn remove_many_from_playlist(app: AppHandle, st: State<'_, AppState>, id: String, indices: Vec<usize>, keys: Option<Vec<String>>) -> AppResult<usize> {
    with_playlist(&app, &st, &id, |p| {
        let drop: std::collections::HashSet<usize> = positions(p, indices, keys).into_iter().collect();
        let before = p.tracks.len();
        let mut i = 0;
        p.tracks.retain(|_| {
            let keep = !drop.contains(&i);
            i += 1;
            keep
        });
        let keys: std::collections::HashSet<&String> = p.tracks.iter().map(|t| &t.key).collect();
        p.added.retain(|k, _| keys.contains(k));
        Ok(before - p.tracks.len())
    })
}

/// Move a block of songs (drag and drop of a selection) to `to` (index in the final list).
#[tauri::command(async)]
pub fn move_many_in_playlist(app: AppHandle, st: State<'_, AppState>, id: String, indices: Vec<usize>, to: usize, keys: Option<Vec<String>>) -> AppResult<()> {
    with_playlist(&app, &st, &id, |p| {
        let mut idx = positions(p, indices, keys);
        idx.sort_unstable();
        idx.dedup();
        let moving: Vec<Track> = idx.iter().map(|&i| p.tracks[i].clone()).collect();
        for &i in idx.iter().rev() {
            p.tracks.remove(i);
        }
        let at = to.min(p.tracks.len());
        for (k, t) in moving.into_iter().enumerate() {
            p.tracks.insert(at + k, t);
        }
        Ok(())
    })
}

/// Link a playlist to its online copy (or unlink with nulls).
#[tauri::command(async)]
pub fn set_playlist_cloud(app: AppHandle, st: State<'_, AppState>, id: String, cloud_id: Option<String>, role: Option<String>) -> AppResult<()> {
    with_playlist(&app, &st, &id, |p| {
        p.cloud_id = cloud_id;
        p.cloud_role = role;
        Ok(())
    })
}

/// Error unless the songs' keys are still exactly `expect` (sync applies a
/// remote change only to the list it computed it from).
fn check_keys(tracks: &[Track], expect: Option<&Vec<String>>) -> AppResult<()> {
    match expect {
        Some(keys) if tracks.len() != keys.len() || tracks.iter().zip(keys).any(|(t, k)| &t.key != k) => Err(AppError::Conflict("the songs changed".into())),
        _ => Ok(()),
    }
}

/// Take the songs from the online copy (collaborative edits, followed updates).
#[tauri::command(async)]
pub fn replace_playlist_tracks(app: AppHandle, st: State<'_, AppState>, id: String, tracks: Vec<Track>, added: Option<HashMap<String, u64>>, expect: Option<Vec<String>>) -> AppResult<()> {
    with_playlist(&app, &st, &id, |p| {
        check_keys(&p.tracks, expect.as_ref())?;
        let now = store::now_ms();
        p.tracks = crate::matcher::with_matches(clean_tracks(tracks));
        if let Some(a) = added {
            p.added.extend(a);
        }
        for t in &p.tracks {
            p.added.entry(t.key.clone()).or_insert(now);
        }
        Ok(())
    })
}

#[derive(Debug, serde::Deserialize)]
pub struct PlaylistPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    /// "" removes the custom cover.
    pub cover: Option<String>,
    pub pinned: Option<bool>,
    /// "" takes it out of its folder.
    pub folder: Option<String>,
}

/// Name, description, cover, pin and folder.
#[tauri::command(async)]
pub fn update_playlist_meta(app: AppHandle, st: State<'_, AppState>, id: String, patch: PlaylistPatch) -> AppResult<()> {
    with_playlist(&app, &st, &id, |p| {
        if let Some(n) = patch.name {
            let n = n.trim().chars().take(120).collect::<String>();
            if !n.is_empty() {
                p.name = n;
            }
        }
        if let Some(d) = patch.description {
            p.description = d.trim().chars().take(600).collect();
        }
        if let Some(c) = patch.cover {
            if c.len() > 1_500_000 {
                return Err(AppError::Invalid("cover image is too large".into()));
            }
            p.cover = (!c.is_empty() && c.starts_with("data:image/")).then_some(c);
        }
        if let Some(pin) = patch.pinned {
            p.pinned = pin;
        }
        if let Some(f) = patch.folder {
            let f = f.trim().chars().take(60).collect::<String>();
            p.folder = (!f.is_empty()).then_some(f);
        }
        Ok(())
    })
}

#[tauri::command(async)]
pub fn move_in_playlist(app: AppHandle, st: State<'_, AppState>, id: String, from: usize, to: usize) -> AppResult<()> {
    with_playlist(&app, &st, &id, |p| {
        if from >= p.tracks.len() || to >= p.tracks.len() {
            return Err(AppError::Invalid("index out of range".into()));
        }
        let t = p.tracks.remove(from);
        p.tracks.insert(to, t);
        Ok(())
    })
}

#[tauri::command(async)]
pub fn list_favorites(st: State<'_, AppState>) -> Vec<Track> {
    st.store.library.lock().unwrap().favorites.clone()
}

/// Liked Songs from another device (sync), if they're still `expect` here.
#[tauri::command(async)]
pub fn replace_favorites(app: AppHandle, st: State<'_, AppState>, tracks: Vec<Track>, expect: Option<Vec<String>>) -> AppResult<()> {
    let changed = st.store.update_library_if(|lib| {
        check_keys(&lib.favorites, expect.as_ref())?;
        let tracks = crate::matcher::with_matches(clean_tracks(tracks));
        let changed = tracks.iter().map(|t| &t.key).ne(lib.favorites.iter().map(|t| &t.key));
        if changed {
            lib.favorites = tracks;
        }
        Ok(((), changed))
    })?;
    if changed.1 {
        emit_library(&app, "favorites");
    }
    Ok(())
}

/// Where sync keeps what each playlist looked like at the last sync
/// (sync.json in the data dir; the page owns its contents).
#[tauri::command(async)]
pub fn load_sync_state(st: State<'_, AppState>) -> String {
    std::fs::read_to_string(st.data_dir.join("sync.json")).unwrap_or_default()
}

#[tauri::command]
pub async fn save_sync_state(st: State<'_, AppState>, data: String) -> AppResult<()> {
    if data.len() > 64 * 1024 * 1024 {
        return Err(AppError::Invalid("sync state is too large".into()));
    }
    serde_json::from_str::<serde::de::IgnoredAny>(&data)?;
    let path = st.data_dir.join("sync.json");
    static SAVE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _w = SAVE.lock().await;
    tauri::async_runtime::spawn_blocking(move || store::write_atomic(&path, data.as_bytes())).await.map_err(|e| AppError::Storage(e.to_string()))?
}

/// Returns the new liked state.
#[tauri::command(async)]
pub fn toggle_favorite(app: AppHandle, st: State<'_, AppState>, track: Track) -> AppResult<bool> {
    let liked = st.store.update_library(|lib| {
        if let Some(pos) = lib.favorites.iter().position(|t| t.key == track.key) {
            lib.favorites.remove(pos);
            Ok(false)
        } else {
            let mut t = track;
            t.score = 0.0;
            lib.favorites.insert(0, t);
            Ok(true)
        }
    })?;
    emit_library(&app, "favorites");
    Ok(liked)
}

#[tauri::command(async)]
pub fn list_history(st: State<'_, AppState>, limit: Option<usize>) -> Vec<HistoryEntry> {
    let lib = st.store.library.lock().unwrap();
    lib.history.iter().take(limit.unwrap_or(store::HISTORY_MAX)).cloned().collect()
}

#[tauri::command(async)]
pub fn clear_history(app: AppHandle, st: State<'_, AppState>) -> AppResult<()> {
    st.store.update_history(|h| {
        h.clear();
        Ok(())
    })?;
    emit_library(&app, "history");
    Ok(())
}

/// Songs that fit a playlist (see radio::recommend). `nonce` changes the seeds.
#[tauri::command]
pub async fn recommend_for_playlist(st: State<'_, AppState>, tracks: Vec<Track>, nonce: Option<u64>, count: Option<usize>) -> AppResult<Vec<crate::radio::Recommendation>> {
    let tracks: Vec<Track> = tracks.into_iter().take(2000).collect();
    crate::radio::recommend(tracks, nonce.unwrap_or(1), count.unwrap_or(20).clamp(5, 40), &st.cache).await
}
