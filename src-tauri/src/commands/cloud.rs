//! Accounts and online features (see cloud.rs; the page talks to the
//! database and realtime directly with the session token).

use crate::cloud::{self, CloudConfig, CloudSession};
use crate::error::{AppError, AppResult};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError::Network(e.to_string()))?
}

/// Project URL + public anon key, or null when this build has no backend.
#[tauri::command]
pub fn cloud_config() -> Option<CloudConfig> {
    cloud::config()
}

#[tauri::command]
pub async fn cloud_session() -> AppResult<Option<CloudSession>> {
    blocking(cloud::session_blocking).await
}

#[tauri::command]
pub async fn cloud_sign_in(provider: String) -> AppResult<CloudSession> {
    blocking(move || cloud::sign_in_blocking(&provider)).await
}

#[tauri::command]
pub async fn cloud_sign_out() -> AppResult<()> {
    blocking(|| {
        cloud::sign_out_blocking();
        Ok(())
    })
    .await
}

/// An update is about to restart the app (or that failed): see cloud.rs.
/// Pending library and queue saves are written first (a restart skips the
/// exit handler).
#[tauri::command]
pub async fn cloud_prepare_restart(app: tauri::AppHandle, restarting: bool) -> AppResult<()> {
    blocking(move || {
        cloud::freeze_for_restart(restarting);
        if restarting {
            if let Some(st) = tauri::Manager::try_state::<crate::state::AppState>(&app) {
                st.save_all_now();
            }
        }
        Ok(())
    })
    .await
}

/// The user closed a sign-in dialog: stop waiting for the browser.
#[tauri::command]
pub fn cancel_sign_in() {
    crate::loopback::cancel();
}
