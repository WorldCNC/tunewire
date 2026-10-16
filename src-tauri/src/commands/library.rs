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

#[tauri::command]
pub fn list_playlists(st: State<'_, AppState>) -> Vec<Playlist> {
    st.store.library.lock().unwrap().playlists.clone()
}

#[tauri::command]
pub fn create_playlist(app: AppHandle, st: State<'_, AppState>, name: String, tracks: Option<Vec<Track>>, source_url: Option<String>) -> AppResult<Playlist> {
    let name = name.trim().chars().take(120).collect::<String>();
    if name.is_empty() {
        return Err(AppError::Invalid("playlist name cannot be empty".into()));
    }
    let now = store::now_ms();
    let pl = Playlist { id: store::new_id("pl"), name, tracks: clean_tracks(tracks.unwrap_or_default()), source_url, created: now, updated: now };
    let out = pl.clone();
    st.store.update_library(|lib| {
        lib.playlists.push(pl);
        Ok(())
    })?;
    emit_library(&app, "playlists");
    Ok(out)
}

fn with_playlist<T>(app: &AppHandle, st: &AppState, id: &str, f: impl FnOnce(&mut Playlist) -> AppResult<T>) -> AppResult<T> {
    let r = st.store.update_library(|lib| {
        let pl = lib.playlists.iter_mut().find(|p| p.id == id).ok_or_else(|| AppError::NotFound("playlist".into()))?;
        let r = f(pl)?;
        pl.updated = store::now_ms();
        Ok(r)
    })?;
    emit_library(app, "playlists");
    Ok(r)
}

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
pub fn add_to_playlist(app: AppHandle, st: State<'_, AppState>, id: String, tracks: Vec<Track>) -> AppResult<usize> {
    with_playlist(&app, &st, &id, |p| {
        let mut added = 0;
        for t in clean_tracks(tracks) {
            if !p.tracks.iter().any(|x| x.key == t.key) {
                p.tracks.push(t);
                added += 1;
            }
        }
        Ok(added)
    })
}

#[tauri::command]
pub fn remove_from_playlist(app: AppHandle, st: State<'_, AppState>, id: String, index: usize) -> AppResult<()> {
    with_playlist(&app, &st, &id, |p| {
        if index >= p.tracks.len() {
            return Err(AppError::Invalid("index out of range".into()));
        }
        p.tracks.remove(index);
        Ok(())
    })
}

#[tauri::command]
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

#[tauri::command]
pub fn list_favorites(st: State<'_, AppState>) -> Vec<Track> {
    st.store.library.lock().unwrap().favorites.clone()
}

/// Returns the new liked state.
#[tauri::command]
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

#[tauri::command]
pub fn list_history(st: State<'_, AppState>, limit: Option<usize>) -> Vec<HistoryEntry> {
    let lib = st.store.library.lock().unwrap();
    lib.history.iter().take(limit.unwrap_or(store::HISTORY_MAX)).cloned().collect()
}

#[tauri::command]
pub fn clear_history(app: AppHandle, st: State<'_, AppState>) -> AppResult<()> {
    st.store.update_library(|lib| {
        lib.history.clear();
        Ok(())
    })?;
    emit_library(&app, "history");
    Ok(())
}
