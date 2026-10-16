//! Spotify songs can't be streamed without Premium, so each one is played from
//! its exact recording on YouTube Music. Finding that recording used to happen
//! at play time (slow); now it happens once, in the background, as soon as a
//! Spotify playlist is imported, and the result is saved (matches.json and
//! the playlist itself), so playing starts like any other song.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::AppResult;
use crate::models::{Platform, Track, TrackVersion};
use crate::state::AppState;
use crate::store::now_ms;

const CONCURRENCY: usize = 8;
/// A song with no match is retried after this long (new uploads appear).
const RETRY_MS: u64 = 3 * 86_400_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    v: Option<TrackVersion>,
    at: u64,
}

static MATCHES: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);
static PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);

pub fn init(dir: &std::path::Path) {
    let path = dir.join("matches.json");
    *MATCHES.lock().unwrap() = Some(crate::store::load_json(&path));
    *PATH.lock().unwrap() = Some(path);
}

fn save() {
    let Some(path) = PATH.lock().unwrap().clone() else { return };
    let data = MATCHES.lock().unwrap().as_ref().and_then(|m| serde_json::to_vec(m).ok());
    if let Some(data) = data {
        let _ = crate::store::write_atomic(&path, &data);
    }
}

fn entry(id: &str) -> Option<Entry> {
    MATCHES.lock().unwrap().as_ref().and_then(|m| m.get(id).cloned())
}

/// The saved match for a Spotify track id, if any.
pub fn lookup(id: &str) -> Option<TrackVersion> {
    entry(id).and_then(|e| e.v)
}

/// Worth (re)trying: never tried, or failed long enough ago.
fn pending(id: &str) -> bool {
    match entry(id) {
        None => true,
        Some(Entry { v: Some(_), .. }) => false,
        Some(Entry { v: None, at }) => at + RETRY_MS < now_ms(),
    }
}

fn spotify_version(track: &Track) -> Option<&TrackVersion> {
    track.versions.iter().find(|v| v.platform == Platform::Spotify)
}

/// Has something to play without Spotify Premium.
fn playable(track: &Track) -> bool {
    track.versions.iter().any(|v| v.platform != Platform::Spotify)
}

/// Spotify-only track that still needs a match.
fn needs_match(track: &Track) -> Option<TrackVersion> {
    if playable(track) {
        return None;
    }
    spotify_version(track).filter(|v| !v.id.is_empty() && pending(&v.id)).cloned()
}

/// Add the saved match to a Spotify-only track. Returns whether it changed.
pub fn apply(track: &mut Track) -> bool {
    if playable(track) {
        return false;
    }
    let Some(id) = spotify_version(track).map(|v| v.id.clone()) else { return false };
    match lookup(&id) {
        Some(v) => {
            track.versions.push(v);
            true
        }
        None => false,
    }
}

async fn ytm_songs(query: String) -> AppResult<Vec<TrackVersion>> {
    crate::ytm::search_songs(&query).await
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Find the YouTube Music recording of a Spotify track: by ISRC (the exact
/// recording), else by artist + title + length. Saved either way.
pub async fn match_spotify(sp: &TrackVersion) -> Option<TrackVersion> {
    if let Some(e) = entry(&sp.id) {
        if e.v.is_some() || e.at + RETRY_MS >= now_ms() {
            return e.v;
        }
    }
    let found = find(sp).await;
    let Ok(found) = found else { return None }; // network trouble: don't remember
    if let Some(m) = MATCHES.lock().unwrap().as_mut() {
        m.insert(sp.id.clone(), Entry { v: found.clone(), at: now_ms() });
    }
    found
}

async fn find(sp: &TrackVersion) -> AppResult<Option<TrackVersion>> {
    let finish = |mut v: TrackVersion, isrc: bool| {
        v.matched_for = Some(Platform::Spotify);
        v.isrc_match = isrc;
        if v.album.is_none() {
            v.album = sp.album.clone();
        }
        v
    };
    if let Ok(Some(isrc)) = crate::spotify::isrc(&sp.id).await {
        // The first hit is the recording with that ISRC; the rest is noise.
        if let Some(v) = ytm_songs(isrc).await?.into_iter().next() {
            if crate::search::same_title(sp, &v) {
                return Ok(Some(finish(v, true)));
            }
        }
    }
    let query = format!("{} {}", sp.artist.split(',').next().unwrap_or(&sp.artist), crate::search::strip_noise(&sp.title));
    let found = ytm_songs(query).await?;
    Ok(found.into_iter().take(6).find(|v| crate::search::same_recording(sp, v)).map(|v| finish(v, false)))
}

#[derive(Clone, Serialize)]
struct Progress {
    done: usize,
    total: usize,
    matched: usize,
    finished: bool,
    /// "library" (saved playlists, liked) or "other" (an opened link, the queue).
    scope: &'static str,
}

/// Match many Spotify versions in parallel; `tick` runs after each one.
async fn match_all(versions: Vec<TrackVersion>, tick: impl Fn(usize, usize) + Send + Sync + 'static) -> usize {
    let sem = Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let matched = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tick = Arc::new(tick);
    let tasks: Vec<_> = versions
        .into_iter()
        .map(|sp| {
            let (sem, done, matched, tick) = (sem.clone(), done.clone(), matched.clone(), tick.clone());
            tauri::async_runtime::spawn(async move {
                let _permit = sem.acquire().await;
                if match_spotify(&sp).await.is_some() {
                    matched.fetch_add(1, Ordering::SeqCst);
                }
                let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                tick(d, matched.load(Ordering::SeqCst));
            })
        })
        .collect();
    for t in tasks {
        let _ = t.await;
    }
    save();
    matched.load(Ordering::SeqCst)
}

/// Put saved matches into the library's playlists and liked songs.
fn apply_library(app: &AppHandle) {
    let st = app.state::<AppState>();
    let mut changed_pl = false;
    let mut changed_fav = false;
    let _ = st.store.update_library(|lib| {
        for p in lib.playlists.iter_mut() {
            for t in p.tracks.iter_mut() {
                changed_pl |= apply(t);
            }
        }
        for t in lib.favorites.iter_mut() {
            changed_fav |= apply(t);
        }
        Ok(())
    });
    // A quiet event: the UI swaps in the new data without re-rendering
    // (matches don't change what a row shows, only how fast it plays).
    if changed_pl {
        let _ = app.emit("library-matched", "playlists");
    }
    if changed_fav {
        let _ = app.emit("library-matched", "favorites");
    }
    apply_queue(app);
}

/// Put saved matches into the queue.
pub fn apply_queue(app: &AppHandle) {
    let st = app.state::<AppState>();
    let mut changed = false;
    for item in st.queue.lock().unwrap().items.iter_mut() {
        changed |= apply(&mut item.track);
    }
    if changed {
        let _ = app.emit("queue-changed", st.queue_snapshot());
    }
}

/// Forget old matches for songs that are no longer in the library.
fn prune(app: &AppHandle) {
    let keep: HashSet<String> = {
        let st = app.state::<AppState>();
        let lib = st.store.library.lock().unwrap();
        lib.playlists.iter().flat_map(|p| p.tracks.iter()).chain(lib.favorites.iter()).filter_map(spotify_version).map(|v| v.id.clone()).collect()
    };
    let cutoff = now_ms().saturating_sub(60 * 86_400_000);
    let mut removed = false;
    if let Some(m) = MATCHES.lock().unwrap().as_mut() {
        let before = m.len();
        m.retain(|id, e| keep.contains(id) || e.at > cutoff);
        removed = m.len() != before;
    }
    if removed {
        save();
    }
}

/// Match every Spotify-only song in the library (in the background; one run at a time).
pub fn kick(app: &AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            AGAIN.store(false, Ordering::SeqCst);
            apply_library(&app);
            let todo: Vec<TrackVersion> = {
                let st = app.state::<AppState>();
                let lib = st.store.library.lock().unwrap();
                let mut seen = HashSet::new();
                lib.playlists.iter().flat_map(|p| p.tracks.iter()).chain(lib.favorites.iter()).filter_map(needs_match).filter(|v| seen.insert(v.id.clone())).collect()
            };
            if !todo.is_empty() {
                let total = todo.len();
                let h = app.clone();
                let last_apply = Arc::new(Mutex::new(std::time::Instant::now()));
                let matched = match_all(todo, move |done, matched| {
                    let _ = h.emit("match-progress", Progress { done, total, matched, finished: false, scope: "library" });
                    // Save progress into the library every couple of seconds.
                    let mut last = last_apply.lock().unwrap();
                    if last.elapsed().as_secs_f64() > 2.5 {
                        *last = std::time::Instant::now();
                        drop(last);
                        apply_library(&h);
                    }
                })
                .await;
                apply_library(&app);
                let _ = app.emit("match-progress", Progress { done: total, total, matched, finished: true, scope: "library" });
            }
            if !AGAIN.load(Ordering::SeqCst) {
                break;
            }
        }
        prune(&app);
        RUNNING.store(false, Ordering::SeqCst);
        // A kick that raced with the end of the loop.
        if AGAIN.swap(false, Ordering::SeqCst) {
            kick(&app);
        }
    });
}

/// Match songs that aren't in the library (an opened Spotify link, the queue).
pub fn prepare(app: &AppHandle, tracks: Vec<Track>) {
    let mut seen = HashSet::new();
    let todo: Vec<TrackVersion> = tracks.iter().filter_map(needs_match).filter(|v| seen.insert(v.id.clone())).collect();
    if todo.is_empty() {
        return;
    }
    let app = app.clone();
    let total = todo.len();
    tauri::async_runtime::spawn(async move {
        let h = app.clone();
        let last_apply = Mutex::new(std::time::Instant::now());
        let matched = match_all(todo, move |done, matched| {
            let _ = h.emit("match-progress", Progress { done, total, matched, finished: false, scope: "other" });
            let mut last = last_apply.lock().unwrap();
            if last.elapsed().as_secs_f64() > 1.5 {
                *last = std::time::Instant::now();
                apply_queue(&h);
            }
        })
        .await;
        apply_queue(&app);
        let _ = app.emit("match-progress", Progress { done: total, total, matched, finished: true, scope: "other" });
    });
}

/// Saved matches for a list of tracks (for a remote playlist view).
pub fn with_matches(mut tracks: Vec<Track>) -> Vec<Track> {
    for t in tracks.iter_mut() {
        apply(t);
    }
    tracks
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn live_match_playlist() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        *MATCHES.lock().unwrap() = Some(HashMap::new());
        let mut sps = Vec::new();
        for q in ["daft punk one more time", "tame impala let it happen", "kendrick lamar money trees", "hiatus kaiyote nakamarra", "fleetwood mac dreams", "radiohead weird fishes", "billie eilish birds of a feather", "sza snooze", "arctic monkeys 505", "frank ocean pink + white", "mitski my love mine all mine", "the weeknd blinding lights"] {
            sps.push(rt.block_on(crate::spotify::search(q, 1)).unwrap().remove(0));
        }
        let t0 = std::time::Instant::now();
        let n = sps.len();
        let matched = rt.block_on(match_all(sps.clone(), |_, _| {}));
        println!("matched {matched}/{n} in {:?}", t0.elapsed());
        for sp in &sps {
            match lookup(&sp.id) {
                Some(v) => println!("  {}{} — {}  =>  {} — {} ({:?}s)", if v.isrc_match { "[isrc] " } else { "[text] " }, sp.title, sp.artist, v.title, v.artist, v.duration),
                None => println!("  [none] {} — {}", sp.title, sp.artist),
            }
        }
        assert!(matched * 10 >= n * 8, "at least 80% matched");
        let t1 = std::time::Instant::now();
        rt.block_on(match_all(sps, |_, _| {}));
        assert!(t1.elapsed().as_millis() < 50, "second run is from the cache");
    }

    #[test]
    #[ignore]
    fn live_match_big_playlist() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        *MATCHES.lock().unwrap() = Some(HashMap::new());
        let (title, tracks) = crate::spotify::fetch_playlist_full("37i9dQZF1DX4o1oenSJRJd").unwrap();
        let sps: Vec<TrackVersion> = tracks.into_iter().filter_map(|t| t.versions.into_iter().find(|v| v.platform == Platform::Spotify)).collect();
        let n = sps.len();
        let t0 = std::time::Instant::now();
        let matched = rt.block_on(match_all(sps.clone(), |_, _| {}));
        let isrc = sps.iter().filter_map(|s| lookup(&s.id)).filter(|v| v.isrc_match).count();
        println!("{title}: matched {matched}/{n} ({isrc} by ISRC) in {:?}", t0.elapsed());
        for sp in sps.iter().filter(|s| lookup(&s.id).is_none()) {
            println!("  [none] {} — {} ({:?})", sp.title, sp.artist, sp.duration);
        }
        assert!(matched * 100 >= n * 90);
    }
}
