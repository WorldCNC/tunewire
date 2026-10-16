//! Autoplay / radio: related songs from YouTube Music's automatic mix
//! ("RDAMVM" + video id), fetched with yt-dlp like any playlist.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track};
use crate::search::{self, SearchCache};
use crate::ytdlp;

/// A YouTube video id for the seed song (searching YouTube Music if needed).
async fn seed_id(track: &Track, cache: &Arc<SearchCache>) -> AppResult<String> {
    let known = track
        .versions
        .iter()
        .find(|v| matches!(v.platform, Platform::Ytmusic | Platform::Youtube) && v.id.len() == 11)
        .map(|v| v.id.clone());
    if let Some(id) = known {
        return Ok(id);
    }
    let pref = [Platform::Ytmusic, Platform::Youtube];
    let v = search::resolve_equivalent(&track.artist, &track.title, track.duration, &pref, &[Platform::Soundcloud], cache).await?;
    if v.id.len() == 11 {
        Ok(v.id)
    } else {
        Err(AppError::Unavailable("no YouTube Music match to start a radio from".into()))
    }
}

fn parse_duration(s: &str) -> Option<f64> {
    s.split(':').try_fold(0.0, |acc, p| p.trim().parse::<f64>().ok().map(|n| acc * 60.0 + n))
}

fn runs(v: &Value) -> Vec<String> {
    v.get("runs").and_then(Value::as_array).map(|r| r.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).map(str::to_string).collect()).unwrap_or_default()
}

/// YouTube Music's own radio for a song ("Up next" with a mix playlist):
/// real song metadata (artist, album, duration, art) in one request.
fn ytm_radio_blocking(id: &str) -> AppResult<Vec<crate::models::TrackVersion>> {
    let body = serde_json::json!({
        "videoId": id, "playlistId": format!("RDAMVM{id}"), "isAudioOnly": true,
        "context": { "client": { "clientName": "WEB_REMIX", "clientVersion": "1.20250101.01.00", "hl": "en", "gl": "US" } }
    });
    let mut r = crate::http::agent()
        .post("https://music.youtube.com/youtubei/v1/next?prettyPrint=false")
        .header("Content-Type", "application/json")
        .header("Origin", "https://music.youtube.com")
        .header("User-Agent", crate::http::USER_AGENT)
        .send(body.to_string())
        .map_err(|e| AppError::Network(e.to_string()))?;
    if !r.status().is_success() {
        return Err(AppError::Network(format!("YouTube Music radio returned HTTP {}", r.status().as_u16())));
    }
    let text = r.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
    let v: Value = serde_json::from_str(&text)?;
    let items = v
        .pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer/contents")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Extraction("YouTube Music radio format changed".into()))?;
    Ok(items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let p = it.get("playlistPanelVideoRenderer")?;
            let vid = p.get("videoId").and_then(Value::as_str)?.to_string();
            let title = runs(p.get("title")?).concat();
            // "Artist • Album • Year" (artists can span several runs).
            let byline = runs(p.get("longBylineText")?).concat();
            let mut parts = byline.split(" • ");
            let artist = parts.next().unwrap_or("").trim().to_string();
            let album = parts.next().map(|a| a.trim().to_string()).filter(|a| !a.is_empty() && a.parse::<u32>().is_err() && !a.contains("views"));
            let duration = p.get("lengthText").map(|l| runs(l).concat()).and_then(|d| parse_duration(&d));
            let thumbnail = p.pointer("/thumbnail/thumbnails").and_then(Value::as_array).and_then(|a| a.last()).and_then(|t| t.get("url").and_then(Value::as_str)).map(str::to_string);
            (!title.is_empty()).then(|| crate::models::TrackVersion {
                platform: Platform::Ytmusic,
                url: format!("https://music.youtube.com/watch?v={vid}"),
                id: vid,
                title,
                artist,
                duration,
                thumbnail,
                preview: false,
                partial: false,
                artist_url: None,
                album,
                rank: i,
                matched_for: None,
                isrc_match: false,
            })
        })
        .collect())
}

/// Up to `limit` songs related to `track`, skipping keys in `exclude`.
pub async fn related(track: &Track, exclude: &HashSet<String>, limit: usize, cache: &Arc<SearchCache>) -> AppResult<Vec<Track>> {
    let id = seed_id(track, cache).await?;
    let id2 = id.clone();
    if let Ok(Ok(versions)) = tauri::async_runtime::spawn_blocking(move || ytm_radio_blocking(&id2)).await {
        let mut seen: HashSet<String> = exclude.clone();
        seen.insert(track.key.clone());
        let out: Vec<Track> = versions
            .into_iter()
            .filter(|v| v.id != id && v.duration.map(|d| d <= 15.0 * 60.0).unwrap_or(true))
            .map(|v| search::build_track(vec![v], 0.0))
            .filter(|t| seen.insert(t.key.clone()))
            .take(limit)
            .collect();
        if !out.is_empty() {
            return Ok(out);
        }
    }
    // Fallback: the same mix through yt-dlp (less metadata).
    let url = format!("https://www.youtube.com/watch?v={id}&list=RDAMVM{id}");
    let json = tokio::time::timeout(Duration::from_secs(60), ytdlp::flat(&url, Some(limit + 10)))
        .await
        .map_err(|_| AppError::Timeout("radio took too long".into()))??;
    let entries = json.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut seen: HashSet<String> = exclude.clone();
    seen.insert(track.key.clone());
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if e.get("id").and_then(Value::as_str) == Some(id.as_str()) {
            continue;
        }
        let Some(v) = search::parse_entry(e, Platform::Ytmusic, i) else { continue };
        // Skip long mixes/compilations.
        if v.duration.map(|d| d > 15.0 * 60.0).unwrap_or(false) {
            continue;
        }
        let t = search::build_track(vec![v], 0.0);
        if seen.insert(t.key.clone()) {
            out.push(t);
        }
        if out.len() >= limit {
            break;
        }
    }
    if out.is_empty() {
        return Err(AppError::EmptyResults);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Playlist recommendations: several seeds' mixes, scored together
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;

static MIXES: Mutex<Option<HashMap<String, (Instant, Vec<Track>)>>> = Mutex::new(None);
const MIX_TTL_SECS: u64 = 30 * 60;

/// A seed's whole mix (cached), in YouTube Music's order.
async fn mix(seed: &Track, cache: &Arc<SearchCache>) -> AppResult<Vec<Track>> {
    if let Some((t, v)) = MIXES.lock().unwrap().as_ref().and_then(|m| m.get(&seed.key).cloned()) {
        if t.elapsed().as_secs() < MIX_TTL_SECS {
            return Ok(v);
        }
    }
    let v = related(seed, &HashSet::new(), 30, cache).await?;
    let mut guard = MIXES.lock().unwrap();
    let m = guard.get_or_insert_with(HashMap::new);
    if m.len() > 300 {
        m.clear();
    }
    m.insert(seed.key.clone(), (Instant::now(), v.clone()));
    Ok(v)
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub track: Track,
    /// Title of the seed song it's most related to.
    pub because: String,
    /// How many of the seeds' mixes it appeared in.
    pub seeds: usize,
}

/// Small deterministic RNG so "Refresh" (a new nonce) picks other seeds.
fn shuffle<T>(v: &mut [T], mut x: u64) {
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

pub(crate) fn primary_artist(a: &str) -> String {
    let first = a.split([',', '&']).next().unwrap_or(a);
    let first = first.split(" feat").next().unwrap_or(first).split(" ft.").next().unwrap_or(first);
    crate::search::norm_text(first)
}

/// Seeds: the latest additions (the playlist's current direction) plus a
/// random spread of the rest.
fn pick_seeds(tracks: &[Track], nonce: u64, n: usize) -> Vec<Track> {
    let mut seen = HashSet::new();
    let unique: Vec<&Track> = tracks.iter().filter(|t| seen.insert(t.key.clone())).collect();
    let mut seeds: Vec<Track> = unique.iter().rev().take(2).map(|t| (*t).clone()).collect();
    let mut rest: Vec<&Track> = unique.iter().rev().skip(2).copied().collect();
    shuffle(&mut rest, nonce.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    // Prefer seeds by different artists so the mixes cover the playlist.
    let mut artists: HashSet<String> = seeds.iter().map(|t| primary_artist(&t.artist)).collect();
    for t in rest.iter().filter(|t| artists.insert(primary_artist(&t.artist))).chain(rest.iter()) {
        if seeds.len() >= n {
            break;
        }
        if !seeds.iter().any(|s| s.key == t.key) {
            seeds.push((*t).clone());
        }
    }
    seeds
}

pub async fn recommend(tracks: Vec<Track>, nonce: u64, count: usize, cache: &Arc<SearchCache>) -> AppResult<Vec<Recommendation>> {
    if tracks.is_empty() {
        return Err(AppError::EmptyResults);
    }
    let in_playlist: HashSet<String> = tracks.iter().map(|t| t.key.clone()).collect();
    let playlist_artists: HashMap<String, usize> = tracks.iter().fold(HashMap::new(), |mut m, t| {
        *m.entry(primary_artist(&t.artist)).or_default() += 1;
        m
    });
    let seeds = pick_seeds(&tracks, nonce, 5);
    let tasks: Vec<_> = seeds
        .iter()
        .cloned()
        .map(|s| {
            let cache = cache.clone();
            tauri::async_runtime::spawn(async move { (s.clone(), mix(&s, &cache).await) })
        })
        .collect();

    struct Cand {
        track: Track,
        score: f64,
        seeds: usize,
        best: (f64, String),
    }
    let mut cands: HashMap<String, Cand> = HashMap::new();
    let mut last_err = None;
    for t in tasks {
        let Ok((seed, res)) = t.await else { continue };
        let list = match res {
            Ok(l) => l,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let seed_artist = primary_artist(&seed.artist);
        let n = list.len().max(1) as f64;
        for (rank, track) in list.into_iter().enumerate() {
            if in_playlist.contains(&track.key) {
                continue;
            }
            // Earlier in the mix = more closely related.
            let mut s = 1.0 - (rank as f64 / n) * 0.7;
            let artist = primary_artist(&track.artist);
            if artist == seed_artist {
                s -= 0.3; // mixes lean on the seed's own artist; favour discovery
            }
            let c = cands.entry(track.key.clone()).or_insert_with(|| Cand { track, score: 0.0, seeds: 0, best: (f64::MIN, String::new()) });
            c.score += s;
            c.seeds += 1;
            if s > c.best.0 {
                c.best = (s, seed.title.clone());
            }
        }
    }
    if cands.is_empty() {
        return Err(last_err.unwrap_or(AppError::EmptyResults));
    }
    let mut ranked: Vec<Cand> = cands.into_values().collect();
    for c in ranked.iter_mut() {
        // Showing up for several seeds = fits the playlist as a whole.
        c.score += 0.6 * (c.seeds.saturating_sub(1)) as f64;
        // Familiar artists help a little (capped, so they don't take over).
        if let Some(n) = playlist_artists.get(&primary_artist(&c.track.artist)) {
            c.score += 0.12 * (*n as f64).min(3.0);
        }
    }
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    // At most two songs per artist.
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    let out: Vec<Recommendation> = ranked
        .into_iter()
        .filter(|c| {
            let n = per_artist.entry(primary_artist(&c.track.artist)).or_default();
            *n += 1;
            *n <= 2
        })
        .take(count)
        .map(|c| Recommendation { track: c.track, because: c.best.1, seeds: c.seeds })
        .collect();
    Ok(out)
}

#[cfg(test)]
mod rec_tests {
    use super::*;
    use crate::models::{Platform, TrackVersion};

    fn t(title: &str, artist: &str) -> Track {
        crate::search::build_track(
            vec![TrackVersion {
                platform: Platform::Ytmusic, url: format!("u/{title}"), id: String::new(), title: title.into(), artist: artist.into(),
                duration: Some(200.0), thumbnail: None, preview: false, partial: false, artist_url: None, album: None, rank: 0, matched_for: None, isrc_match: false,
            }],
            0.0,
        )
    }

    #[test]
    fn seeds_cover_recent_and_varied_artists() {
        let list: Vec<Track> = (0..20).map(|i| t(&format!("Song {i}"), if i < 15 { "Same" } else { ["A", "B", "C", "D", "E"][i - 15] })).collect();
        let seeds = pick_seeds(&list, 1, 5);
        assert_eq!(seeds.len(), 5);
        assert_eq!(seeds[0].title, "Song 19", "latest addition first");
        let artists: HashSet<String> = seeds.iter().map(|s| s.artist.clone()).collect();
        assert!(artists.len() >= 4, "seeds spread over artists: {artists:?}");
        assert_ne!(pick_seeds(&list, 1, 5).iter().map(|s| s.key.clone()).collect::<Vec<_>>(), pick_seeds(&(0..40).map(|i| t(&format!("X{i}"), &format!("Art{i}"))).collect::<Vec<_>>(), 2, 5).iter().map(|s| s.key.clone()).collect::<Vec<_>>());
    }

    #[test]
    #[ignore]
    fn live_recommendations() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let cache = Arc::new(SearchCache::default());
        let mut pl = Vec::new();
        for q in ["tame impala let it happen", "hiatus kaiyote nakamarra", "khruangbin maria tambien", "mac demarco chamber of reflection"] {
            let r = rt.block_on(crate::search::search(q.into(), "ytmusic".into(), 3, vec![Platform::Ytmusic], cache.clone(), false, false)).unwrap();
            pl.push(r.tracks[0].clone());
        }
        let t0 = std::time::Instant::now();
        let recs = rt.block_on(recommend(pl.clone(), 7, 20, &cache)).unwrap();
        println!("{} recommendations in {:?}", recs.len(), t0.elapsed());
        for r in &recs {
            println!("  [{}] {} — {}   (because {})", r.seeds, r.track.title, r.track.artist, r.because);
        }
        assert!(recs.len() >= 10);
        assert!(recs.iter().all(|r| !pl.iter().any(|p| p.key == r.track.key)));
        let t1 = std::time::Instant::now();
        let again = rt.block_on(recommend(pl, 7, 20, &cache)).unwrap();
        println!("cached: {:?}", t1.elapsed());
        assert_eq!(again.len(), recs.len());
    }
}
