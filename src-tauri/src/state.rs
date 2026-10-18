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
    /// Queue order (uids) from before shuffle was turned on.
    pub unshuffled: Option<Vec<u64>>,
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
    pub last_full: AtomicU64,
    /// Bumped per search; stale background work compares against it.
    pub search_gen: AtomicU64,
    /// Debounces settings.json writes while the volume slider moves.
    pub volume_save_gen: AtomicU64,
    /// (queue uid, seconds): seek there when that entry next starts playing
    /// (output device switch, or the position saved at the last exit).
    pub resume_position: Mutex<Option<(u64, f64)>>,
    pub queue_save_gen: AtomicU64,
    pub queue_saved_at: Mutex<std::time::Instant>,
}

/// The queue as saved in queue.json (restored paused at the next start).
/// `current`/`position` there date from the last change to the songs; the
/// newer ones are in queue-pos.json (see `SavedPosition`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedQueue {
    pub items: Vec<QueueItem>,
    pub current: Option<usize>,
    pub position: f64,
}

/// queue-pos.json: which entry was playing and where (a few bytes, saved
/// every few seconds instead of the whole queue).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedPosition {
    pub uid: Option<u64>,
    pub position: f64,
}

impl SavedQueue {
    /// Load queue.json plus the newer position (older versions: queue.json only).
    pub fn load(dir: &std::path::Path) -> SavedQueue {
        let mut q: SavedQueue = crate::store::load_json(&dir.join("queue.json"));
        if let Ok(text) = std::fs::read_to_string(dir.join("queue-pos.json")) {
            match serde_json::from_str::<SavedPosition>(&text) {
                Ok(SavedPosition { uid: None, .. }) => {
                    q.current = None;
                    q.position = 0.0;
                }
                Ok(SavedPosition { uid: Some(uid), position }) => {
                    if let Some(i) = q.items.iter().position(|i| i.uid == uid) {
                        q.current = Some(i);
                        q.position = position;
                    }
                }
                Err(_) => {}
            }
        }
        q
    }
}

/// queue.json has changes not written yet.
static QUEUE_DIRTY: AtomicBool = AtomicBool::new(false);
static POSITION_SAVE_GEN: AtomicU64 = AtomicU64::new(0);

impl AppState {
    fn saved_position(&self, q: &QueueState) -> (Option<usize>, SavedPosition) {
        let active = self.now.lock().unwrap().phase == Phase::Active;
        let uid = q.current.and_then(|i| q.items.get(i)).map(|i| i.uid);
        let position = if active {
            self.audio.status().position
        } else {
            // Not started since launch: keep the position we restored.
            self.resume_position.lock().unwrap().filter(|(u, _)| Some(*u) == uid).map(|(_, p)| p).unwrap_or(0.0)
        };
        (q.current.filter(|_| uid.is_some()), SavedPosition { uid, position })
    }

    fn write_position(&self, pos: &SavedPosition) {
        if let Ok(bytes) = serde_json::to_vec(pos) {
            let _ = crate::store::write_replace(&self.data_dir.join("queue-pos.json"), &bytes);
        }
    }

    /// The whole queue (songs changed) and the position.
    pub fn save_queue_now(&self) {
        QUEUE_DIRTY.store(false, std::sync::atomic::Ordering::SeqCst);
        let (data, pos) = {
            let q = self.queue.lock().unwrap();
            let (current, pos) = self.saved_position(&q);
            (SavedQueue { items: q.items.clone(), current, position: pos.position }, pos)
        };
        *self.queue_saved_at.lock().unwrap() = std::time::Instant::now();
        if let Ok(bytes) = serde_json::to_vec(&data) {
            let _ = crate::store::write_replace(&self.data_dir.join("queue.json"), &bytes);
        }
        self.write_position(&pos);
    }

    /// Just which entry plays and where.
    pub fn save_position_now(&self) {
        let pos = self.saved_position(&self.queue.lock().unwrap()).1;
        *self.queue_saved_at.lock().unwrap() = std::time::Instant::now();
        self.write_position(&pos);
    }

    /// Everything pending, durably (the app is closing or restarting).
    pub fn save_all_now(&self) {
        if QUEUE_DIRTY.load(std::sync::atomic::Ordering::SeqCst) {
            self.save_queue_now();
        } else {
            self.save_position_now();
        }
        self.store.flush();
    }
}

/// Write queue.json shortly after the last change (batches rapid edits).
pub fn save_queue_soon(app: &AppHandle) {
    let Some(st) = app.try_state::<AppState>() else {
        return;
    };
    QUEUE_DIRTY.store(true, std::sync::atomic::Ordering::SeqCst);
    let gen = st.queue_save_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let st = app.state::<AppState>();
        if st.queue_save_gen.load(std::sync::atomic::Ordering::SeqCst) == gen {
            let st2 = app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || st2.state::<AppState>().save_queue_now()).await;
        }
    });
}

/// Write queue-pos.json shortly (the playing entry or its position changed).
pub fn save_position_soon(app: &AppHandle) {
    let gen = POSITION_SAVE_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        if POSITION_SAVE_GEN.load(std::sync::atomic::Ordering::SeqCst) == gen {
            let st2 = app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Some(st) = st2.try_state::<AppState>() {
                    st.save_position_now();
                }
            })
            .await;
        }
    });
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
    pub shuffle: bool,
    /// Sleep timer: ms timestamp, or 0 = at the end of this song.
    pub sleep_at: Option<u64>,
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
            duration: if active { eng.duration } else { None }.or(now.version.as_ref().and_then(|v| v.duration)).or(now.item.as_ref().and_then(|i| i.track.duration)),
            buffered: if active { eng.buffered } else { None },
            stalled: active && eng.stalled,
            volume: settings.volume,
            repeat: settings.repeat.clone(),
            shuffle: settings.shuffle,
            sleep_at: crate::commands::playback::sleep_at(),
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
        st.last_full.store(signature(&ps), std::sync::atomic::Ordering::Relaxed);
        crate::media::update(app, &ps);
        let _ = app.emit("playback", ps);
    }
}

/// Everything except progress, hashed (no allocation 4 times a second):
/// if this is unchanged, a tick is enough.
fn signature(ps: &PlaybackState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (ps.shuffle, ps.sleep_at.unwrap_or(1), ps.status, ps.current.as_ref().map(|c| c.uid)).hash(&mut h);
    (ps.version.as_ref().map(|v| &v.url), &ps.message, ps.index, &ps.repeat, ps.volume.to_bits()).hash(&mut h);
    (ps.preloaded, ps.duration.map(|d| d as i64)).hash(&mut h);
    h.finish()
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
    let Some(st) = app.try_state::<AppState>() else {
        return;
    };
    let ps = st.playback_state();
    crate::stats::on_tick(&ps);
    // Remember the position now and then, so a restart resumes close to it.
    if ps.status == "playing" && st.queue_saved_at.lock().unwrap().elapsed().as_secs() >= 10 {
        *st.queue_saved_at.lock().unwrap() = std::time::Instant::now();
        save_position_soon(app);
    }
    st.discord.update(presence(&ps));
    let sig = signature(&ps);
    let changed = st.last_full.load(std::sync::atomic::Ordering::Relaxed) != sig;
    if changed || ps.status == "playing" {
        crate::media::update(app, &ps);
    }
    if changed {
        st.last_full.store(sig, std::sync::atomic::Ordering::Relaxed);
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
    // A stand-in upload shows (and links to) the version the user picked.
    let shown = v.matched_for.and_then(|p| item.track.versions.iter().find(|x| x.platform == p)).unwrap_or(v);
    Some(discord::Presence {
        key: item.track.key.clone(),
        title: item.track.title.chars().take(120).collect(),
        artist: if item.track.artist.is_empty() { "Unknown artist".into() } else { item.track.artist.chars().take(120).collect() },
        platform: shown.platform.label().into(),
        url: if shown.platform == Platform::Local { String::new() } else { shown.url.clone() },
        cover: v.thumbnail.clone().or(item.track.thumbnail.clone()).map(|u| u.replace("-original.", "-t500x500.").replace("/mqdefault.jpg", "/hqdefault.jpg")),
        playing: ps.status == "playing",
        position: ps.position,
        duration: ps.duration,
    })
}

/// The songs in the queue changed (the whole list goes to the page).
pub fn emit_queue(app: &AppHandle) {
    if let Some(st) = app.try_state::<AppState>() {
        let _ = app.emit("queue-changed", st.queue_snapshot());
        save_queue_soon(app);
    }
}

#[derive(Clone, Serialize)]
struct QueueCurrent {
    current: Option<usize>,
    /// Queue uid at `current`: the page checks it against its own copy.
    uid: Option<u64>,
}

/// Only the playing entry changed: a few bytes instead of the whole queue.
pub fn emit_queue_current(app: &AppHandle) {
    if let Some(st) = app.try_state::<AppState>() {
        let ev = {
            let q = st.queue.lock().unwrap();
            QueueCurrent { current: q.current, uid: q.current.and_then(|i| q.items.get(i)).map(|i| i.uid) }
        };
        let _ = app.emit("queue-current", ev);
        save_position_soon(app);
    }
}

pub fn emit_library(app: &AppHandle, what: &str) {
    let _ = app.emit("library-changed", what);
    if matches!(what, "playlists" | "favorites") {
        crate::matcher::kick(app); // new Spotify songs: find them on YouTube Music
    }
}
