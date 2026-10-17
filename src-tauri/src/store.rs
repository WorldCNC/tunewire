//! JSON persistence for settings and the local library. Writes are atomic
//! and durable (unique temp file, fsync, rename), serialized per file, and
//! library.json keeps 7 daily snapshots in backups/ that a corrupt file is
//! restored from. The library is split so frequent writes stay small:
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

pub const HISTORY_MAX: usize = 5000;

/// Tunewire's Discord application (Rich Presence shows its name and icon).
pub const DISCORD_APP_ID: &str = "1560825729984503888";

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
    /// Window background: "art" (playing cover, blurred), "tint" (cover
    /// color), "image" (background_image) or "plain".
    pub background: String,
    /// 0-100: how visible the background is behind the panels.
    pub background_strength: u32,
    /// Picture for background = "image".
    pub background_image: String,
    /// Shuffle mode (the queue's original order is kept to turn it off again).
    pub shuffle: bool,
    /// Show every source's icon on song rows (otherwise only on hover).
    pub show_sources: bool,
    /// Send automatic error reports to the developer (only once
    /// `privacy_asked`: the first-run question was answered).
    pub error_reports: bool,
    /// The first-run privacy question (error reports) was answered.
    pub privacy_asked: bool,
    /// The after-sign-in question (what friends see) was answered.
    pub cloud_privacy_asked: bool,
    /// Scrobbling (credentials are in the keychain).
    pub scrobble_lastfm: bool,
    pub scrobble_listenbrainz: bool,
    /// Display names of the connected accounts.
    pub lastfm_user: String,
    pub listenbrainz_user: String,
    /// The user's own Last.fm API key (its secret is in the keychain); empty = built in.
    pub lastfm_api_key: String,
    /// Development override of the online backend (normally built in).
    pub supabase_url: String,
    pub supabase_anon_key: String,
    /// Followers can see what you're playing.
    pub share_activity: bool,
    /// Interface updates from the beta channel.
    pub beta_updates: bool,
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
            discord_client_id: DISCORD_APP_ID.into(),
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
            background: "art".into(),
            background_strength: 45,
            background_image: String::new(),
            shuffle: false,
            show_sources: false,
            error_reports: false,
            privacy_asked: false,
            cloud_privacy_asked: false,
            scrobble_lastfm: false,
            scrobble_listenbrainz: false,
            lastfm_user: String::new(),
            listenbrainz_user: String::new(),
            lastfm_api_key: String::new(),
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            share_activity: false,
            beta_updates: false,
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
        self.background_strength = self.background_strength.min(100);
        if !matches!(self.background.as_str(), "art" | "tint" | "image" | "plain") {
            self.background = "art".into();
        }
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
    #[serde(default)]
    pub description: String,
    /// Custom cover (data: URL, resized in the UI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    /// Sidebar folder name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Track key -> when it was added (ms).
    #[serde(default)]
    pub added: HashMap<String, u64>,
    /// Linked online playlist (shared link / collaborative / followed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_id: Option<String>,
    /// "owner" | "editor" | "follower"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_role: Option<String>,
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
            // Keep the broken file (never overwritten by a later failure) and
            // fall back to the newest snapshot that still parses.
            eprintln!("tunewire: {} is corrupt ({e}); trying backups", path.display());
            let _ = std::fs::rename(path, path.with_extension(format!("corrupt-{}.json", now_ms())));
            for b in backups_of(path) {
                if let Some(v) = std::fs::read_to_string(&b).ok().and_then(|s| serde_json::from_str(&s).ok()) {
                    eprintln!("tunewire: restored {} from {}", path.display(), b.display());
                    return v;
                }
            }
            T::default()
        }),
        Err(_) => T::default(),
    }
}

/// Snapshots of `path` in <dir>/backups, newest first.
fn backups_of(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else { return Vec::new() };
    let prefix = format!("{stem}-");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir.join("backups"))
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".json"))).collect())
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

const BACKUPS_KEPT: usize = 7;

/// Once a day, keep a copy of `path` before it's replaced (7 days kept).
fn snapshot_daily(path: &Path) {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else { return };
    if !path.exists() {
        return;
    }
    let day = chrono::Local::now().format("%Y%m%d").to_string();
    let target = dir.join("backups").join(format!("{stem}-{day}.json"));
    if target.exists() {
        return;
    }
    // Only snapshot a file that parses: a broken one would push out good copies.
    let Ok(data) = std::fs::read(path) else { return };
    if serde_json::from_slice::<serde::de::IgnoredAny>(&data).is_err() {
        return;
    }
    let _ = std::fs::create_dir_all(dir.join("backups"));
    if write_atomic(&target, &data).is_ok() {
        for old in backups_of(path).into_iter().skip(BACKUPS_KEPT) {
            let _ = std::fs::remove_file(old);
        }
    }
}

/// Crash- and power-safe replace: a unique temp file (concurrent writers
/// never share one), flushed to disk before the rename, then the directory
/// entry flushed too.
pub fn write_atomic(path: &Path, data: &[u8]) -> AppResult<()> {
    use std::io::Write;
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Some(dir) = path.parent() {
            if let Ok(d) = std::fs::File::open(dir) {
                let _ = d.sync_all();
            }
        }
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(res?)
}

/// One lock per file kind: serializing and writing happen in the same order,
/// so an older snapshot can never land on top of a newer one.
static SAVE_SETTINGS: Mutex<()> = Mutex::new(());
static SAVE_LIBRARY: Mutex<()> = Mutex::new(());
static SAVE_HISTORY: Mutex<()> = Mutex::new(());
static SAVE_DOWNLOADS: Mutex<()> = Mutex::new(());

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
        let _w = SAVE_SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
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
        let _w = SAVE_LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
        let lib = self.library.lock().unwrap();
        let data = serde_json::to_vec(&Main { playlists: &lib.playlists, favorites: &lib.favorites, followed: &lib.followed })?;
        drop(lib);
        let path = self.dir.join("library.json");
        snapshot_daily(&path);
        write_atomic(&path, &data)
    }

    fn save_history(&self) -> AppResult<()> {
        let _w = SAVE_HISTORY.lock().unwrap_or_else(|e| e.into_inner());
        let data = serde_json::to_vec(&self.library.lock().unwrap().history)?;
        write_atomic(&self.dir.join("history.json"), &data)
    }

    fn save_downloads(&self) -> AppResult<()> {
        let _w = SAVE_DOWNLOADS.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Like `update_library`, but only writes when `f` says it changed something.
    pub fn update_library_if<T>(&self, f: impl FnOnce(&mut Library) -> AppResult<(T, bool)>) -> AppResult<(T, bool)> {
        let (r, changed) = f(&mut self.library.lock().unwrap())?;
        if changed {
            self.save_library()?;
        }
        Ok((r, changed))
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

#[cfg(test)]
mod save_tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tw-store-{name}-{}", now_ms()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn concurrent_saves_never_corrupt_or_go_back_in_time() {
        let dir = tmp("race");
        let store = std::sync::Arc::new(Store::open(dir.clone()).unwrap());
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let s = store.clone();
                std::thread::spawn(move || {
                    for i in 0..40 {
                        s.update_library(|lib| {
                            lib.followed.push(FollowedArtist { name: format!("{t}-{i}"), url: None, thumbnail: None, seen: vec![], followed_at: 0 });
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let on_disk: Library = load_json(&dir.join("library.json"));
        assert_eq!(on_disk.followed.len(), 320, "the last write must hold every change");
        // No temp files left behind.
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains(".tmp")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_corrupt_library_comes_back_from_the_daily_snapshot() {
        let dir = tmp("corrupt");
        let store = Store::open(dir.clone()).unwrap();
        store.update_library(|lib| { lib.followed.push(FollowedArtist { name: "kept".into(), url: None, thumbnail: None, seen: vec![], followed_at: 0 }); Ok(()) }).unwrap();
        // Second save snapshots the first version.
        store.update_library(|lib| { lib.followed.push(FollowedArtist { name: "newer".into(), url: None, thumbnail: None, seen: vec![], followed_at: 0 }); Ok(()) }).unwrap();
        std::fs::write(dir.join("library.json"), "").unwrap(); // power loss
        let lib: Library = load_json(&dir.join("library.json"));
        assert_eq!(lib.followed.len(), 1);
        assert_eq!(lib.followed[0].name, "kept");
        // The broken file is kept for inspection.
        assert!(std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("corrupt-")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
