//! Backup and restore of everything Tunewire keeps about your music: one
//! `.tunewire-backup` file (JSON) with settings, playlists, Liked Songs,
//! followed artists, history, the listening log and custom themes.
//!
//! Secrets (keychain) and audio files are never included. Restoring first
//! saves the current state as a backup in <data>/backups, so a restore can
//! itself be undone.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::store::{now_ms, write_atomic, FollowedArtist, HistoryEntry, Playlist, Settings, Store};
use crate::models::Track;
use crate::themes::{Theme, ThemeStore};

pub const FORMAT: &str = "tunewire-backup";
const VERSION: u32 = 1;
/// A backup file is JSON text; anything far bigger isn't one of ours.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct Backup {
    pub format: String,
    pub version: u32,
    pub created: u64,
    pub app_version: String,
    pub settings: Value,
    pub playlists: Vec<Playlist>,
    pub favorites: Vec<Track>,
    #[serde(default)]
    pub followed: Vec<FollowedArtist>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    /// plays.jsonl lines (the listening log behind stats and Wrapped).
    #[serde(default)]
    pub plays: Vec<Value>,
    #[serde(default)]
    pub themes: Vec<Theme>,
}

#[derive(Debug, Serialize)]
pub struct BackupSummary {
    pub path: String,
    pub created: u64,
    pub playlists: usize,
    pub songs: usize,
    pub favorites: usize,
    pub plays: usize,
    pub themes: usize,
}

/// Settings fields that only make sense on the machine they came from.
const MACHINE_SETTINGS: [&str; 7] = ["local_folders", "downloads_dir", "output_device", "ytdlp_source", "supabase_url", "supabase_anon_key", "background_image"];

pub fn build(store: &Store, themes: &ThemeStore) -> Backup {
    let lib = store.library.lock().unwrap();
    let mut settings = serde_json::to_value(store.settings()).unwrap_or(Value::Null);
    if let Some(o) = settings.as_object_mut() {
        o.remove("spotify_client_secret");
    }
    let plays = std::fs::read_to_string(store.dir.join("plays.jsonl"))
        .map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default();
    let custom = themes.list().into_iter().filter(|m| !m.builtin).filter_map(|m| themes.get(&m.id).ok()).collect();
    Backup {
        format: FORMAT.into(),
        version: VERSION,
        created: now_ms(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        settings,
        playlists: lib.playlists.clone(),
        favorites: lib.favorites.clone(),
        followed: lib.followed.clone(),
        history: lib.history.clone(),
        plays,
        themes: custom,
    }
}

fn summary(b: &Backup, path: &Path) -> BackupSummary {
    BackupSummary {
        path: path.display().to_string(),
        created: b.created,
        playlists: b.playlists.len(),
        songs: b.playlists.iter().map(|p| p.tracks.len()).sum(),
        favorites: b.favorites.len(),
        plays: b.plays.len(),
        themes: b.themes.len(),
    }
}

pub fn export(store: &Store, themes: &ThemeStore, path: &Path) -> AppResult<BackupSummary> {
    let b = build(store, themes);
    write_atomic(path, &serde_json::to_vec(&b)?)?;
    Ok(summary(&b, path))
}

pub fn read(path: &Path) -> AppResult<Backup> {
    let meta = std::fs::metadata(path).map_err(|e| AppError::Storage(format!("{}: {e}", path.display())))?;
    if meta.len() > MAX_BYTES {
        return Err(AppError::Invalid("that file is too big to be a Tunewire backup".into()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| AppError::Storage(format!("{}: {e}", path.display())))?;
    let b: Backup = serde_json::from_str(&text).map_err(|e| AppError::Invalid(format!("not a Tunewire backup ({e})")))?;
    if b.format != FORMAT {
        return Err(AppError::Invalid("not a Tunewire backup".into()));
    }
    if b.version > VERSION {
        return Err(AppError::Invalid("this backup was made by a newer Tunewire; update first".into()));
    }
    Ok(b)
}

/// What a backup file holds (shown before restoring).
pub fn inspect(path: &Path) -> AppResult<BackupSummary> {
    let b = read(path)?;
    Ok(summary(&b, path))
}

/// Where the automatic pre-restore copy goes.
pub fn safety_dir(store: &Store) -> PathBuf {
    store.dir.join("backups")
}

/// Replace the library with the backup's (the current one is saved first).
pub fn restore(store: &Store, themes: &ThemeStore, path: &Path) -> AppResult<BackupSummary> {
    let b = read(path)?;
    // Undo point.
    let dir = safety_dir(store);
    std::fs::create_dir_all(&dir)?;
    export(store, themes, &dir.join(format!("before-restore-{}.tunewire-backup", now_ms())))?;

    // Settings: keep this machine's own paths/devices.
    let current = serde_json::to_value(store.settings())?;
    let mut merged = current.as_object().cloned().unwrap_or_default();
    if let Some(o) = b.settings.as_object() {
        for (k, v) in o {
            if merged.contains_key(k) && !MACHINE_SETTINGS.contains(&k.as_str()) && k != "spotify_client_secret" {
                merged.insert(k.clone(), v.clone());
            }
        }
    }
    let settings: Settings = serde_json::from_value(Value::Object(merged)).unwrap_or_else(|_| store.settings());
    store.update_settings(|s| *s = settings)?;

    store.update_library(|lib| {
        lib.playlists = b.playlists.clone();
        lib.favorites = b.favorites.clone();
        lib.followed = b.followed.clone();
        Ok(())
    })?;
    store.update_history(|h| {
        *h = b.history.clone();
        h.truncate(crate::store::HISTORY_MAX);
        Ok(())
    })?;
    let mut log = String::new();
    for p in &b.plays {
        log.push_str(&p.to_string());
        log.push('\n');
    }
    write_atomic(&store.dir.join("plays.jsonl"), log.as_bytes())?;
    crate::stats::reload();
    for t in &b.themes {
        let _ = themes.save(t.clone());
    }
    Ok(summary(&b, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(k: &str) -> Track {
        Track { key: k.into(), title: k.into(), artist: "a".into(), duration: Some(100.0), thumbnail: None, versions: vec![], score: 0.0 }
    }

    #[test]
    fn export_then_restore_roundtrips_and_keeps_an_undo_point() {
        let dir = std::env::temp_dir().join(format!("tw-backup-{}", now_ms()));
        let store = Store::open(dir.join("data")).unwrap();
        let themes = ThemeStore::new(dir.join("data/themes"));
        store
            .update_library(|lib| {
                lib.playlists.push(Playlist { id: "p1".into(), name: "Road trip".into(), tracks: vec![track("x"), track("y")], source_url: None, created: 1, updated: 1, description: String::new(), cover: None, pinned: false, folder: None, added: Default::default(), cloud_id: None, cloud_role: None });
                lib.favorites.push(track("z"));
                Ok(())
            })
            .unwrap();
        store.update_settings(|s| { s.crossfade = 4.0; s.local_folders = vec!["/music".into()]; }).unwrap();
        std::fs::write(store.dir.join("plays.jsonl"), "{\"t\":1,\"secs\":40.0,\"platform\":null,\"track\":{\"key\":\"x\",\"title\":\"x\",\"versions\":[]}}\n").unwrap();

        let file = dir.join("my.tunewire-backup");
        let s = export(&store, &themes, &file).unwrap();
        assert_eq!((s.playlists, s.songs, s.favorites, s.plays), (1, 2, 1, 1));

        // Change everything, then restore.
        store.update_library(|lib| { lib.playlists.clear(); lib.favorites.clear(); Ok(()) }).unwrap();
        store.update_settings(|s| { s.crossfade = 0.0; s.local_folders = vec!["/other".into()]; }).unwrap();
        std::fs::write(store.dir.join("plays.jsonl"), "").unwrap();
        restore(&store, &themes, &file).unwrap();

        let lib = store.library.lock().unwrap();
        assert_eq!(lib.playlists.len(), 1);
        assert_eq!(lib.playlists[0].tracks.len(), 2);
        assert_eq!(lib.favorites.len(), 1);
        drop(lib);
        let st = store.settings();
        assert_eq!(st.crossfade, 4.0);
        assert_eq!(st.local_folders, vec!["/other".to_string()], "machine paths stay");
        assert_eq!(std::fs::read_to_string(store.dir.join("plays.jsonl")).unwrap().lines().count(), 1);
        let undo: Vec<_> = std::fs::read_dir(safety_dir(&store)).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("before-restore-")).collect();
        assert_eq!(undo.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_files_that_are_not_backups() {
        let p = std::env::temp_dir().join(format!("tw-notbackup-{}.json", now_ms()));
        std::fs::write(&p, "{\"format\":\"something-else\",\"version\":1}").unwrap();
        assert!(read(&p).is_err());
        std::fs::write(&p, "not json").unwrap();
        assert!(read(&p).is_err());
        let _ = std::fs::remove_file(p);
    }
}
