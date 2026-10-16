//! Shared application state and the event helpers every module uses.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::{AudioHandle, EngineState};
use crate::discord;
use crate::models::{Platform, Track, TrackVersion};
use crate::plugins::PluginStore;
use crate::search::SearchCache;
use crate::store::Store;
use crate::themes::ThemeStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub uid: u64,
    pub track: Track,
    /// User-chosen platform override for this queue entry.
    #[serde(default)]
    pub platform: Option<Platform>,
}

#[derive(Default)]
pub struct QueueState {
    pub items: Vec<QueueItem>,
    pub current: Option<usize>,
    /// When the playing item is removed, the index that should play next.
    pub resume_at: Option<usize>,
    pub next_uid: u64,
}

impl QueueState {
    pub fn make(&mut self, track: Track, platform: Option<Platform>) -> QueueItem {
        self.next_uid += 1;
        let mut track = track;
        track.score = 0.0;
        QueueItem { uid: self.next_uid, track, platform }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Loading,
    Active,
    Error,
}

#[derive(Debug, Clone)]
pub struct NowPlaying {
    pub phase: Phase,
    pub item: Option<QueueItem>,
    pub version: Option<TrackVersion>,
    pub message: Option<String>,
}

/// The next track, already decoding in the engine.
#[derive(Debug, Clone)]
pub struct Preload {
    pub uid: u64,
    pub token: u64,
    pub item: QueueItem,
    pub version: TrackVersion,
}

pub struct AppState {
    pub store: Store,
    pub themes: ThemeStore,
    pub plugins: PluginStore,
    pub cache: Arc<SearchCache>,
    pub queue: Mutex<QueueState>,
    pub now: Mutex<NowPlaying>,
    pub audio: AudioHandle,
    /// Bumped on every user play request; stale loads compare against it and bail.
    pub load_gen: AtomicU64,
    /// Source of unique engine tokens (current + preloaded tracks).
    pub next_token: AtomicU64,
    /// Token of the track the engine is playing.
    pub current_token: AtomicU64,
    pub preload: Mutex<Option<Preload>>,
    pub preload_gen: AtomicU64,
    pub preload_inflight: AtomicBool,
    pub discord: discord::DiscordHandle,
    pub consecutive_failures: AtomicU32,
    pub data_dir: PathBuf,
    /// Signature of the last full playback event (see `emit_tick`).
    pub last_full: Mutex<String>,
    /// Bumped per search; stale background work compares against it.
    pub search_gen: AtomicU64,
    /// Debounces settings.json writes while the volume slider moves.
    pub volume_save_gen: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaybackState {
    pub status: &'static str,
    pub position: f64,
    pub duration: Option<f64>,
    pub buffered: Option<f64>,
    pub stalled: bool,
    pub volume: f32,
    pub repeat: String,
    pub index: Option<usize>,
    pub current: Option<QueueItem>,
    pub version: Option<TrackVersion>,
    pub message: Option<String>,
    /// Queue uid of the preloaded next track (gapless), if any.
    pub preloaded: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueSnapshot {
    pub items: Vec<QueueItem>,
    pub current: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct StatusMsg {
    level: &'static str,
    message: String,
}

impl AppState {
    pub fn playback_state(&self) -> PlaybackState {
        let eng = self.audio.status();
        let now = self.now.lock().unwrap().clone();
        let settings = self.store.settings.lock().unwrap();
        let index = self.queue.lock().unwrap().current;
        let status = match now.phase {
            Phase::Idle => "idle",
            Phase::Loading => "loading",
            Phase::Error => "error",
            Phase::Active => match eng.state {
                EngineState::Playing => "playing",
                EngineState::Paused => "paused",
                EngineState::Idle => "idle",
            },
        };
        let active = now.phase == Phase::Active;
        PlaybackState {
            status,
            position: if active { eng.position } else { 0.0 },
            duration: if active { eng.duration } else { None }
                .or(now.version.as_ref().and_then(|v| v.duration))
                .or(now.item.as_ref().and_then(|i| i.track.duration)),
            buffered: if active { eng.buffered } else { None },
            stalled: active && eng.stalled,
            volume: settings.volume,
            repeat: settings.repeat.clone(),
            index,
            current: now.item,
            version: now.version,
            message: now.message,
            preloaded: self.preload.lock().unwrap().as_ref().filter(|p| eng.preloaded == Some(p.token)).map(|p| p.uid),
        }
    }

    pub fn queue_snapshot(&self) -> QueueSnapshot {
        let q = self.queue.lock().unwrap();
        QueueSnapshot { items: q.items.clone(), current: q.current }
    }

    pub fn set_now(&self, f: impl FnOnce(&mut NowPlaying)) {
        f(&mut self.now.lock().unwrap());
    }

    pub fn downloads_dir(&self) -> PathBuf {
        let d = self.store.settings().downloads_dir;
        if d.trim().is_empty() {
            self.data_dir.join("downloads")
        } else {
            PathBuf::from(d)
        }
    }
}

pub fn emit_status(app: &AppHandle, level: &'static str, message: impl Into<String>) {
    let _ = app.emit("status", StatusMsg { level, message: message.into() });
}

pub fn emit_playback(app: &AppHandle) {
    if let Some(st) = app.try_state::<AppState>() {
        let ps = st.playback_state();
        st.discord.update(presence(&ps));
        *st.last_full.lock().unwrap() = signature(&ps);
        let _ = app.emit("playback", ps);
    }
}

/// Everything except progress: if this is unchanged, a tick is enough.
fn signature(ps: &PlaybackState) -> String {
    format!(
        "{}|{:?}|{:?}|{:?}|{:?}|{}|{}|{:?}|{:?}",
        ps.status,
        ps.current.as_ref().map(|c| c.uid),
        ps.version.as_ref().map(|v| &v.url),
        ps.message,
        ps.index,
        ps.repeat,
        ps.volume,
        ps.preloaded,
        ps.duration.map(|d| d as i64)
    )
}

#[derive(Clone, Serialize)]
struct Tick {
    status: &'static str,
    position: f64,
    duration: Option<f64>,
    buffered: Option<f64>,
    stalled: bool,
}

/// Periodic progress update: a full event only when the state changed,
/// otherwise just position/buffering (a few bytes instead of the whole track).
pub fn emit_tick(app: &AppHandle) {
    let Some(st) = app.try_state::<AppState>() else { return };
    let ps = st.playback_state();
    st.discord.update(presence(&ps));
    let sig = signature(&ps);
    let changed = *st.last_full.lock().unwrap() != sig;
    if changed {
        *st.last_full.lock().unwrap() = sig;
        let _ = app.emit("playback", ps);
    } else {
        let _ = app.emit("playback-tick", Tick { status: ps.status, position: ps.position, duration: ps.duration, buffered: ps.buffered, stalled: ps.stalled });
    }
}

fn presence(ps: &PlaybackState) -> Option<discord::Presence> {
    if !matches!(ps.status, "playing" | "paused") {
        return None;
    }
    let (item, v) = (ps.current.as_ref()?, ps.version.as_ref()?);
    Some(discord::Presence {
        key: item.track.key.clone(),
        title: item.track.title.chars().take(120).collect(),
        artist: if item.track.artist.is_empty() { "Unknown artist".into() } else { item.track.artist.chars().take(120).collect() },
        platform: v.platform.label().into(),
        url: if v.platform == Platform::Local { String::new() } else { v.url.clone() },
        cover: v.thumbnail.clone().or(item.track.thumbnail.clone()).map(|u| u.replace("-original.", "-t500x500.").replace("/mqdefault.jpg", "/hqdefault.jpg")),
        playing: ps.status == "playing",
        position: ps.position,
        duration: ps.duration,
    })
}

pub fn emit_queue(app: &AppHandle) {
    if let Some(st) = app.try_state::<AppState>() {
        let _ = app.emit("queue-changed", st.queue_snapshot());
    }
}

pub fn emit_library(app: &AppHandle, what: &str) {
    let _ = app.emit("library-changed", what);
}
