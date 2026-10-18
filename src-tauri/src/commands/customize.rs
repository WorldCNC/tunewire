use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::plugins::{PluginInfo, PluginSource};
use crate::state::AppState;
use crate::themes::{self, Theme, ThemeMeta};

#[tauri::command]
pub fn list_themes(st: State<'_, AppState>) -> Vec<ThemeMeta> {
    st.themes.list()
}

/// Load a theme and make it the active one.
#[tauri::command]
pub fn load_theme(st: State<'_, AppState>, id: String) -> AppResult<Theme> {
    let t = st.themes.get(&id)?;
    st.store.update_settings(|s| s.theme = t.id.clone())?;
    Ok(t)
}

#[tauri::command]
pub fn get_theme(st: State<'_, AppState>, id: String) -> AppResult<Theme> {
    st.themes.get(&id)
}

#[tauri::command]
pub fn save_theme(st: State<'_, AppState>, theme: Theme) -> AppResult<ThemeMeta> {
    st.themes.save(theme)
}

#[tauri::command]
pub fn delete_theme(st: State<'_, AppState>, id: String) -> AppResult<()> {
    st.themes.delete(&id)?;
    if st.store.settings().theme == id {
        st.store.update_settings(|s| s.theme = "midnight".into())?;
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ExportResult {
    json: String,
    path: Option<String>,
}

/// Returns the theme JSON and also writes it to the Downloads folder.
#[tauri::command]
pub fn export_theme(app: AppHandle, st: State<'_, AppState>, id: String) -> AppResult<ExportResult> {
    let json = st.themes.export_json(&id)?;
    let path = app.path().download_dir().ok().map(|d| d.join(format!("{}.tunewire-theme.json", themes::slug(&id)))).and_then(|p| std::fs::write(&p, &json).ok().map(|_| p.display().to_string()));
    Ok(ExportResult { json, path })
}

#[tauri::command]
pub fn import_theme(st: State<'_, AppState>, json: String) -> AppResult<ThemeMeta> {
    st.themes.import_json(&json)
}

#[tauri::command]
pub fn list_plugins(st: State<'_, AppState>) -> Vec<PluginInfo> {
    st.plugins.list(&st.store.settings().enabled_plugins)
}

#[tauri::command]
pub fn enable_plugin(st: State<'_, AppState>, id: String) -> AppResult<()> {
    if !st.plugins.exists(&id) {
        return Err(AppError::NotFound(format!("plugin '{id}'")));
    }
    st.store.update_settings(|s| {
        if !s.enabled_plugins.contains(&id) {
            s.enabled_plugins.push(id);
        }
    })?;
    Ok(())
}

#[tauri::command]
pub fn disable_plugin(st: State<'_, AppState>, id: String) -> AppResult<()> {
    st.store.update_settings(|s| s.enabled_plugins.retain(|p| *p != id))?;
    Ok(())
}

/// Source for an *enabled* plugin only.
#[tauri::command]
pub fn get_plugin_source(st: State<'_, AppState>, id: String) -> AppResult<PluginSource> {
    let settings = st.store.settings();
    if !settings.enabled_plugins.contains(&id) {
        return Err(AppError::Plugin(format!("plugin '{id}' is not enabled")));
    }
    st.plugins.source(&id, settings.plugin_settings.get(&id).cloned().unwrap_or_default())
}

#[tauri::command]
pub fn set_plugin_setting(st: State<'_, AppState>, id: String, key: String, value: Value) -> AppResult<()> {
    if key.len() > 64 || serde_json::to_string(&value).map(|s| s.len()).unwrap_or(0) > 64 * 1024 {
        return Err(AppError::Plugin("setting too large".into()));
    }
    st.store.update_settings(|s| {
        s.plugin_settings.entry(id).or_insert_with(HashMap::new).insert(key, value);
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_dir(st: State<'_, AppState>) -> String {
    st.plugins.dir().display().to_string()
}
