//! Home ("For you") mixes, listening stats and Wrapped.

use base64::Engine;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::foryou::{self, ForYou};
use crate::models::Track;
use crate::state::AppState;
use crate::stats::{self, Stats};

fn taste_sources(st: &AppState) -> (Vec<Track>, Vec<Track>) {
    let lib = st.store.library.lock().unwrap();
    let playlists = lib.playlists.iter().flat_map(|p| p.tracks.iter().cloned()).take(3000).collect();
    (lib.favorites.clone(), playlists)
}

/// Cached mixes (instant); `stale` says a refresh is due.
#[tauri::command]
pub fn for_you(st: State<'_, AppState>) -> ForYou {
    let (fav, pl) = taste_sources(&st);
    let plays = stats::plays();
    foryou::current(&st.store.dir, &plays, !(plays.is_empty() && fav.is_empty() && pl.is_empty()))
}

#[tauri::command]
pub async fn refresh_for_you(st: State<'_, AppState>, force: Option<bool>) -> AppResult<ForYou> {
    let (fav, pl) = taste_sources(&st);
    foryou::refresh(&st.store.dir, stats::plays(), fav, pl, force.unwrap_or(false), &st.cache).await
}

/// `range`: "wk" | "4w" | "6m" | "year" | "all".
#[tauri::command]
pub fn listening_stats(range: String, year: Option<i32>) -> Stats {
    stats::summarize(&range, year)
}

/// Save a PNG (the Wrapped share card) to Pictures and return its path.
#[tauri::command]
pub fn save_share_image(app: AppHandle, name: String, data: String) -> AppResult<String> {
    let b64 = data.split_once(',').map(|(_, b)| b).unwrap_or(&data);
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| AppError::Invalid(e.to_string()))?;
    let dir = app.path().picture_dir().or_else(|_| app.path().download_dir()).map_err(|e| AppError::Storage(e.to_string()))?;
    std::fs::create_dir_all(&dir)?;
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == ' ' { c } else { '_' }).collect();
    let path = dir.join(format!("{}.png", safe.trim()));
    std::fs::write(&path, bytes)?;
    Ok(path.to_string_lossy().into_owned())
}
