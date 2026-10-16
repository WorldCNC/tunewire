//! Local music library: audio files in user-chosen folders, indexed by their
//! tags (title / artist / album / duration) and searchable like any source.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use lofty::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::models::{Platform, TrackVersion};
use crate::search::norm_text;

const EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "mp4", "flac", "ogg", "oga", "opus", "wav", "webm", "mka", "mkv", "aif", "aiff", "caf"];
const MAX_FILES: usize = 100_000;

#[derive(Default, Serialize, Deserialize)]
struct Index {
    tracks: Vec<TrackVersion>,
}

fn index() -> &'static RwLock<Index> {
    static I: OnceLock<RwLock<Index>> = OnceLock::new();
    I.get_or_init(Default::default)
}

static INDEX_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Load the saved index (call once at startup).
pub fn init(data_dir: &Path) {
    let p = data_dir.join("local_index.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(i) = serde_json::from_str::<Index>(&s) {
            *index().write().unwrap() = i;
        }
    }
    let _ = INDEX_PATH.set(p);
}

pub fn has_tracks() -> bool {
    !index().read().unwrap().tracks.is_empty()
}

pub fn count() -> usize {
    index().read().unwrap().tracks.len()
}

pub fn is_audio(p: &Path) -> bool {
    p.extension().map(|e| EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let p = e.path();
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&p, out),
            Ok(t) if t.is_file() && is_audio(&p) => out.push(p),
            _ => {}
        }
    }
}

/// Read tags from one file; falls back to "Artist - Title.ext" file names.
pub fn read_file(p: &Path) -> TrackVersion {
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (mut artist, mut title) = crate::search::split_artist_title(&stem, "");
    let mut album = None;
    let mut duration = None;
    if let Ok(f) = lofty::read_from_path(p) {
        let d = f.properties().duration().as_secs_f64();
        if d > 0.0 {
            duration = Some(d);
        }
        if let Some(tag) = f.primary_tag().or_else(|| f.first_tag()) {
            if let Some(t) = tag.title().filter(|t| !t.trim().is_empty()) {
                title = t.trim().to_string();
            }
            if let Some(a) = tag.artist().filter(|a| !a.trim().is_empty()) {
                artist = a.trim().to_string();
            }
            album = tag.album().map(|a| a.trim().to_string()).filter(|a| !a.is_empty());
        }
    }
    let path = p.to_string_lossy().into_owned();
    TrackVersion {
        platform: Platform::Local,
        id: path.clone(),
        url: path,
        title,
        artist,
        duration,
        thumbnail: None,
        preview: false,
        partial: false,
        artist_url: None,
        album,
        rank: 0,
    }
}

/// Rescan the given folders (blocking) and persist the index.
pub fn scan(folders: &[String]) -> AppResult<usize> {
    let mut files = Vec::new();
    for f in folders {
        walk(Path::new(f), &mut files);
    }
    files.sort();
    files.dedup();
    let tracks: Vec<TrackVersion> = files.iter().map(|p| read_file(p)).collect();
    let n = tracks.len();
    let idx = Index { tracks };
    if let Some(p) = INDEX_PATH.get() {
        crate::store::write_atomic(p, &serde_json::to_vec(&idx)?)?;
    }
    *index().write().unwrap() = idx;
    Ok(n)
}

/// Token match over title / artist / album, best matches first.
pub fn search(query: &str, limit: usize) -> Vec<TrackVersion> {
    let q: Vec<String> = norm_text(query).split_whitespace().map(str::to_string).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let idx = index().read().unwrap();
    let mut scored: Vec<(f64, &TrackVersion)> = idx
        .tracks
        .iter()
        .filter_map(|t| {
            let hay = norm_text(&format!("{} {} {}", t.title, t.artist, t.album.as_deref().unwrap_or("")));
            let toks: Vec<&str> = hay.split_whitespace().collect();
            let hits = q.iter().filter(|w| toks.iter().any(|t| t.starts_with(w.as_str()))).count();
            (hits == q.len()).then(|| (hits as f64 + if norm_text(&t.title).starts_with(&q[0]) { 0.5 } else { 0.0 }, t))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(i, (_, t))| TrackVersion { rank: i, ..t.clone() })
        .collect()
}

/// All indexed tracks (for the Local view).
pub fn all() -> Vec<TrackVersion> {
    index().read().unwrap().tracks.clone()
}
