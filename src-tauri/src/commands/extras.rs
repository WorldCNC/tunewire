//! Backups, scrobbling, the theme/plugin gallery and undo support.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::backup::{self, BackupSummary};
use crate::error::{AppError, AppResult};
use crate::state::{emit_library, AppState};
use crate::store::{HistoryEntry, Playlist};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError::Storage(e.to_string()))?
}

// Backup ---------------------------------------------------------------------

/// Suggested file name: Tunewire-2026-10-17.tunewire-backup
#[tauri::command]
pub fn backup_file_name() -> String {
    format!("Tunewire-{}.{}", chrono::Local::now().format("%Y-%m-%d"), "tunewire-backup")
}

#[tauri::command]
pub async fn export_backup(app: AppHandle, path: String) -> AppResult<BackupSummary> {
    blocking(move || {
        use tauri::Manager;
        let st = app.state::<AppState>();
        backup::export(&st.store, &st.themes, &PathBuf::from(path))
    })
    .await
}

#[tauri::command]
pub async fn inspect_backup(path: String) -> AppResult<BackupSummary> {
    blocking(move || backup::inspect(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn restore_backup(app: AppHandle, path: String) -> AppResult<BackupSummary> {
    let a = app.clone();
    let s = blocking(move || {
        use tauri::Manager;
        let st = a.state::<AppState>();
        backup::restore(&st.store, &st.themes, &PathBuf::from(path))
    })
    .await?;
    for what in ["playlists", "favorites", "history", "followed"] {
        emit_library(&app, what);
    }
    Ok(s)
}

#[derive(Serialize)]
pub struct Snapshot {
    pub path: String,
    pub name: String,
    pub modified: u64,
}

/// Automatic copies: daily library snapshots and pre-restore backups.
#[tauri::command]
pub fn list_snapshots(st: State<'_, AppState>) -> Vec<Snapshot> {
    let mut v: Vec<Snapshot> = std::fs::read_dir(backup::safety_dir(&st.store))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".tunewire-backup"))
                .map(|e| Snapshot {
                    path: e.path().display().to_string(),
                    name: e.file_name().to_string_lossy().into_owned(),
                    modified: e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.modified.cmp(&a.modified));
    v
}

// Undo -------------------------------------------------------------------------

/// Put a deleted playlist back exactly as it was (same id, cloud link, cover).
#[tauri::command]
pub fn restore_playlist(app: AppHandle, st: State<'_, AppState>, playlist: Playlist, index: Option<usize>) -> AppResult<()> {
    st.store.update_library(|lib| {
        if lib.playlists.iter().any(|p| p.id == playlist.id) {
            return Ok(());
        }
        let at = index.unwrap_or(lib.playlists.len()).min(lib.playlists.len());
        lib.playlists.insert(at, playlist);
        Ok(())
    })?;
    emit_library(&app, "playlists");
    Ok(())
}

/// Put cleared history back.
#[tauri::command]
pub fn restore_history(app: AppHandle, st: State<'_, AppState>, entries: Vec<HistoryEntry>) -> AppResult<()> {
    st.store.update_history(|h| {
        let mut merged = entries;
        merged.extend(h.drain(..));
        merged.sort_by(|a, b| b.played_at.cmp(&a.played_at));
        merged.truncate(crate::store::HISTORY_MAX);
        *h = merged;
        Ok(())
    })?;
    emit_library(&app, "history");
    Ok(())
}

// Scrobbling ---------------------------------------------------------------------

#[derive(Serialize)]
pub struct ScrobbleStatus {
    pub lastfm_available: bool,
    pub lastfm_connected: bool,
    pub listenbrainz_connected: bool,
    pub pending: usize,
}

#[tauri::command]
pub fn scrobble_status(st: State<'_, AppState>) -> ScrobbleStatus {
    let s = st.store.settings();
    ScrobbleStatus {
        lastfm_available: crate::scrobble::lastfm_available(&s.lastfm_api_key),
        lastfm_connected: crate::secrets::get(crate::scrobble::LASTFM_SESSION).is_some(),
        listenbrainz_connected: crate::secrets::get(crate::scrobble::LB_TOKEN).is_some(),
        pending: crate::scrobble::pending(),
    }
}

/// Use your own Last.fm API account (key in settings, secret in the keychain).
#[tauri::command]
pub fn set_lastfm_app(st: State<'_, AppState>, key: String, secret: String) -> AppResult<()> {
    crate::secrets::set(crate::scrobble::LASTFM_OWN_SECRET, secret.trim())?;
    let s = st.store.update_settings(|s| s.lastfm_api_key = key.trim().to_string())?;
    crate::commands::system::apply_settings(&st, &s);
    Ok(())
}

/// Opens Last.fm's approval page and waits (up to 3 minutes) for the user.
#[tauri::command]
pub async fn lastfm_connect(st: State<'_, AppState>) -> AppResult<String> {
    let (token, url) = blocking(crate::scrobble::lastfm_begin).await?;
    open::that_detached(&url).map_err(|e| AppError::Unavailable(format!("couldn't open the browser: {e}")))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let t = token.clone();
        match blocking(move || crate::scrobble::lastfm_finish(&t)).await {
            Ok(name) => {
                let s = st.store.update_settings(|s| {
                    s.scrobble_lastfm = true;
                    s.lastfm_user = name.clone();
                })?;
                crate::commands::system::apply_settings(&st, &s);
                return Ok(name);
            }
            // 14: not authorized yet: keep waiting.
            Err(_) if std::time::Instant::now() < deadline => continue,
            Err(e) => return Err(e),
        }
    }
}

#[tauri::command]
pub async fn listenbrainz_connect(st: State<'_, AppState>, token: String) -> AppResult<String> {
    let name = blocking(move || crate::scrobble::listenbrainz_connect(&token)).await?;
    let s = st.store.update_settings(|s| {
        s.scrobble_listenbrainz = true;
        s.listenbrainz_user = name.clone();
    })?;
    crate::commands::system::apply_settings(&st, &s);
    Ok(name)
}

/// "lastfm" | "listenbrainz"
#[tauri::command]
pub fn scrobble_disconnect(st: State<'_, AppState>, service: String) -> AppResult<()> {
    let s = match service.as_str() {
        "lastfm" => {
            crate::secrets::set(crate::scrobble::LASTFM_SESSION, "")?;
            st.store.update_settings(|s| {
                s.scrobble_lastfm = false;
                s.lastfm_user.clear();
            })?
        }
        "listenbrainz" => {
            crate::secrets::set(crate::scrobble::LB_TOKEN, "")?;
            st.store.update_settings(|s| {
                s.scrobble_listenbrainz = false;
                s.listenbrainz_user.clear();
            })?
        }
        _ => return Err(AppError::Invalid("unknown service".into())),
    };
    crate::commands::system::apply_settings(&st, &s);
    Ok(())
}

// Gallery ---------------------------------------------------------------------------

#[tauri::command]
pub async fn gallery_index() -> AppResult<crate::gallery::Gallery> {
    blocking(crate::gallery::index).await
}

#[tauri::command]
pub async fn gallery_install(app: AppHandle, kind: String, id: String) -> AppResult<serde_json::Value> {
    blocking(move || {
        use tauri::Manager;
        let st = app.state::<AppState>();
        match kind.as_str() {
            "theme" => Ok(serde_json::to_value(crate::gallery::install_theme(&st.themes, &id)?)?),
            "plugin" => Ok(serde_json::to_value(crate::gallery::install_plugin(&st.plugins, &id)?)?),
            _ => Err(AppError::Invalid("unknown gallery item".into())),
        }
    })
    .await
}
