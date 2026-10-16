use tauri::{AppHandle, State};

use crate::commands::playback::stop;
use crate::controller::invalidate_preload;
use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track};
use crate::state::{emit_queue, AppState, QueueItem, QueueSnapshot};
use crate::store;

/// Every queue mutation can change what plays next.
fn changed(app: &AppHandle, st: &AppState) -> QueueSnapshot {
    invalidate_preload(st);
    emit_queue(app);
    st.queue_snapshot()
}

#[tauri::command]
pub fn get_queue(st: State<'_, AppState>) -> QueueSnapshot {
    st.queue_snapshot()
}

#[tauri::command]
pub fn add_to_queue(app: AppHandle, st: State<'_, AppState>, tracks: Vec<Track>, next: Option<bool>) -> QueueSnapshot {
    {
        let mut q = st.queue.lock().unwrap();
        let items: Vec<QueueItem> = tracks.into_iter().map(|t| q.make(t, None)).collect();
        let at = if next.unwrap_or(false) { q.current.map(|c| c + 1).unwrap_or(0) } else { q.items.len() };
        let at = at.min(q.items.len());
        let n = items.len();
        q.items.splice(at..at, items);
        if let Some(r) = q.resume_at {
            if at <= r {
                q.resume_at = Some(r + n);
            }
        }
    }
    changed(&app, &st)
}

#[tauri::command]
pub fn remove_from_queue(app: AppHandle, st: State<'_, AppState>, uid: u64) -> AppResult<QueueSnapshot> {
    {
        let mut q = st.queue.lock().unwrap();
        let idx = q.items.iter().position(|i| i.uid == uid).ok_or_else(|| AppError::NotFound("queue item".into()))?;
        q.items.remove(idx);
        match q.current {
            Some(c) if c == idx => {
                // Keep playing; when it ends, continue with whatever slid into its slot.
                q.current = None;
                q.resume_at = Some(idx);
            }
            Some(c) if c > idx => q.current = Some(c - 1),
            _ => {}
        }
        if let Some(r) = q.resume_at {
            if r > idx {
                q.resume_at = Some(r - 1);
            }
        }
    }
    Ok(changed(&app, &st))
}

#[tauri::command]
pub fn clear_queue(app: AppHandle, st: State<'_, AppState>, keep_current: Option<bool>) -> QueueSnapshot {
    {
        let mut q = st.queue.lock().unwrap();
        if keep_current.unwrap_or(true) {
            if let Some(c) = q.current {
                let item = q.items[c].clone();
                q.items = vec![item];
                q.current = Some(0);
            } else {
                q.items.clear();
            }
        } else {
            q.items.clear();
            q.current = None;
        }
        q.resume_at = None;
    }
    if !keep_current.unwrap_or(true) {
        stop(app.clone(), st.clone());
    }
    changed(&app, &st)
}

#[tauri::command]
pub fn reorder_queue(app: AppHandle, st: State<'_, AppState>, from: usize, to: usize) -> AppResult<QueueSnapshot> {
    {
        let mut q = st.queue.lock().unwrap();
        let len = q.items.len();
        if from >= len || to >= len {
            return Err(AppError::Invalid("queue index out of range".into()));
        }
        let item = q.items.remove(from);
        q.items.insert(to, item);
        if let Some(c) = q.current {
            q.current = Some(if c == from {
                to
            } else if from < c && c <= to {
                c - 1
            } else if to <= c && c < from {
                c + 1
            } else {
                c
            });
        }
    }
    Ok(changed(&app, &st))
}

/// Shuffle everything after the current item.
#[tauri::command]
pub fn shuffle_queue(app: AppHandle, st: State<'_, AppState>) -> QueueSnapshot {
    {
        let mut q = st.queue.lock().unwrap();
        let start = q.current.map(|c| c + 1).unwrap_or(0);
        // xorshift seeded from the clock — good enough for a shuffle.
        let mut x = store::now_ms() | 1;
        let tail = &mut q.items[start..];
        for i in (1..tail.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            tail.swap(i, (x % (i as u64 + 1)) as usize);
        }
    }
    changed(&app, &st)
}

/// Shuffle mode: on shuffles what's coming up (remembering the order);
/// off puts the remaining songs back in their original order.
#[tauri::command]
pub fn set_shuffle(app: AppHandle, st: State<'_, AppState>, on: bool) -> QueueSnapshot {
    let _ = st.store.update_settings(|s| s.shuffle = on);
    {
        let mut q = st.queue.lock().unwrap();
        if on {
            q.unshuffled = Some(q.items.iter().map(|i| i.uid).collect());
            shuffle_tail(&mut q);
        } else if let Some(order) = q.unshuffled.take() {
            let start = q.current.map(|c| c + 1).unwrap_or(0);
            let pos: std::collections::HashMap<u64, usize> = order.iter().enumerate().map(|(i, u)| (*u, i)).collect();
            q.items[start..].sort_by_key(|i| pos.get(&i.uid).copied().unwrap_or(usize::MAX));
        }
    }
    crate::controller::invalidate_preload(&st);
    crate::state::emit_playback(&app);
    changed(&app, &st)
}

/// Shuffle everything after the current item.
pub fn shuffle_tail(q: &mut crate::state::QueueState) {
    let start = q.current.map(|c| c + 1).unwrap_or(0);
    let mut x = store::now_ms() | 1;
    let tail = &mut q.items[start..];
    for i in (1..tail.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        tail.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

/// Override which platform a queue entry should play from.
#[tauri::command]
pub fn set_item_platform(app: AppHandle, st: State<'_, AppState>, uid: u64, platform: Option<Platform>) -> AppResult<()> {
    {
        let mut q = st.queue.lock().unwrap();
        let it = q.items.iter_mut().find(|i| i.uid == uid).ok_or_else(|| AppError::NotFound("queue item".into()))?;
        it.platform = platform;
    }
    changed(&app, &st);
    Ok(())
}
