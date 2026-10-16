use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::downloads::{self, Download};
use crate::error::{AppError, AppResult};
use crate::localfiles;
use crate::models::{Platform, Track};
use crate::search;
use crate::state::{emit_library, AppState};
use crate::store;

#[derive(Clone, Serialize)]
struct DownloadEvent<'a> {
    key: &'a str,
    title: &'a str,
    state: &'a str,
    message: Option<String>,
}

/// Queue tracks for offline download (runs in the background, 2 at a time).
#[tauri::command]
pub fn download_tracks(app: AppHandle, st: State<'_, AppState>, tracks: Vec<Track>) -> usize {
    let dir = st.downloads_dir();
    let pending: Vec<Track> = {
        let lib = st.store.library.lock().unwrap();
        tracks.into_iter().filter(|t| downloads::find(&lib.downloads, t).is_none()).collect()
    };
    let n = pending.len();
    for track in pending {
        let (app, dir) = (app.clone(), dir.clone());
        let _ = app.emit("download", DownloadEvent { key: &track.key, title: &track.title, state: "queued", message: None });
        tauri::async_runtime::spawn(async move {
            let st = app.state::<AppState>();
            let _ = app.emit("download", DownloadEvent { key: &track.key, title: &track.title, state: "downloading", message: None });
            let res = async {
                let pref = st.store.settings().platform_order;
                let mut versions = track.versions.clone();
                search::sort_versions(&mut versions, &pref);
                let mut last = AppError::Unavailable("no downloadable source".into());
                for v in versions.iter().filter(|v| !matches!(v.platform, Platform::Spotify | Platform::Local) && !v.preview) {
                    match downloads::fetch(v, &dir).await {
                        Ok(p) => return Ok((v.clone(), p)),
                        Err(e) => last = e,
                    }
                }
                // Spotify-only (or all failed): download the matched upload.
                let v = search::resolve_equivalent(&track.artist, &track.title, track.duration, &pref, &[], &st.cache).await.map_err(|_| last)?;
                let p = downloads::fetch(&v, &dir).await?;
                Ok::<_, AppError>((v, p))
            }
            .await;
            match res {
                Ok((version, path)) => {
                    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    let d = Download { key: track.key.clone(), track: track.clone(), version, path: path.to_string_lossy().into_owned(), size, added: store::now_ms() };
                    let _ = st.store.update_downloads(|dl| {
                        dl.retain(|x| x.key != d.key);
                        dl.insert(0, d);
                        Ok(())
                    });
                    emit_library(&app, "downloads");
                    let _ = app.emit("download", DownloadEvent { key: &track.key, title: &track.title, state: "done", message: None });
                }
                Err(e) => {
                    let _ = app.emit("download", DownloadEvent { key: &track.key, title: &track.title, state: "error", message: Some(e.to_string()) });
                }
            }
        });
    }
    n
}

#[tauri::command]
pub fn list_downloads(st: State<'_, AppState>) -> Vec<Download> {
    st.store.library.lock().unwrap().downloads.iter().filter(|d| Path::new(&d.path).exists()).cloned().collect()
}

#[tauri::command]
pub fn delete_download(app: AppHandle, st: State<'_, AppState>, key: String) -> AppResult<()> {
    st.store.update_downloads(|dl| {
        if let Some(d) = dl.iter().find(|d| d.key == key) {
            let _ = std::fs::remove_file(&d.path);
        }
        dl.retain(|d| d.key != key);
        Ok(())
    })?;
    emit_library(&app, "downloads");
    Ok(())
}

#[tauri::command]
pub fn downloads_dir(st: State<'_, AppState>) -> String {
    st.downloads_dir().display().to_string()
}

async fn rescan(app: &AppHandle, folders: Vec<String>) -> AppResult<usize> {
    let n = tauri::async_runtime::spawn_blocking(move || localfiles::scan(&folders))
        .await
        .map_err(|e| AppError::Storage(e.to_string()))??;
    emit_library(app, "local");
    Ok(n)
}

#[tauri::command]
pub async fn add_local_folder(app: AppHandle, st: State<'_, AppState>, path: String) -> AppResult<usize> {
    if !Path::new(&path).is_dir() {
        return Err(AppError::Invalid(format!("not a folder: {path}")));
    }
    let s = st.store.update_settings(|s| {
        if !s.local_folders.contains(&path) {
            s.local_folders.push(path.clone());
        }
    })?;
    rescan(&app, s.local_folders).await
}

#[tauri::command]
pub async fn remove_local_folder(app: AppHandle, st: State<'_, AppState>, path: String) -> AppResult<usize> {
    let s = st.store.update_settings(|s| s.local_folders.retain(|p| *p != path))?;
    rescan(&app, s.local_folders).await
}

#[tauri::command]
pub async fn rescan_local(app: AppHandle, st: State<'_, AppState>) -> AppResult<usize> {
    rescan(&app, st.store.settings().local_folders).await
}

#[tauri::command]
pub fn list_local() -> Vec<Track> {
    let mut v: Vec<Track> = localfiles::all().into_iter().map(Track::from_version).collect();
    v.sort_by(|a, b| (a.artist.to_lowercase(), a.title.to_lowercase()).cmp(&(b.artist.to_lowercase(), b.title.to_lowercase())));
    v
}
