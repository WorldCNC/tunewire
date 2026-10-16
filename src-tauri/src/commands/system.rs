use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::{emit_status, AppState};
use crate::store::Settings;
use crate::{localfiles, secrets, spotdl, ytdlp};

/// Push settings into the running subsystems (engine, Discord, spotDL, yt-dlp).
pub fn apply_settings(st: &AppState, s: &Settings) {
    st.audio.set_volume(s.volume);
    st.audio.configure(s.crossfade, s.normalize);
    st.discord.configure(s.discord_rpc, &s.discord_client_id, s.discord_buttons);
    spotdl::set_credentials(&s.spotify_client_id, &secrets::get(secrets::SPOTIFY_SECRET).unwrap_or_default());
    let bundled = ytdlp::bundled_path(&st.data_dir.join("bin"));
    if s.ytdlp_source == "bundled" && bundled.exists() {
        ytdlp::set_binary(&bundled.to_string_lossy());
    } else {
        ytdlp::set_binary("yt-dlp");
    }
}

#[tauri::command]
pub async fn health_check() -> ytdlp::Health {
    ytdlp::health_check().await
}

#[derive(Serialize)]
pub struct ToolStatus {
    spotdl: Option<String>,
    spotify_secret_saved: bool,
    bundled_ytdlp: bool,
    local_tracks: usize,
}

#[tauri::command]
pub async fn tools_status(st: State<'_, AppState>) -> AppResult<ToolStatus> {
    Ok(ToolStatus {
        spotdl: spotdl::version().await,
        spotify_secret_saved: secrets::get(secrets::SPOTIFY_SECRET).is_some(),
        bundled_ytdlp: ytdlp::bundled_path(&st.data_dir.join("bin")).exists(),
        local_tracks: localfiles::count(),
    })
}

#[tauri::command]
pub fn discord_status(st: State<'_, AppState>) -> String {
    st.discord.status()
}

#[tauri::command]
pub fn get_settings(st: State<'_, AppState>) -> Settings {
    st.store.settings()
}

/// Shallow-merge a partial settings object.
#[tauri::command]
pub fn update_settings(st: State<'_, AppState>, patch: Value) -> AppResult<Settings> {
    let current = serde_json::to_value(st.store.settings())?;
    let mut merged = current.as_object().cloned().unwrap_or_default();
    if let Some(p) = patch.as_object() {
        for (k, v) in p {
            // Secrets never travel through settings.json.
            if merged.contains_key(k) && k != "spotify_client_secret" {
                merged.insert(k.clone(), v.clone());
            }
        }
    }
    let new: Settings = serde_json::from_value(Value::Object(merged))?;
    let s = st.store.update_settings(|s| *s = new)?;
    apply_settings(&st, &s);
    Ok(s)
}

/// Store (or clear, with "") the Spotify client secret in the OS keychain.
#[tauri::command]
pub fn set_spotify_secret(st: State<'_, AppState>, secret: String) -> AppResult<()> {
    secrets::set(secrets::SPOTIFY_SECRET, secret.trim())?;
    apply_settings(&st, &st.store.settings());
    Ok(())
}

/// Download/update the bundled yt-dlp and switch to it.
#[tauri::command]
pub async fn ytdlp_install(app: AppHandle, st: State<'_, AppState>) -> AppResult<ytdlp::Health> {
    let dir = st.data_dir.join("bin");
    emit_status(&app, "info", "Downloading yt-dlp…");
    tauri::async_runtime::spawn_blocking(move || ytdlp::install_bundled(&dir))
        .await
        .map_err(|e| AppError::Network(e.to_string()))??;
    let s = st.store.update_settings(|s| s.ytdlp_source = "bundled".into())?;
    apply_settings(&st, &s);
    Ok(ytdlp::health_check().await)
}
