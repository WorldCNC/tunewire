use std::sync::atomic::Ordering;

use tauri::{AppHandle, State};

use crate::audio::EngineState;
use crate::controller::{advance, invalidate_preload, spawn_play};
use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track};
use crate::state::{emit_playback, emit_queue, AppState, Phase, PlaybackState, QueueItem};

pub fn play_track_inner(app: &AppHandle, st: &AppState, mut track: Track, platform: Option<Platform>) {
    crate::matcher::apply(&mut track);
    let idx = {
        let mut q = st.queue.lock().unwrap();
        let item = q.make(track, platform);
        let at = q.current.map(|c| c + 1).unwrap_or(q.items.len()).min(q.items.len());
        q.items.insert(at, item);
        at
    };
    spawn_play(app, idx, false);
}

pub fn set_queue_and_play(app: &AppHandle, st: &AppState, tracks: Vec<Track>, start: usize) {
    let tracks = crate::matcher::with_matches(tracks);
    // Spotify songs coming up: match them before they're reached.
    crate::matcher::prepare(app, tracks.iter().skip(start).take(40).cloned().collect());
    let shuffle = st.store.settings().shuffle;
    let start = {
        let mut q = st.queue.lock().unwrap();
        let items: Vec<QueueItem> = tracks.into_iter().map(|t| q.make(t, None)).collect();
        q.items = items;
        q.current = None;
        q.resume_at = None;
        q.unshuffled = None;
        if shuffle && !q.items.is_empty() {
            // The clicked song plays first, the rest in random order.
            q.unshuffled = Some(q.items.iter().map(|i| i.uid).collect());
            let at = start.min(q.items.len() - 1);
            let first = q.items.remove(at);
            q.items.insert(0, first);
            q.current = Some(0);
            crate::commands::queue::shuffle_tail(&mut q);
            q.current = None;
            0
        } else {
            start
        }
    };
    let len = st.queue.lock().unwrap().items.len();
    emit_queue(app);
    if len > 0 {
        spawn_play(app, start.min(len - 1), false);
    }
}

/// Play a logical track now (inserted after the current item).
#[tauri::command]
pub fn play_track(app: AppHandle, st: State<'_, AppState>, track: Track, platform: Option<Platform>) {
    play_track_inner(&app, &st, track, platform);
}

/// Replace the queue with `tracks` and start at `start` (used for playlists).
#[tauri::command]
pub fn play_tracks(app: AppHandle, st: State<'_, AppState>, tracks: Vec<Track>, start: Option<usize>) {
    set_queue_and_play(&app, &st, tracks, start.unwrap_or(0));
}

#[tauri::command]
pub fn play_index_cmd(app: AppHandle, st: State<'_, AppState>, index: usize) -> AppResult<()> {
    if index >= st.queue.lock().unwrap().items.len() {
        return Err(AppError::NotFound("queue index".into()));
    }
    spawn_play(&app, index, false);
    Ok(())
}

#[tauri::command]
pub fn next_track(app: AppHandle) {
    advance(&app, false);
}

#[tauri::command]
pub fn previous_track(app: AppHandle, st: State<'_, AppState>) {
    let pos = st.audio.status().position;
    let cur = st.queue.lock().unwrap().current;
    match cur {
        Some(i) if pos < 3.0 && i > 0 => spawn_play(&app, i - 1, false),
        Some(_) if st.now.lock().unwrap().phase == Phase::Active => st.audio.seek(0.0),
        Some(i) => spawn_play(&app, i, false),
        None => {}
    }
}

#[tauri::command]
pub fn pause(app: AppHandle, st: State<'_, AppState>) {
    st.audio.pause();
    emit_soon(&app);
}

#[tauri::command]
pub fn resume(app: AppHandle, st: State<'_, AppState>) {
    let (phase, cur) = (st.now.lock().unwrap().phase, st.queue.lock().unwrap().current);
    match (phase, cur) {
        (Phase::Active, _) => st.audio.resume(),
        // Nothing loaded (fresh start / after error): (re)start the current item.
        (Phase::Idle | Phase::Error, Some(i)) => spawn_play(&app, i, false),
        (Phase::Idle | Phase::Error, None) if !st.queue.lock().unwrap().items.is_empty() => spawn_play(&app, 0, false),
        _ => {}
    }
    emit_soon(&app);
}

#[tauri::command]
pub fn toggle_pause(app: AppHandle, st: State<'_, AppState>) {
    if st.audio.status().state == EngineState::Playing && st.now.lock().unwrap().phase == Phase::Active {
        pause(app, st);
    } else {
        resume(app, st);
    }
}

#[tauri::command]
pub fn stop(app: AppHandle, st: State<'_, AppState>) {
    st.load_gen.fetch_add(1, Ordering::SeqCst);
    invalidate_preload(&st);
    st.audio.stop();
    st.set_now(|n| {
        n.phase = Phase::Idle;
        n.message = None;
    });
    emit_soon(&app);
}

/// Emit after the engine thread has processed the command.
pub fn emit_soon(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        emit_playback(&app);
    });
}

// Sleep timer ----------------------------------------------------------------

static SLEEP_AT: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);
static SLEEP_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// When playback will stop: a ms timestamp, or 0 = at the end of this song.
pub fn sleep_at() -> Option<u64> {
    *SLEEP_AT.lock().unwrap()
}

/// Called when a song ends: true if the sleep timer wants playback to stop now.
pub fn sleep_due_at_track_end() -> bool {
    let mut g = SLEEP_AT.lock().unwrap();
    if *g == Some(0) {
        *g = None;
        return true;
    }
    false
}

/// `minutes`: stop after that long; `end_of_track`: stop when this song ends; neither: cancel.
#[tauri::command]
pub fn set_sleep_timer(app: AppHandle, st: State<'_, AppState>, minutes: Option<f64>, end_of_track: Option<bool>) -> Option<u64> {
    let gen = SLEEP_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let at = if end_of_track.unwrap_or(false) {
        // Gapless preloading would roll straight into the next song.
        invalidate_preload(&st);
        Some(0)
    } else {
        minutes.filter(|m| *m > 0.0).map(|m| crate::store::now_ms() + (m * 60_000.0) as u64)
    };
    *SLEEP_AT.lock().unwrap() = at;
    if let Some(deadline) = at.filter(|a| *a > 0) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let wait = deadline.saturating_sub(crate::store::now_ms());
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            if SLEEP_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            *SLEEP_AT.lock().unwrap() = None;
            let st = tauri::Manager::state::<AppState>(&app);
            st.audio.pause();
            emit_playback(&app);
            crate::state::emit_status(&app, "info", "Sleep timer: playback paused");
        });
    }
    emit_playback(&app);
    at
}

#[tauri::command]
pub fn set_volume(app: AppHandle, st: State<'_, AppState>, volume: f32) -> AppResult<f32> {
    let v = volume.clamp(0.0, 1.0);
    st.audio.set_volume(v);
    if (st.store.settings().volume - v).abs() > f32::EPSILON {
        st.store.update_settings_nosave(|s| s.volume = v);
        // Write settings.json once the slider has been still for 600 ms.
        let gen = st.volume_save_gen.fetch_add(1, Ordering::SeqCst) + 1;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            let st = tauri::Manager::state::<AppState>(&app);
            if st.volume_save_gen.load(Ordering::SeqCst) == gen {
                let _ = st.store.save_settings();
            }
        });
    }
    Ok(v)
}

#[tauri::command]
pub fn seek(app: AppHandle, st: State<'_, AppState>, position: f64) {
    st.audio.seek(position);
    emit_soon(&app);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        crate::media::position_changed(&app);
    });
}

#[tauri::command]
pub fn get_playback_state(st: State<'_, AppState>) -> PlaybackState {
    st.playback_state()
}

#[tauri::command]
pub fn set_repeat(app: AppHandle, st: State<'_, AppState>, mode: String) -> AppResult<String> {
    let s = st.store.update_settings(|s| s.repeat = mode)?;
    invalidate_preload(&st);
    emit_playback(&app);
    Ok(s.repeat)
}

/// Play `track` now and fill the queue with related songs ("Start radio").
#[tauri::command]
pub fn start_radio(app: AppHandle, st: State<'_, AppState>, track: Track) {
    set_queue_and_play(&app, &st, vec![track.clone()], 0);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let n = crate::controller::extend_with_radio(&app, track).await;
        if n > 0 {
            crate::state::emit_status(&app, "ok", format!("Radio: {n} songs queued"));
        }
    });
}

/// Warm up the stream for a song the user is likely to play (hover, top result).
#[tauri::command]
pub fn prefetch_track(app: AppHandle, track: Track, platform: Option<crate::models::Platform>) {
    crate::controller::prefetch(&app, track, platform);
}
