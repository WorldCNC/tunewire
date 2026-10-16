use std::sync::atomic::Ordering;

use tauri::{AppHandle, State};

use crate::audio::EngineState;
use crate::controller::{advance, invalidate_preload, spawn_play};
use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track};
use crate::state::{emit_playback, emit_queue, AppState, Phase, PlaybackState, QueueItem};

pub fn play_track_inner(app: &AppHandle, st: &AppState, track: Track, platform: Option<Platform>) {
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
    {
        let mut q = st.queue.lock().unwrap();
        let items: Vec<QueueItem> = tracks.into_iter().map(|t| q.make(t, None)).collect();
        q.items = items;
        q.current = None;
        q.resume_at = None;
    }
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

#[tauri::command]
pub fn set_volume(st: State<'_, AppState>, volume: f32) -> AppResult<f32> {
    let v = volume.clamp(0.0, 1.0);
    st.audio.set_volume(v);
    if (st.store.settings().volume - v).abs() > f32::EPSILON {
        st.store.update_settings(|s| s.volume = v)?;
    }
    Ok(v)
}

#[tauri::command]
pub fn seek(app: AppHandle, st: State<'_, AppState>, position: f64) {
    st.audio.seek(position);
    emit_soon(&app);
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
