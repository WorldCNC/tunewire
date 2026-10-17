//! "For you": Daily Mixes (one per corner of your taste, familiar songs mixed
//! with new ones), Discover Weekly (only songs you've never played), plus
//! On Repeat and Forgotten favorites straight from the play log.
//!
//! Network mixes are built once a day / once a week and kept in mixes.json.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::Track;
use crate::radio::{primary_artist, recommend};
use crate::search::SearchCache;
use crate::stats::Play;
use crate::store::now_ms;

const DAY_MS: u64 = 86_400_000;
const MAX_DAILY: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mix {
    pub id: String,
    /// "daily" | "discover" | "repeat" | "rediscover"
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    /// Up to four covers for the tile collage.
    pub covers: Vec<String>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct MixCache {
    day: String,
    week: String,
    daily: Vec<Mix>,
    discover: Option<Mix>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForYou {
    pub daily: Vec<Mix>,
    pub discover: Option<Mix>,
    pub local: Vec<Mix>,
    /// Daily/weekly mixes are out of date (call `refresh`).
    pub stale: bool,
    /// Nothing to go on yet: no plays, likes or playlists.
    pub empty: bool,
}

fn cache_path(dir: &Path) -> PathBuf {
    dir.join("mixes.json")
}

fn day_key() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn week_key() -> String {
    let w = Local::now().iso_week();
    format!("{}-W{:02}", w.year(), w.week())
}

fn covers(tracks: &[Track]) -> Vec<String> {
    let mut seen = HashSet::new();
    tracks
        .iter()
        .filter_map(|t| t.thumbnail.clone())
        .filter(|c| seen.insert(c.clone()))
        .take(4)
        .collect()
}

/// Weighted taste: listened time with a 30-day half-life, likes, playlists.
struct Taste {
    tracks: Vec<(Track, f64)>,
    /// Everything ever played, liked or saved (Discover Weekly skips these).
    known: HashSet<String>,
}

fn taste(plays: &[Play], favorites: &[Track], playlist_tracks: &[Track]) -> Taste {
    let now = now_ms();
    let mut w: HashMap<String, (Track, f64)> = HashMap::new();
    let mut known = HashSet::new();
    for p in plays {
        known.insert(p.track.key.clone());
        let age_days = now.saturating_sub(p.t) as f64 / DAY_MS as f64;
        let weight = (p.secs / 60.0) * 0.5f64.powf(age_days / 30.0);
        let e = w.entry(p.track.key.clone()).or_insert_with(|| (p.track.clone(), 0.0));
        e.1 += weight;
    }
    for t in favorites {
        known.insert(t.key.clone());
        w.entry(t.key.clone()).or_insert_with(|| (t.clone(), 0.0)).1 += 6.0;
    }
    for t in playlist_tracks {
        known.insert(t.key.clone());
        w.entry(t.key.clone()).or_insert_with(|| (t.clone(), 0.0)).1 += 1.0;
    }
    let mut tracks: Vec<(Track, f64)> = w.into_values().filter(|(_, s)| *s > 0.0).collect();
    tracks.sort_by(|a, b| b.1.total_cmp(&a.1));
    Taste { tracks, known }
}

/// On Repeat and Forgotten favorites: no network needed.
pub fn local_mixes(plays: &[Play]) -> Vec<Mix> {
    let now = now_ms();
    let mut recent: HashMap<String, (Track, usize)> = HashMap::new();
    let mut ever: HashMap<String, (Track, usize, u64)> = HashMap::new();
    for p in plays.iter().filter(|p| p.counts()) {
        if p.t + 30 * DAY_MS >= now {
            recent.entry(p.track.key.clone()).or_insert_with(|| (p.track.clone(), 0)).1 += 1;
        }
        let e = ever.entry(p.track.key.clone()).or_insert_with(|| (p.track.clone(), 0, 0));
        e.1 += 1;
        e.2 = e.2.max(p.t);
    }
    let mut out = Vec::new();

    let mut rep: Vec<(Track, usize)> = recent.into_values().filter(|(_, n)| *n >= 2).collect();
    rep.sort_by(|a, b| b.1.cmp(&a.1));
    let rep: Vec<Track> = rep.into_iter().take(30).map(|(t, _)| t).collect();
    if rep.len() >= 5 {
        out.push(Mix { id: "repeat".into(), kind: "repeat".into(), title: "On Repeat".into(), subtitle: "Your most played songs of the last 30 days".into(), covers: covers(&rep), tracks: rep });
    }

    let mut old: Vec<(Track, usize, u64)> = ever.into_values().filter(|(_, n, last)| *n >= 3 && *last + 45 * DAY_MS < now).collect();
    old.sort_by(|a, b| b.1.cmp(&a.1));
    let old: Vec<Track> = old.into_iter().take(30).map(|(t, ..)| t).collect();
    if old.len() >= 5 {
        out.push(Mix { id: "rediscover".into(), kind: "rediscover".into(), title: "Forgotten favorites".into(), subtitle: "Songs you loved and haven't played in a while".into(), covers: covers(&old), tracks: old });
    }
    out
}

/// Stable per-day shuffle.
fn day_shuffle<T>(v: &mut [T], seed: u64) {
    let mut x = seed | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

fn seed_of(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn artist_list(tracks: &[Track], first: &str) -> String {
    let mut counts: HashMap<String, (String, usize)> = HashMap::new();
    for t in tracks {
        let name = t.artist.split([',', '&']).next().unwrap_or(&t.artist).split(" feat").next().unwrap_or("").trim().to_string();
        if !name.is_empty() {
            counts.entry(primary_artist(&t.artist)).or_insert_with(|| (name, 0)).1 += 1;
        }
    }
    let mut names: Vec<(String, (String, usize))> = counts.into_iter().collect();
    names.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    let mut out: Vec<String> = Vec::new();
    if let Some((_, (n, _))) = names.iter().find(|(k, _)| k == first) {
        out.push(n.clone());
    }
    out.extend(names.into_iter().filter(|(k, _)| k != first).map(|(_, (n, _))| n).take(3 - out.len()));
    match out.len() {
        0 => String::new(),
        1 => out[0].clone(),
        _ => format!("{} and more", out.join(", ")),
    }
}

async fn build_daily(t: &Taste, cache: &Arc<SearchCache>, day: &str) -> Vec<Mix> {
    // Candidate anchors: your top artists by weight.
    let mut by_artist: HashMap<String, (f64, Vec<(Track, f64)>)> = HashMap::new();
    for (tr, w) in &t.tracks {
        if tr.artist.is_empty() {
            continue;
        }
        let e = by_artist.entry(primary_artist(&tr.artist)).or_default();
        e.0 += w;
        e.1.push((tr.clone(), *w));
    }
    let mut anchors: Vec<(String, (f64, Vec<(Track, f64)>))> = by_artist.into_iter().collect();
    anchors.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
    anchors.truncate(MAX_DAILY * 2);

    let nonce = seed_of(day);
    let tasks: Vec<_> = anchors
        .iter()
        .map(|(artist, (_, tracks))| {
            // Best songs last: `recommend` always seeds with the last two.
            let seeds: Vec<Track> = tracks.iter().take(5).rev().map(|(t, _)| t.clone()).collect();
            let cache = cache.clone();
            let artist = artist.clone();
            tauri::async_runtime::spawn(async move { (artist, recommend(seeds, nonce, 40, &cache).await) })
        })
        .collect();
    let mut results: HashMap<String, Vec<Track>> = HashMap::new();
    for task in tasks {
        if let Ok((artist, Ok(recs))) = task.await {
            results.insert(artist, recs.into_iter().map(|r| r.track).collect());
        }
    }

    let mut mixes: Vec<Mix> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();
    let mut used: HashSet<String> = HashSet::new();
    for (artist, (_, own)) in &anchors {
        if mixes.len() >= MAX_DAILY || covered.contains(artist) {
            continue;
        }
        let Some(recs) = results.get(artist) else { continue };
        // The anchor plus the artists closest to it (the top of its radio).
        let mix_artists: HashSet<String> = recs.iter().take(15).map(|r| primary_artist(&r.artist)).chain([artist.clone()]).collect();
        // Familiar: your own songs from the anchor and artists in its orbit.
        let mut familiar: Vec<(Track, f64)> = t.tracks.iter().filter(|(tr, _)| mix_artists.contains(&primary_artist(&tr.artist)) && !used.contains(&tr.key)).cloned().collect();
        familiar.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut familiar: Vec<Track> = familiar.into_iter().take(12).map(|(t, _)| t).collect();
        if familiar.is_empty() {
            familiar = own.iter().take(3).map(|(t, _)| t.clone()).collect();
        }
        let fresh: Vec<Track> = recs.iter().filter(|r| !t.known.contains(&r.key) && !used.contains(&r.key)).take(20).cloned().collect();
        if fresh.len() < 8 {
            continue;
        }
        day_shuffle(&mut familiar, nonce ^ seed_of(artist));
        // Two new songs, then one you know.
        let mut tracks = Vec::new();
        let (mut f, mut n) = (familiar.into_iter(), fresh.into_iter());
        loop {
            let a = n.next();
            let b = n.next();
            let c = f.next();
            if a.is_none() && c.is_none() {
                break;
            }
            tracks.extend([c, a, b].into_iter().flatten());
        }
        tracks.truncate(30);
        used.extend(tracks.iter().map(|t| t.key.clone()));
        // Artists this mix already covers don't get a mix of their own.
        covered.extend(mix_artists.iter().filter(|a| *a != artist).cloned());
        covered.insert(artist.clone());
        let i = mixes.len() + 1;
        let anchor_cover = own.first().and_then(|(t, _)| t.thumbnail.clone());
        let mut cv = covers(&tracks);
        if let Some(c) = anchor_cover {
            cv.retain(|x| *x != c);
            cv.insert(0, c);
            cv.truncate(4);
        }
        mixes.push(Mix { id: format!("daily-{i}"), kind: "daily".into(), title: format!("Daily Mix {i}"), subtitle: artist_list(&tracks, artist), covers: cv, tracks });
    }
    mixes
}

async fn build_discover(t: &Taste, cache: &Arc<SearchCache>, week: &str) -> AppResult<Mix> {
    // Your top songs right now, best last (always used as seeds).
    let seeds: Vec<Track> = t.tracks.iter().take(12).rev().map(|(t, _)| t.clone()).collect();
    if seeds.is_empty() {
        return Err(AppError::EmptyResults);
    }
    let familiar_artists: HashMap<String, f64> = t.tracks.iter().fold(HashMap::new(), |mut m, (tr, w)| {
        *m.entry(primary_artist(&tr.artist)).or_default() += w;
        m
    });
    let recs = recommend(seeds, seed_of(week), 80, cache).await?;
    let fresh: Vec<Track> = recs.into_iter().map(|r| r.track).filter(|r| !t.known.contains(&r.key)).collect();
    // Mostly artists you don't know yet, one song each.
    let (mut new_artists, mut known_artists): (Vec<Track>, Vec<Track>) = fresh.into_iter().partition(|r| !familiar_artists.contains_key(&primary_artist(&r.artist)));
    let mut seen = HashSet::new();
    new_artists.retain(|r| seen.insert(primary_artist(&r.artist)));
    known_artists.retain(|r| seen.insert(primary_artist(&r.artist)));
    let mut tracks: Vec<Track> = new_artists.into_iter().take(22).collect();
    tracks.extend(known_artists.into_iter().take(30 - tracks.len()));
    if tracks.len() < 10 {
        return Err(AppError::EmptyResults);
    }
    day_shuffle(&mut tracks, seed_of(week));
    Ok(Mix { id: "discover".into(), kind: "discover".into(), title: "Discover Weekly".into(), subtitle: "New music picked for you · refreshed every Monday".into(), covers: covers(&tracks), tracks })
}

/// What's cached right now (instant).
pub fn current(dir: &Path, plays: &[Play], has_taste: bool) -> ForYou {
    let c: MixCache = crate::store::load_json(&cache_path(dir));
    let stale = has_taste && (c.day != day_key() || c.week != week_key() || c.daily.is_empty());
    ForYou { daily: c.daily, discover: c.discover, local: local_mixes(plays), stale, empty: !has_taste }
}

/// Rebuild what's out of date (or everything with `force`).
pub async fn refresh(dir: &Path, plays: Arc<Vec<Play>>, favorites: Vec<Track>, playlist_tracks: Vec<Track>, force: bool, cache: &Arc<SearchCache>) -> AppResult<ForYou> {
    let t = taste(&plays, &favorites, &playlist_tracks);
    if t.tracks.is_empty() {
        return Ok(ForYou { daily: vec![], discover: None, local: vec![], stale: false, empty: true });
    }
    let mut c: MixCache = crate::store::load_json(&cache_path(dir));
    let (day, week) = (day_key(), week_key());
    let need_daily = force || c.day != day || c.daily.is_empty();
    let need_weekly = force || c.week != week || c.discover.is_none();
    let (daily, discover) = tokio::join!(
        async {
            if need_daily {
                Some(build_daily(&t, cache, &day).await)
            } else {
                None
            }
        },
        async {
            if need_weekly {
                Some(build_discover(&t, cache, &week).await)
            } else {
                None
            }
        }
    );
    if let Some(d) = daily {
        if !d.is_empty() {
            c.daily = d;
            c.day = day;
        }
    }
    match discover {
        Some(Ok(m)) => {
            c.discover = Some(m);
            c.week = week;
        }
        Some(Err(e)) if c.daily.is_empty() => return Err(e),
        _ => {}
    }
    if let Ok(data) = serde_json::to_vec(&c) {
        let _ = crate::store::write_atomic(&cache_path(dir), &data);
    }
    Ok(ForYou { daily: c.daily, discover: c.discover, local: local_mixes(&plays), stale: false, empty: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Platform, TrackVersion};

    fn tr(title: &str, artist: &str) -> Track {
        Track::from_version(TrackVersion {
            platform: Platform::Ytmusic, url: format!("u/{title}"), id: String::new(), title: title.into(), artist: artist.into(),
            duration: Some(200.0), thumbnail: Some(format!("c/{title}")), preview: false, partial: false, artist_url: None, album: None, rank: 0, matched_for: None, isrc_match: false,
        })
    }

    #[test]
    fn local_mixes_from_plays() {
        let now = now_ms();
        let mut plays = Vec::new();
        for i in 0..6 {
            for _ in 0..3 {
                plays.push(Play { t: now - DAY_MS, secs: 180.0, est: false, platform: None, track: tr(&format!("New {i}"), "A") });
                plays.push(Play { t: now - 90 * DAY_MS, secs: 180.0, est: false, platform: None, track: tr(&format!("Old {i}"), "B") });
            }
        }
        let m = local_mixes(&plays);
        assert_eq!(m.len(), 2);
        assert!(m[0].tracks.iter().all(|t| t.title.starts_with("New")));
        assert!(m[1].tracks.iter().all(|t| t.title.starts_with("Old")));
        assert_eq!(m[0].covers.len(), 4);
    }

    #[test]
    fn taste_decays_and_counts_likes() {
        let now = now_ms();
        let plays = vec![
            Play { t: now, secs: 120.0, est: false, platform: None, track: tr("Fresh", "A") },
            Play { t: now - 120 * DAY_MS, secs: 600.0, est: false, platform: None, track: tr("Stale", "B") },
        ];
        let t = taste(&plays, &[tr("Liked", "C")], &[]);
        assert_eq!(t.tracks[0].0.title, "Liked");
        assert_eq!(t.tracks[1].0.title, "Fresh");
        assert!(t.known.contains(&tr("Stale", "B").key));
    }

    #[test]
    #[ignore]
    fn live_mixes() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let cache = Arc::new(SearchCache::default());
        let now = now_ms();
        let mut plays = Vec::new();
        let queries = [
            ("tame impala let it happen", 6), ("tame impala the less i know the better", 5), ("mac demarco chamber of reflection", 4),
            ("daft punk digital love", 6), ("justice d.a.n.c.e.", 4), ("daft punk something about us", 3),
            ("kendrick lamar money trees", 5), ("sza good days", 3),
        ];
        for (q, n) in queries {
            let r = rt.block_on(crate::search::search(q.into(), "ytmusic".into(), 3, vec![Platform::Ytmusic], cache.clone(), false, false)).unwrap();
            for k in 0..n {
                plays.push(Play { t: now - k * DAY_MS, secs: 200.0, est: false, platform: Some(Platform::Ytmusic), track: r.tracks[0].clone() });
            }
        }
        let t = taste(&plays, &[], &[]);
        let t0 = std::time::Instant::now();
        let daily = rt.block_on(build_daily(&t, &cache, "2026-10-16"));
        println!("{} daily mixes in {:?}", daily.len(), t0.elapsed());
        for m in &daily {
            println!("{} · {} ({} songs)", m.title, m.subtitle, m.tracks.len());
            for tr in m.tracks.iter().take(8) {
                println!("    {}{} — {}", if t.known.contains(&tr.key) { "* " } else { "  " }, tr.title, tr.artist);
            }
        }
        assert!(daily.len() >= 2, "expected several mixes");
        let t1 = std::time::Instant::now();
        let d = rt.block_on(build_discover(&t, &cache, "2026-W42")).unwrap();
        println!("discover in {:?}: {} songs", t1.elapsed(), d.tracks.len());
        for tr in d.tracks.iter().take(12) {
            println!("    {} — {}", tr.title, tr.artist);
        }
        assert!(d.tracks.iter().all(|x| !t.known.contains(&x.key)));
    }
}
