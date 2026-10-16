//! Plugin discovery. Plugins live in `<data>/plugins/<id>/` with a
//! `manifest.json`, a JavaScript entry file and an optional stylesheet.
//!
//! The backend only *serves* plugin source to the frontend; the frontend runs
//! each plugin inside a dedicated Web Worker with no DOM, no Tauri bridge and
//! (unless the `network` permission is granted) no fetch/XHR/WebSocket. Every
//! capability a plugin uses goes through a message-based API that the host
//! checks against the manifest's permissions. No IPC command exists that can
//! spawn processes, so plugins cannot run system commands.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};

pub const PERMISSIONS: &[&str] = &["playback", "queue", "search", "library", "ui", "network", "storage"];
const MAX_JS: u64 = 1024 * 1024;
const MAX_CSS: u64 = 256 * 1024;

const EXAMPLE_MANIFEST: &str = include_str!("../plugins/now-playing-notes/manifest.json");
const EXAMPLE_JS: &str = include_str!("../plugins/now-playing-notes/main.js");
const EXAMPLE_CSS: &str = include_str!("../plugins/now-playing-notes/style.css");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingSpec {
    pub key: String,
    pub label: String,
    #[serde(rename = "type", default = "text")]
    pub kind: String,
    #[serde(default)]
    pub default: Value,
    #[serde(default)]
    pub options: Vec<String>,
}

fn text() -> String {
    "text".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default = "main_js")]
    pub main: String,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub settings: Vec<SettingSpec>,
}

fn main_js() -> String {
    "main.js".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub manifest: Option<Manifest>,
    pub enabled: bool,
    pub error: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginSource {
    pub manifest: Manifest,
    pub js: String,
    pub css: Option<String>,
    pub settings: HashMap<String, Value>,
}

pub struct PluginStore {
    dir: PathBuf,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Only plain file names inside the plugin folder.
fn valid_file(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.') && name.len() < 128
}

impl PluginStore {
    pub fn new(dir: PathBuf) -> PluginStore {
        let _ = std::fs::create_dir_all(&dir);
        PluginStore { dir }
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    /// Copy the bundled example plugin on first run.
    pub fn install_example(&self) -> AppResult<()> {
        let d = self.dir.join("now-playing-notes");
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join("manifest.json"), EXAMPLE_MANIFEST)?;
        std::fs::write(d.join("main.js"), EXAMPLE_JS)?;
        std::fs::write(d.join("style.css"), EXAMPLE_CSS)?;
        Ok(())
    }

    fn read_manifest(&self, id: &str) -> AppResult<Manifest> {
        let path = self.dir.join(id).join("manifest.json");
        let s = std::fs::read_to_string(&path).map_err(|e| AppError::Plugin(format!("{id}: cannot read manifest.json ({e})")))?;
        let mut m: Manifest = serde_json::from_str(&s).map_err(|e| AppError::Plugin(format!("{id}: invalid manifest.json ({e})")))?;
        m.id = id.to_string();
        if !valid_file(&m.main) || m.style.as_deref().map(|s| !valid_file(s)).unwrap_or(false) {
            return Err(AppError::Plugin(format!("{id}: entry files must be plain file names inside the plugin folder")));
        }
        if let Some(bad) = m.permissions.iter().find(|p| !PERMISSIONS.contains(&p.as_str())) {
            return Err(AppError::Plugin(format!("{id}: unknown permission '{bad}'")));
        }
        Ok(m)
    }

    pub fn list(&self, enabled: &[String]) -> Vec<PluginInfo> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out: Vec<PluginInfo> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let id = e.file_name().to_string_lossy().into_owned();
                if !valid_id(&id) {
                    return None;
                }
                let (manifest, error) = match self.read_manifest(&id) {
                    Ok(m) => (Some(m), None),
                    Err(e) => (None, Some(e.to_string())),
                };
                Some(PluginInfo {
                    enabled: enabled.contains(&id) && manifest.is_some(),
                    path: e.path().display().to_string(),
                    id,
                    manifest,
                    error,
                })
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn exists(&self, id: &str) -> bool {
        valid_id(id) && self.dir.join(id).join("manifest.json").exists()
    }

    pub fn source(&self, id: &str, settings: HashMap<String, Value>) -> AppResult<PluginSource> {
        if !valid_id(id) {
            return Err(AppError::Plugin("invalid plugin id".into()));
        }
        let manifest = self.read_manifest(id)?;
        let base = self.dir.join(id);
        let read = |name: &str, max: u64| -> AppResult<String> {
            let p = base.join(name);
            let meta = std::fs::metadata(&p).map_err(|e| AppError::Plugin(format!("{id}/{name}: {e}")))?;
            if meta.len() > max {
                return Err(AppError::Plugin(format!("{id}/{name} is too large")));
            }
            std::fs::read_to_string(&p).map_err(|e| AppError::Plugin(format!("{id}/{name}: {e}")))
        };
        let js = read(&manifest.main, MAX_JS)?;
        let css = match &manifest.style {
            Some(s) => Some(read(s, MAX_CSS)?),
            None => None,
        };
        // Fill in declared defaults for settings the user never touched.
        let mut merged = HashMap::new();
        for spec in &manifest.settings {
            merged.insert(spec.key.clone(), settings.get(&spec.key).cloned().unwrap_or_else(|| spec.default.clone()));
        }
        Ok(PluginSource { manifest, js, css, settings: merged })
    }
}
