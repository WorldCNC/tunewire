//! JSON persistence for settings and the local library. Writes are atomic
//! (temp file + rename). The library is split so frequent writes stay small:
//!   library.json   playlists + favorites
//!   history.json   playback history (written on every track start)
//!   downloads.json offline downloads

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track};

pub const HISTORY_MAX: usize = 500;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn new_id(prefix: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{prefix}{:x}{:x}", now_ms(), N.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub volume: f32,
    pub platform_order: Vec<Platform>,
    pub last_source: String,
    pub search_limit: usize,
    pub theme: String,
    pub enabled_plugins: Vec<String>,
    pub plugin_settings: HashMap<String, HashMap<String, Value>>,
    pub close_to_tray: bool,
    pub include_youtube_in_all: bool,
    /// "auto" (spotDL if installed, else embed page) | "spotdl" | "embed" | "off"
    pub spotify_backend: String,
    pub include_spotify_in_all: bool,
    /// Optional personal Spotify app credentials for spotDL (its shared ones are rate-limited).
    pub spotify_client_id: String,
    /// Legacy: only read once to migrate a plain-text secret into the OS
    /// keychain (see lib.rs). Safe to delete after the next release.
    pub spotify_client_secret: String,
    pub normalize: bool,
    /// Crossfade length in seconds (0 = gapless, no overlap).
    pub crossfade: f32,
    pub lyrics: bool,
    pub local_folders: Vec<String>,
    /// Empty = <app data>/downloads.
    pub downloads_dir: String,
    /// "system" (yt-dlp on PATH) or "bundled" (managed copy in the data dir).
    pub ytdlp_source: String,
    pub auto_update: bool,
    pub discord_rpc: bool,
    pub discord_client_id: String,
    pub discord_buttons: bool,
    pub repeat: String,
    pub example_plugin_installed: bool,
    /// Keep playing related songs when the queue runs out.
    pub autoplay: bool,
    pub eq_enabled: bool,
    /// Gains in dB for EQ_BANDS (31 Hz … 16 kHz).
    pub eq_gains: Vec<f32>,
    /// cpal device id ("" = system default).
    pub output_device: String,
    /// Tint the accent color from the playing song's cover.
    pub cover_accent: bool,
    /// ISO country code for Charts ("ZZ" = global).
    pub charts_country: String,
    /// Recent searches, newest first.
    pub search_history: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            volume: 0.7,
            platform_order: vec![Platform::Ytmusic, Platform::Soundcloud, Platform::Youtube, Platform::Spotify],
            last_source: "all".into(),
            search_limit: 12,
            theme: "midnight".into(),
            enabled_plugins: Vec::new(),
            plugin_settings: HashMap::new(),
            close_to_tray: false,
            include_youtube_in_all: true,
            spotify_backend: "auto".into(),
            include_spotify_in_all: true,
            spotify_client_id: String::new(),
            spotify_client_secret: String::new(),
            normalize: true,
            crossfade: 0.0,
            lyrics: true,
            local_folders: Vec::new(),
            downloads_dir: String::new(),
            ytdlp_source: "system".into(),
            auto_update: true,
            discord_rpc: false,
            discord_client_id: "1428914566795890738".into(),
            discord_buttons: true,
            repeat: "off".into(),
            example_plugin_installed: false,
            autoplay: true,
            eq_enabled: false,
            eq_gains: vec![0.0; 10],
            output_device: String::new(),
            cover_accent: true,
            charts_country: "ZZ".into(),
            search_history: Vec::new(),
        }
    }
}

impl Settings {
    /// Make sure every platform appears exactly once in the preference order.
    pub fn normalize(&mut self) {
        let mut seen = Vec::new();
        for p in self.platform_order.drain(..) {
            if !seen.contains(&p) {
                seen.push(p);
            }
        }
        for p in Platform::ALL {
            if !seen.contains(&p) {
                seen.push(p);
            }
        }
        self.platform_order = seen;
        self.volume = self.volume.clamp(0.0, 1.0);
        self.search_limit = self.search_limit.clamp(3, 40);
        self.eq_gains.resize(10, 0.0);
        for g in self.eq_gains.iter_mut() {
            *g = if g.is_finite() { g.clamp(-12.0, 12.0) } else { 0.0 };
        }
        self.search_history.truncate(30);
        if !matches!(self.spotify_backend.as_str(), "auto" | "spotdl" | "embed" | "off") {
            self.spotify_backend = "auto".into();
        }
        self.crossfade = self.crossfade.clamp(0.0, 12.0);
        if !matches!(self.ytdlp_source.as_str(), "system" | "bundled") {
            self.ytdlp_source = "system".into();
        }
        if !matches!(self.repeat.as_str(), "off" | "all" | "one") {
            self.repeat = "off".into();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub source_url: Option<String>,
    pub created: u64,
    pub updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub track: Track,
    pub platform: Option<Platform>,
    pub played_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowedArtist {
    pub name: String,
    /// YouTube channel of the artist (releases come from its /releases tab).
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// Release URLs already shown to the user (for "new" badges).
    #[serde(default)]
    pub seen: Vec<String>,
    #[serde(default)]
    pub followed_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub playlists: Vec<Playlist>,
    pub favorites: Vec<Track>,
    pub followed: Vec<FollowedArtist>,
    pub history: Vec<HistoryEntry>,
    pub downloads: Vec<crate::downloads::Download>,
}

pub struct Store {
    pub dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub library: Mutex<Library>,
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            // Keep the broken file around instead of silently overwriting it.
            eprintln!("tunewire: {} is corrupt ({e}); starting fresh", path.display());
            let _ = std::fs::rename(path, path.with_extension("json.bak"));
            T::default()
        }),
        Err(_) => T::default(),
    }
}

pub fn write_atomic(path: &Path, data: &[u8]) -> AppResult<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Store {
    pub fn open(dir: PathBuf) -> AppResult<Store> {
        std::fs::create_dir_all(&dir).map_err(|e| AppError::Storage(format!("{}: {e}", dir.display())))?;
        let mut settings: Settings = load_json(&dir.join("settings.json"));
        settings.normalize();
        // library.json may still contain history/downloads from older versions.
        let mut library: Library = load_json(&dir.join("library.json"));
        if dir.join("history.json").exists() {
            library.history = load_json(&dir.join("history.json"));
        }
        if dir.join("downloads.json").exists() {
            library.downloads = load_json(&dir.join("downloads.json"));
        }
        Ok(Store { dir, settings: Mutex::new(settings), library: Mutex::new(library) })
    }

    pub fn save_settings(&self) -> AppResult<()> {
        let data = serde_json::to_vec_pretty(&*self.settings.lock().unwrap())?;
        write_atomic(&self.dir.join("settings.json"), &data)
    }

    /// Playlists + favorites.
    pub fn save_library(&self) -> AppResult<()> {
        #[derive(Serialize)]
        struct Main<'a> {
            playlists: &'a [Playlist],
            favorites: &'a [Track],
            followed: &'a [FollowedArtist],
        }
        let lib = self.library.lock().unwrap();
        let data = serde_json::to_vec(&Main { playlists: &lib.playlists, favorites: &lib.favorites, followed: &lib.followed })?;
        drop(lib);
        write_atomic(&self.dir.join("library.json"), &data)
    }

    fn save_history(&self) -> AppResult<()> {
        let data = serde_json::to_vec(&self.library.lock().unwrap().history)?;
        write_atomic(&self.dir.join("history.json"), &data)
    }

    fn save_downloads(&self) -> AppResult<()> {
        let data = serde_json::to_vec(&self.library.lock().unwrap().downloads)?;
        write_atomic(&self.dir.join("downloads.json"), &data)
    }

    pub fn update_history<T>(&self, f: impl FnOnce(&mut Vec<HistoryEntry>) -> AppResult<T>) -> AppResult<T> {
        let r = f(&mut self.library.lock().unwrap().history)?;
        self.save_history()?;
        Ok(r)
    }

    pub fn update_downloads<T>(&self, f: impl FnOnce(&mut Vec<crate::downloads::Download>) -> AppResult<T>) -> AppResult<T> {
        let r = f(&mut self.library.lock().unwrap().downloads)?;
        self.save_downloads()?;
        Ok(r)
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Change settings in memory only (call `save_settings` later).
    pub fn update_settings_nosave(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.lock().unwrap();
        f(&mut s);
        s.normalize();
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> AppResult<Settings> {
        let s = {
            let mut s = self.settings.lock().unwrap();
            f(&mut s);
            s.normalize();
            s.clone()
        };
        self.save_settings()?;
        Ok(s)
    }

    pub fn update_library<T>(&self, f: impl FnOnce(&mut Library) -> AppResult<T>) -> AppResult<T> {
        let r = f(&mut self.library.lock().unwrap())?;
        self.save_library()?;
        Ok(r)
    }

    pub fn push_history(&self, track: &Track, platform: Option<Platform>) -> AppResult<()> {
        self.update_history(|history| {
            let mut t = track.clone();
            t.score = 0.0;
            // Collapse immediate repeats.
            if history.first().map(|h| h.track.key == t.key).unwrap_or(false) {
                history.remove(0);
            }
            history.insert(0, HistoryEntry { track: t, platform, played_at: now_ms() });
            history.truncate(HISTORY_MAX);
            Ok(())
        })
    }
}
