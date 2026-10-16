//! Playback controller: resolving a queue item to a playable stream (local
//! file, download, or yt-dlp with platform fallback), starting it, preloading
//! the next one for gapless playback, and reacting to engine events.

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager};

use crate::audio::{self, EngineEvent, EngineState, LoadedTrack};
use crate::downloads;
use crate::error::{AppError, AppResult};
use crate::models::{Platform, TrackVersion};
use crate::search;
use crate::state::{emit_library, emit_playback, emit_queue, emit_status, AppState, Phase, Preload, QueueItem};
use crate::stream;
use crate::ytdlp;

/// Open one concrete version: local file, offline download, or network stream.
async fn load_version(st: &AppState, v: &TrackVersion, token: u64) -> AppResult<(LoadedTrack, TrackVersion)> {
    let local_path = if v.platform == Platform::Local {
        Some(std::path::PathBuf::from(&v.url))
    } else {
        let lib = st.store.library.lock().unwrap();
        lib.downloads.iter().find(|d| d.version.url == v.url && std::path::Path::new(&d.path).exists()).map(|d| d.path.clone().into())
    };
    if let Some(path) = local_path {
        let fallback = v.duration;
        let loaded = tauri::async_runtime::spawn_blocking(move || {
            let opened = stream::open_local(&path)?;
            audio::prepare(opened, fallback, token)
        })
        .await
        .map_err(|e| AppError::Extraction(e.to_string()))??;
        return Ok((loaded, v.clone()));
    }

    if v.platform == Platform::Spotify {
        // Real Spotify audio (librespot); only reached when logged in.
        let opened = crate::spotify_play::open(&v.id).await?;
        let fallback = v.duration;
        let loaded = tauri::async_runtime::spawn_blocking(move || audio::prepare(opened, fallback, token))
            .await
            .map_err(|e| AppError::Extraction(e.to_string()))??;
        return Ok((loaded, v.clone()));
    }

    let info = ytdlp::stream_info(&v.url).await?;
    let mut version = v.clone();
    if v.partial {
        if let Some(t) = &info.title {
            version.title = t.clone();
        }
        if let Some(a) = &info.artist {
            version.artist = search::clean_uploader(a);
        }
        version.thumbnail = version.thumbnail.or(info.thumbnail.clone());
        version.duration = version.duration.or(info.duration);
        version.partial = false;
    }
    let fallback = info.duration.or(v.duration);
    let open = |info: ytdlp::StreamInfo| {
        tauri::async_runtime::spawn_blocking(move || {
            let opened = stream::open(&info)?;
            audio::prepare(opened, fallback, token)
        })
    };
    let loaded = match open(info).await.map_err(|e| AppError::Extraction(e.to_string()))? {
        Ok(l) => l,
        // A prefetched URL may have expired or be rejected: resolve it again once.
        Err(AppError::Unavailable(_) | AppError::Network(_) | AppError::Extraction(_)) => {
            ytdlp::forget_stream(&v.url);
            let fresh = ytdlp::stream_info(&v.url).await?;
            open(fresh).await.map_err(|e| AppError::Extraction(e.to_string()))??
        }
        Err(e) => return Err(e),
    };
    Ok((loaded, version))
}

/// Versions to try, best first: downloads, then the user's override, then preference order.
fn candidates(st: &AppState, item: &QueueItem) -> Vec<TrackVersion> {
    let pref = st.store.settings().platform_order;
    let mut c = item.track.versions.clone();
    search::sort_versions(&mut c, &pref);
    // The matched recording starts much faster than Spotify's own stream;
    // that's used only when Spotify was picked explicitly.
    if item.platform != Some(Platform::Spotify) && c.iter().any(|v| v.matched_for == Some(Platform::Spotify)) {
        c.sort_by_key(|v| v.platform == Platform::Spotify);
    }
    if let Some(p) = item.platform {
        if let Some(pos) = c.iter().position(|v| v.platform == p) {
            let v = c.remove(pos);
            c.insert(0, v);
        }
    }
    let lib = st.store.library.lock().unwrap();
    if let Some(d) = downloads::find(&lib.downloads, &item.track) {
        c.retain(|v| v.url != d.version.url);
        c.insert(0, d.version.clone());
    }
    c
}

/// Resolve the stream a click on `track` would play, ahead of time.
pub fn prefetch(app: &AppHandle, track: crate::models::Track, platform: Option<Platform>) {
    let st = app.state::<AppState>();
    let mut track = track;
    crate::matcher::apply(&mut track);
    let item = QueueItem { uid: 0, track, platform };
    let logged_in = crate::spotify_play::has_login();
    let first = candidates(&st, &item).into_iter().find(|v| v.platform != Platform::Spotify || logged_in);
    match first {
        Some(v) if matches!(v.platform, Platform::Local | Platform::Spotify) => {}
        Some(v) if !std::path::Path::new(&v.url).exists() => ytdlp::prefetch_stream(v.url),
        Some(_) => {}
        // Spotify-only: find the exact recording now, then its stream.
        None => {
            let Some(sp) = item.track.versions.iter().find(|v| v.platform == Platform::Spotify).cloned() else { return };
            tauri::async_runtime::spawn(async move {
                if let Some(v) = crate::matcher::match_spotify(&sp).await {
                    ytdlp::prefetch_stream(v.url);
                }
            });
        }
    }
}

/// Replace a placeholder track's metadata (queue entry + the item we report).
fn complete_metadata(st: &AppState, item: &mut QueueItem, version: &TrackVersion) {
    if !item.track.versions.iter().any(|v| v.partial && v.url == version.url) {
        return;
    }
    let mut versions = item.track.versions.clone();
    for v in versions.iter_mut().filter(|v| v.url == version.url) {
        *v = version.clone();
    }
    let track = search::build_track(versions, 0.0);
    item.track = track.clone();
    let mut q = st.queue.lock().unwrap();
    if let Some(qi) = q.items.iter_mut().find(|i| i.uid == item.uid) {
        qi.track = track;
    }
}

/// Resolve and open a queue item, trying every version and finally a search
/// for an equivalent upload. `progress` gets user-facing status updates.
async fn load_item(
    app: &AppHandle,
    item: &mut QueueItem,
    token: u64,
    cancelled: &(dyn Fn() -> bool + Send + Sync),
    interactive: bool,
) -> AppResult<(LoadedTrack, TrackVersion)> {
    let st = app.state::<AppState>();
    let say = |msg: String| {
        if interactive {
            st.set_now(|n| n.message = Some(msg));
            emit_playback(app);
        }
    };
    if crate::matcher::apply(&mut item.track) {
        let mut q = st.queue.lock().unwrap();
        if let Some(qi) = q.items.iter_mut().find(|i| i.uid == item.uid) {
            qi.track = item.track.clone();
        }
    }
    let mut tried: Vec<Platform> = Vec::new();
    let mut last_err: Option<AppError> = None;
    let chose_spotify = item.platform == Some(Platform::Spotify) && !crate::spotify_play::has_login();
    for v in candidates(&st, item) {
        if cancelled() {
            return Err(AppError::Cancelled);
        }
        if v.platform == Platform::Spotify && !crate::spotify_play::has_login() {
            continue; // not logged in: played through a matched upload (below)
        }
        // Spotify picked explicitly: play its recording (ISRC match), not
        // whatever other upload happened to be grouped with the song.
        if chose_spotify && v.platform != Platform::Spotify && v.matched_for != Some(Platform::Spotify) {
            continue;
        }
        say(format!("Loading from {}…", v.platform.label()));
        match load_version(&st, &v, token).await {
            Ok((loaded, v)) => {
                complete_metadata(&st, item, &v);
                return Ok((loaded, v));
            }
            Err(e) => {
                if interactive && !cancelled() {
                    emit_status(app, "warn", format!("{} failed: {e}. Trying the next source…", v.platform.label()));
                }
                tried.push(v.platform);
                last_err = Some(e);
            }
        }
    }
    if cancelled() {
        return Err(AppError::Cancelled);
    }
    let mut found = None;
    if let Some(sp) = item.track.versions.iter().find(|v| v.platform == Platform::Spotify).cloned() {
        if !tried.contains(&Platform::Ytmusic) {
            say("Finding the exact recording…".into());
            found = crate::matcher::match_spotify(&sp).await;
            if cancelled() {
                return Err(AppError::Cancelled);
            }
        }
    }
    if let Some(v) = found {
        say(format!("Loading from {}…", v.platform.label()));
        match load_version(&st, &v, token).await {
            Ok((loaded, v)) => {
                remember_match(&st, item, &v);
                return Ok((loaded, v));
            }
            Err(e) => {
                tried.push(v.platform);
                last_err = Some(e);
            }
        }
        if cancelled() {
            return Err(AppError::Cancelled);
        }
    }
    say("Searching for a playable equivalent…".into());
    let pref = st.store.settings().platform_order;
    let exclude: Vec<Platform> = tried.into_iter().filter(|p| !matches!(p, Platform::Spotify | Platform::Local)).collect();
    let v = match search::resolve_equivalent(&item.track.artist, &item.track.title, item.track.duration, &pref, &exclude, &st.cache).await {
        Ok(mut v) => {
            if item.track.has(Platform::Spotify) {
                v.matched_for = Some(Platform::Spotify);
            }
            v
        }
        Err(e) => return Err(last_err.filter(|_| !matches!(e, AppError::Unavailable(_))).unwrap_or(e)),
    };
    if cancelled() {
        return Err(AppError::Cancelled);
    }
    say(format!("Loading from {}…", v.platform.label()));
    let (loaded, v) = load_version(&st, &v, token).await?;
    remember_match(&st, item, &v);
    Ok((loaded, v))
}

/// Remember a matched upload so the version picker / next play can use it.
fn remember_match(st: &AppState, item: &mut QueueItem, v: &TrackVersion) {
    let known = if v.matched_for.is_some() { item.track.versions.iter().any(|x| x.url == v.url) } else { item.track.has(v.platform) };
    if !known {
        item.track.versions.push(v.clone());
        let mut q = st.queue.lock().unwrap();
        if let Some(qi) = q.items.iter_mut().find(|i| i.uid == item.uid) {
            qi.track = item.track.clone();
        }
    }
}

fn superseded(st: &AppState, gen: u64) -> bool {
    st.load_gen.load(Ordering::SeqCst) != gen
}

/// Forget the preloaded next track (queue order or repeat mode changed).
pub fn invalidate_preload(st: &AppState) {
    st.preload_gen.fetch_add(1, Ordering::SeqCst);
    if st.preload.lock().unwrap().take().is_some() {
        st.audio.clear_preload();
    }
}

/// Start playing queue item `index` in the background.
pub fn spawn_play(app: &AppHandle, index: usize, auto: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let st = app.state::<AppState>();
        match play_index(&app, index).await {
            Ok(()) => st.consecutive_failures.store(0, Ordering::SeqCst),
            Err(AppError::Cancelled) => {}
            Err(e) => {
                emit_status(&app, "error", e.to_string());
                st.set_now(|n| {
                    n.phase = Phase::Error;
                    n.message = Some(e.to_string());
                });
                emit_playback(&app);
                // Keep a playlist going past dead tracks, but never loop forever.
                let fails = st.consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
                if auto && fails < 5 {
                    advance(&app, true);
                }
            }
        }
    });
}

async fn play_index(app: &AppHandle, index: usize) -> AppResult<()> {
    let st = app.state::<AppState>();
    let gen = st.load_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let token = st.next_token.fetch_add(1, Ordering::SeqCst) + 1;
    invalidate_preload(&st);
    let mut item = {
        let mut q = st.queue.lock().unwrap();
        let item = q.items.get(index).cloned().ok_or_else(|| AppError::NotFound("queue item".into()))?;
        q.current = Some(index);
        q.resume_at = None;
        item
    };
    st.audio.stop();
    st.set_now(|n| {
        n.phase = Phase::Loading;
        n.item = Some(item.clone());
        n.version = None;
        n.message = Some("Resolving stream…".into());
    });
    emit_queue(app);
    emit_playback(app);

    let st2 = st.inner();
    let cancelled = move || superseded(st2, gen);
    let (loaded, version) = load_item(app, &mut item, token, &cancelled, true).await?;
    if superseded(&st, gen) {
        loaded.cancel();
        return Err(AppError::Cancelled);
    }
    st.audio.play(loaded);
    st.current_token.store(token, Ordering::SeqCst);
    let resume = st.resume_position.lock().unwrap().take();
    if let Some((_, pos)) = resume.filter(|(uid, _)| *uid == item.uid) {
        st.audio.seek(pos);
    }
    started(app, item, version);
    Ok(())
}

/// Bookkeeping once a track is audible.
fn started(app: &AppHandle, item: QueueItem, version: TrackVersion) {
    let st = app.state::<AppState>();
    st.set_now(|n| {
        n.phase = Phase::Active;
        n.item = Some(item.clone());
        n.version = Some(version.clone());
        n.message = None;
    });
    if st.store.push_history(&item.track, Some(version.platform)).is_ok() {
        emit_library(app, "history");
    }
    let _ = app.emit("track-started", serde_json::json!({ "item": item, "version": version }));
    emit_queue(app);
    emit_playback(app);
    maybe_autoplay(app);
    // "Next" is the most likely click; have its stream ready.
    let next = {
        let q = st.queue.lock().unwrap();
        q.current.and_then(|i| q.items.get(i + 1)).map(|n| (n.track.clone(), n.platform))
    };
    if let Some((t, p)) = next {
        prefetch(app, t, p);
    }
}

/// Index that should play after the current one (None = stop).
fn next_index(st: &AppState, auto: bool) -> Option<usize> {
    let repeat = st.store.settings().repeat;
    let q = st.queue.lock().unwrap();
    let len = q.items.len();
    if len == 0 {
        return None;
    }
    if let Some(i) = q.resume_at {
        return if i < len { Some(i) } else if repeat == "all" { Some(0) } else { None };
    }
    match q.current {
        Some(i) if auto && repeat == "one" => Some(i),
        Some(i) if i + 1 < len => Some(i + 1),
        Some(_) if repeat == "all" => Some(0),
        None => Some(0),
        _ => None,
    }
}

static RADIO_INFLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Append songs related to `seed` to the queue. Returns how many were added.
pub async fn extend_with_radio(app: &AppHandle, seed: crate::models::Track) -> usize {
    if RADIO_INFLIGHT.swap(true, Ordering::SeqCst) {
        return 0;
    }
    let st = app.state::<AppState>();
    let mut exclude: std::collections::HashSet<String> = st.queue.lock().unwrap().items.iter().map(|i| i.track.key.clone()).collect();
    exclude.extend(st.store.library.lock().unwrap().history.iter().take(100).map(|h| h.track.key.clone()));
    let found = crate::radio::related(&seed, &exclude, 20, &st.cache).await;
    RADIO_INFLIGHT.store(false, Ordering::SeqCst);
    match found {
        Ok(tracks) => {
            let n = tracks.len();
            {
                let mut q = st.queue.lock().unwrap();
                for t in tracks {
                    let item = q.make(t, None);
                    q.items.push(item);
                }
            }
            emit_queue(app);
            emit_playback(app);
            n
        }
        Err(e) => {
            emit_status(app, "warn", format!("Autoplay: {e}"));
            0
        }
    }
}

/// When the last song in the queue starts, line up related ones (autoplay).
fn maybe_autoplay(app: &AppHandle) {
    let st = app.state::<AppState>();
    let s = st.store.settings();
    if !s.autoplay || s.repeat != "off" {
        return;
    }
    let seed = {
        let q = st.queue.lock().unwrap();
        match q.current {
            Some(i) if i + 1 == q.items.len() => q.items[i].track.clone(),
            _ => return,
        }
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        extend_with_radio(&app, seed).await;
    });
}

/// Move to the next track according to the repeat mode.
pub fn advance(app: &AppHandle, auto: bool) {
    let st = app.state::<AppState>();
    if auto && crate::commands::playback::sleep_due_at_track_end() {
        st.audio.stop();
        st.set_now(|n| {
            n.phase = Phase::Idle;
            n.message = None;
        });
        emit_playback(app);
        emit_status(app, "info", "Sleep timer: stopped at the end of the song");
        return;
    }
    match next_index(&st, auto) {
        Some(i) => spawn_play(app, i, auto),
        None if auto && st.store.settings().autoplay && !RADIO_INFLIGHT.load(Ordering::SeqCst) => {
            // Queue ran out before autoplay had extended it.
            let seed = {
                let q = st.queue.lock().unwrap();
                q.current.and_then(|i| q.items.get(i).map(|x| x.track.clone()))
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let st = app.state::<AppState>();
                if let Some(seed) = seed {
                    if extend_with_radio(&app, seed).await > 0 {
                        if let Some(i) = next_index(&st, true) {
                            spawn_play(&app, i, true);
                            return;
                        }
                    }
                }
                st.audio.stop();
                st.set_now(|n| {
                    n.phase = Phase::Idle;
                    n.message = None;
                });
                emit_playback(&app);
            });
        }
        None => {
            st.audio.stop();
            st.set_now(|n| {
                n.phase = Phase::Idle;
                n.message = None;
            });
            emit_playback(app);
            emit_status(app, "info", "End of queue");
        }
    }
}

/// Start decoding the next track shortly before the current one ends.
fn maybe_preload(app: &AppHandle) {
    let st = app.state::<AppState>();
    let eng = st.audio.status();
    if st.now.lock().unwrap().phase != Phase::Active || eng.state != EngineState::Playing {
        return;
    }
    if crate::commands::playback::sleep_at() == Some(0) {
        return; // stopping after this song: don't roll into the next one
    }
    if eng.preloaded.is_some() || st.preload_inflight.load(Ordering::SeqCst) || st.preload.lock().unwrap().is_some() {
        return;
    }
    let Some(dur) = eng.duration else { return };
    let lead = (st.store.settings().crossfade as f64 + 20.0).max(25.0);
    if dur - eng.position > lead {
        return;
    }
    let settings = st.store.settings();
    if settings.repeat == "one" {
        return;
    }
    let Some(idx) = next_index(&st, true) else { return };
    let Some(mut item) = st.queue.lock().unwrap().items.get(idx).cloned() else { return };
    let pgen = st.preload_gen.load(Ordering::SeqCst);
    st.preload_inflight.store(true, Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let st = app.state::<AppState>();
        let token = st.next_token.fetch_add(1, Ordering::SeqCst) + 1;
        let st2 = st.inner();
        let cancelled = move || st2.preload_gen.load(Ordering::SeqCst) != pgen;
        let res = load_item(&app, &mut item, token, &cancelled, false).await;
        st.preload_inflight.store(false, Ordering::SeqCst);
        match res {
            Ok((loaded, version)) if !cancelled() => {
                st.audio.preload(loaded);
                *st.preload.lock().unwrap() = Some(Preload { uid: item.uid, token, item, version });
            }
            Ok((loaded, _)) => loaded.cancel(),
            Err(_) => {} // the normal path will retry (and report) at track end
        }
    });
}

pub fn on_engine_event(app: &AppHandle, ev: EngineEvent) {
    let Some(st) = app.try_state::<AppState>() else { return };
    match ev {
        EngineEvent::Tick => {
            crate::state::emit_tick(app);
            maybe_preload(app);
        }
        EngineEvent::Advanced { token } => {
            let pre = st.preload.lock().unwrap().take();
            let Some(p) = pre.filter(|p| p.token == token) else { return };
            let prev = st.now.lock().unwrap().item.clone();
            let _ = app.emit("track-ended", serde_json::json!({ "item": prev }));
            {
                let mut q = st.queue.lock().unwrap();
                q.current = q.items.iter().position(|i| i.uid == p.uid);
                q.resume_at = None;
            }
            st.current_token.store(token, Ordering::SeqCst);
            st.consecutive_failures.store(0, Ordering::SeqCst);
            started(app, p.item, p.version);
        }
        EngineEvent::Ended { token } => {
            if token != st.current_token.load(Ordering::SeqCst) {
                return;
            }
            let item = st.now.lock().unwrap().item.clone();
            let _ = app.emit("track-ended", serde_json::json!({ "item": item }));
            advance(app, true);
        }
        EngineEvent::Error { token, error } => {
            if token != st.current_token.load(Ordering::SeqCst) {
                return;
            }
            emit_status(app, "error", error.to_string());
            st.set_now(|n| {
                n.phase = Phase::Error;
                n.message = Some(error.to_string());
            });
            emit_playback(app);
            if !matches!(error, AppError::AudioDevice(_)) {
                advance(app, true);
            }
        }
    }
}
