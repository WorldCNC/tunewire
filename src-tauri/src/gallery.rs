//! The theme and plugin gallery: a curated list kept in the repository
//! (gallery/index.json). Every file is downloaded from the repository and
//! checked against the SHA-256 in the index before it's installed, and a
//! plugin's manifest must ask for exactly the permissions the gallery shows.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

pub const GALLERY_BASE: &str = "https://raw.githubusercontent.com/WorldCNC/tunewire/main/gallery/";
const MAX_FILE: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalleryFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalleryTheme {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// A few colors for the preview card: bg, accent, text.
    #[serde(default)]
    pub preview: Vec<String>,
    pub file: GalleryFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalleryPlugin {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    pub files: Vec<GalleryFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gallery {
    #[serde(default)]
    pub themes: Vec<GalleryTheme>,
    #[serde(default)]
    pub plugins: Vec<GalleryPlugin>,
}

fn base() -> String {
    std::env::var("TUNEWIRE_GALLERY").unwrap_or_else(|_| GALLERY_BASE.to_string())
}

fn fetch(path: &str) -> AppResult<Vec<u8>> {
    if path.contains("..") || path.starts_with('/') || path.contains('\\') {
        return Err(AppError::Invalid("bad gallery path".into()));
    }
    let base = base();
    if let Some(dir) = base.strip_prefix("file://") {
        // Local gallery (development and tests).
        return std::fs::read(std::path::Path::new(dir).join(path)).map_err(|e| AppError::NotFound(format!("{path}: {e}")));
    }
    let mut r = crate::http::agent().get(&format!("{base}{path}")).call().map_err(|e| AppError::Network(e.to_string()))?;
    if r.status().as_u16() != 200 {
        return Err(AppError::Network(format!("gallery returned HTTP {} for {path}", r.status().as_u16())));
    }
    r.body_mut().with_config().limit(MAX_FILE).read_to_vec().map_err(|e| AppError::Network(e.to_string()))
}

fn verified(f: &GalleryFile) -> AppResult<Vec<u8>> {
    let data = fetch(&f.path)?;
    let got = format!("{:x}", Sha256::digest(&data));
    if !got.eq_ignore_ascii_case(&f.sha256) {
        return Err(AppError::Invalid(format!("{} doesn't match the gallery's checksum; not installed", f.path)));
    }
    Ok(data)
}

pub fn index() -> AppResult<Gallery> {
    let data = fetch("index.json")?;
    serde_json::from_slice(&data).map_err(|e| AppError::Invalid(format!("gallery index: {e}")))
}

fn find<T: Clone>(list: &[T], id: &str, get: impl Fn(&T) -> &str) -> AppResult<T> {
    list.iter().find(|x| get(x) == id).cloned().ok_or_else(|| AppError::NotFound(format!("{id} isn't in the gallery")))
}

pub fn install_theme(themes: &crate::themes::ThemeStore, id: &str) -> AppResult<crate::themes::ThemeMeta> {
    let g = index()?;
    let t = find(&g.themes, id, |t| &t.id)?;
    let data = verified(&t.file)?;
    themes.import_json(std::str::from_utf8(&data).map_err(|_| AppError::Invalid("theme isn't text".into()))?)
}

pub fn install_plugin(plugins: &crate::plugins::PluginStore, id: &str) -> AppResult<crate::plugins::Manifest> {
    let g = index()?;
    let p = find(&g.plugins, id, |p| &p.id)?;
    let mut files = Vec::new();
    for f in &p.files {
        let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
        files.push((name, verified(f)?));
    }
    // The permissions the user agreed to are the ones in the index.
    let manifest = files.iter().find(|(n, _)| n == "manifest.json").map(|(_, d)| d.clone()).unwrap_or_default();
    let m: serde_json::Value = serde_json::from_slice(&manifest).unwrap_or_default();
    let mut asked: Vec<String> = m.get("permissions").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let mut shown = p.permissions.clone();
    asked.sort();
    shown.sort();
    if asked != shown {
        return Err(AppError::Invalid(format!("{id} asks for different permissions than the gallery shows; not installed")));
    }
    plugins.install_files(id, &files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_gallery(dir: &std::path::Path, perms: &str, tamper: bool) {
        let manifest = format!("{{\"name\":\"Hello\",\"main\":\"main.js\",\"permissions\":{perms}}}");
        let js = "tunewire.log('hi')";
        std::fs::create_dir_all(dir.join("plugins/hello")).unwrap();
        std::fs::create_dir_all(dir.join("themes")).unwrap();
        std::fs::write(dir.join("plugins/hello/manifest.json"), &manifest).unwrap();
        std::fs::write(dir.join("plugins/hello/main.js"), if tamper { "evil()" } else { js }).unwrap();
        let theme = "{\"name\":\"Moss\",\"vars\":{\"--accent\":\"#4caf50\"}}";
        std::fs::write(dir.join("themes/moss.json"), theme).unwrap();
        let sha = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
        let index = serde_json::json!({
            "themes": [{ "id": "moss", "name": "Moss", "file": { "path": "themes/moss.json", "sha256": sha(theme) } }],
            "plugins": [{ "id": "hello", "name": "Hello", "permissions": ["ui"], "files": [
                { "path": "plugins/hello/manifest.json", "sha256": sha(&manifest) },
                { "path": "plugins/hello/main.js", "sha256": sha(js) }] }]
        });
        std::fs::write(dir.join("index.json"), index.to_string()).unwrap();
    }

    // One test (the gallery location is a process-wide env var).
    #[test]
    fn installs_verified_files_and_refuses_tampered_ones() {
        let root = std::env::temp_dir().join(format!("tw-gallery-{}", crate::store::now_ms()));
        let gal = root.join("gallery");
        std::env::set_var("TUNEWIRE_GALLERY", format!("file://{}", gal.display()));
        let plugins = crate::plugins::PluginStore::new(root.join("plugins"));
        let themes = crate::themes::ThemeStore::new(root.join("themes"));

        write_gallery(&gal, "[\"ui\"]", false);
        assert_eq!(index().unwrap().plugins.len(), 1);
        let m = install_plugin(&plugins, "hello").unwrap();
        assert_eq!(m.permissions, vec!["ui".to_string()]);
        assert!(plugins.exists("hello"));
        assert_eq!(install_theme(&themes, "moss").unwrap().name, "Moss");

        // Changed file: checksum fails, nothing replaced.
        write_gallery(&gal, "[\"ui\"]", true);
        assert!(install_plugin(&plugins, "hello").is_err());
        assert_eq!(std::fs::read_to_string(root.join("plugins/hello/main.js")).unwrap(), "tunewire.log('hi')");

        // Manifest asks for more than the gallery shows.
        write_gallery(&gal, "[\"ui\",\"network\"]", false);
        assert!(install_plugin(&plugins, "hello").is_err());

        // The real gallery in the repository: every item installs.
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../gallery");
        std::env::set_var("TUNEWIRE_GALLERY", format!("file://{}", repo.canonicalize().unwrap().display()));
        let g = index().unwrap();
        assert!(!g.themes.is_empty() && !g.plugins.is_empty());
        for t in &g.themes {
            install_theme(&themes, &t.id).unwrap_or_else(|e| panic!("theme {}: {e}", t.id));
        }
        for p in &g.plugins {
            install_plugin(&plugins, &p.id).unwrap_or_else(|e| panic!("plugin {}: {e}", p.id));
        }

        std::env::remove_var("TUNEWIRE_GALLERY");
        let _ = std::fs::remove_dir_all(&root);
    }
}
