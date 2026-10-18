//! SoundCloud's own API (api-v2, what soundcloud.com uses): search, sets and
//! stream URLs in one or two quick requests, where yt-dlp takes seconds.
//! The public client id is read from the web app's scripts once and cached
//! on disk; a 401/403 means it rotated, so it's fetched again.
//! Everything falls back to yt-dlp if this fails.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track, TrackVersion};
use crate::ytdlp::StreamInfo;

const API: &str = "https://api-v2.soundcloud.com";

static CLIENT_ID: Mutex<Option<String>> = Mutex::new(None);
static CACHE_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn init(dir: &std::path::Path) {
    let path = dir.join("soundcloud_client_id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim().to_string();
        if id.len() == 32 {
            *CLIENT_ID.lock().unwrap() = Some(id);
        }
    }
    *CACHE_FILE.lock().unwrap() = Some(path);
}

fn get_text(url: &str) -> AppResult<(u16, String)> {
    let resp = crate::http::agent().get(url).header("User-Agent", crate::http::USER_AGENT).call();
    match resp {
        Ok(mut r) => {
            let code = r.status().as_u16();
            let text = r.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
            Ok((code, text))
        }
        Err(ureq::Error::StatusCode(c)) => Ok((c, String::new())),
        Err(e) => Err(AppError::Network(e.to_string())),
    }
}

/// Scrape the client id from soundcloud.com's app scripts.
fn fetch_client_id() -> AppResult<String> {
    let (_, html) = get_text("https://soundcloud.com/")?;
    let scripts: Vec<&str> = html
        .match_indices("src=\"https://a-v2.sndcdn.com/assets/")
        .filter_map(|(i, _)| {
            let start = i + 5;
            let end = html[start..].find('"')? + start;
            Some(&html[start..end])
        })
        .collect();
    for s in scripts.iter().rev() {
        let (_, js) = get_text(s)?;
        if let Some(i) = js.find("client_id:\"").or_else(|| js.find("client_id=\"")) {
            let id: String = js[i + 11..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if id.len() == 32 {
                if let Some(p) = CACHE_FILE.lock().unwrap().as_ref() {
                    let _ = std::fs::write(p, &id);
                }
                return Ok(id);
            }
        }
    }
    Err(AppError::Extraction("SoundCloud client id not found".into()))
}

fn client_id(force: bool) -> AppResult<String> {
    if !force {
        if let Some(id) = CLIENT_ID.lock().unwrap().clone() {
            return Ok(id);
        }
    }
    let id = fetch_client_id()?;
    *CLIENT_ID.lock().unwrap() = Some(id.clone());
    Ok(id)
}

/// GET an api-v2 path (with the client id), retrying once with a fresh id.
fn api(path_and_query: &str) -> AppResult<Value> {
    for attempt in 0..2 {
        let id = client_id(attempt > 0)?;
        let sep = if path_and_query.contains('?') { '&' } else { '?' };
        let url = if path_and_query.starts_with("http") { format!("{path_and_query}{sep}client_id={id}") } else { format!("{API}{path_and_query}{sep}client_id={id}") };
        let (code, text) = get_text(&url)?;
        match code {
            200 => return Ok(serde_json::from_str(&text)?),
            401 | 403 if attempt == 0 => continue,
            404 => return Err(AppError::NotFound("not on SoundCloud".into())),
            c => return Err(AppError::Network(format!("SoundCloud returned HTTP {c}"))),
        }
    }
    Err(AppError::Network("SoundCloud refused the request".into()))
}

fn enc(s: &str) -> String {
    crate::search::percent_encode(s)
}

/// Warm up (client id) so the first search doesn't pay for it.
pub fn warm() {
    let _ = client_id(false);
}

fn artwork(t: &Value) -> Option<String> {
    t.get("artwork_url").and_then(Value::as_str).or_else(|| t.pointer("/user/avatar_url").and_then(Value::as_str)).map(|u| u.replace("-large.", "-t500x500."))
}

/// A track object → our version (same shape the yt-dlp path produces).
fn version(t: &Value, rank: usize) -> Option<TrackVersion> {
    let url = t.get("permalink_url").and_then(Value::as_str)?.to_string();
    let raw_title = t.get("title").and_then(Value::as_str)?;
    let uploader = t.pointer("/publisher_metadata/artist").and_then(Value::as_str).filter(|a| !a.is_empty()).or_else(|| t.pointer("/user/username").and_then(Value::as_str)).unwrap_or("");
    let (artist, title) = if t.pointer("/publisher_metadata/artist").and_then(Value::as_str).is_some_and(|a| !a.is_empty()) {
        (crate::search::clean_uploader(uploader), raw_title.to_string())
    } else {
        crate::search::split_artist_title(raw_title, uploader)
    };
    let duration = t.get("duration").and_then(Value::as_f64).map(|ms| ms / 1000.0);
    // Go+ tracks only stream a 30 s preview ("SNIP").
    let preview = t.get("policy").and_then(Value::as_str) == Some("SNIP") || duration.is_some_and(|d| (d - 30.0).abs() < 0.6);
    Some(TrackVersion {
        platform: Platform::Soundcloud,
        url,
        id: t.get("id").map(|v| v.to_string().trim_matches('"').to_string()).unwrap_or_default(),
        title,
        artist,
        duration: if preview { t.get("full_duration").and_then(Value::as_f64).map(|ms| ms / 1000.0).or(duration) } else { duration },
        thumbnail: artwork(t),
        preview,
        partial: false,
        artist_url: t.pointer("/user/permalink_url").and_then(Value::as_str).map(str::to_string),
        album: t.pointer("/publisher_metadata/album_title").and_then(Value::as_str).filter(|a| !a.is_empty()).map(str::to_string),
        rank,
        matched_for: None,
        isrc_match: false,
    })
}

pub async fn search(query: &str, limit: usize) -> AppResult<Vec<TrackVersion>> {
    let q = query.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let v = api(&format!("/search/tracks?q={}&limit={}", enc(&q), limit.clamp(1, 50)))?;
        Ok(v.get("collection").and_then(Value::as_array).map(|a| a.iter().enumerate().filter_map(|(i, t)| version(t, i)).collect()).unwrap_or_default())
    })
    .await
    .map_err(|e| AppError::Network(e.to_string()))?
}

/// The playable stream of a track page: AAC over HLS like yt-dlp picks,
/// else progressive MP3, else whatever there is.
pub async fn stream(url: &str) -> AppResult<StreamInfo> {
    let u = url.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let t = api(&format!("/resolve?url={}", enc(&u)))?;
        if t.get("kind").and_then(Value::as_str) != Some("track") {
            return Err(AppError::Invalid("not a SoundCloud track".into()));
        }
        let codings = t.pointer("/media/transcodings").and_then(Value::as_array).cloned().unwrap_or_default();
        let score = |c: &Value| {
            let proto = c.pointer("/format/protocol").and_then(Value::as_str).unwrap_or("");
            let mime = c.pointer("/format/mime_type").and_then(Value::as_str).unwrap_or("");
            let preset = c.get("preset").and_then(Value::as_str).unwrap_or("");
            match (proto, mime.contains("mp4a"), mime.contains("mpeg")) {
                ("hls", true, _) => usize::from(!preset.contains("160")),
                ("progressive", _, true) => 2,
                ("hls", _, true) => 3,
                _ => 9,
            }
        };
        let pick = codings
            .iter()
            .filter(|c| score(c) < 9 && !c.get("snipped").and_then(Value::as_bool).unwrap_or(false))
            .min_by_key(|c| score(c))
            .or_else(|| codings.iter().min_by_key(|c| score(c)))
            .ok_or_else(|| AppError::Unavailable("no stream on SoundCloud".into()))?;
        let auth = t.get("track_authorization").and_then(Value::as_str).unwrap_or("");
        let base = pick.get("url").and_then(Value::as_str).ok_or_else(|| AppError::Extraction("SoundCloud stream format changed".into()))?;
        let s = api(&format!("{base}?track_authorization={auth}"))?;
        let stream_url = s.get("url").and_then(Value::as_str).ok_or_else(|| AppError::Extraction("SoundCloud returned no stream URL".into()))?.to_string();
        let proto = pick.pointer("/format/protocol").and_then(Value::as_str).unwrap_or("");
        let mime = pick.pointer("/format/mime_type").and_then(Value::as_str).unwrap_or("");
        let (ext, acodec) = if mime.contains("mp4a") {
            ("m4a", "mp4a.40.2")
        } else if mime.contains("mpeg") {
            ("mp3", "mp3")
        } else {
            ("opus", "opus")
        };
        let v = version(&t, 0);
        Ok(StreamInfo {
            url: stream_url,
            ext: Some(ext.into()),
            acodec: Some(acodec.into()),
            protocol: Some(if proto == "hls" { "m3u8_native".into() } else { "https".into() }),
            duration: t.get("full_duration").or_else(|| t.get("duration")).and_then(Value::as_f64).map(|ms| ms / 1000.0),
            title: v.as_ref().map(|v| v.title.clone()),
            artist: v.as_ref().map(|v| v.artist.clone()),
            thumbnail: artwork(&t),
            filesize: None,
            headers: Default::default(),
        })
    })
    .await
    .map_err(|e| AppError::Network(e.to_string()))?
}

/// A set (playlist/album) or a user's tracks page, with full metadata.
pub async fn playlist(url: &str) -> AppResult<(String, Vec<Track>)> {
    let u = url.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let p = api(&format!("/resolve?url={}", enc(&u)))?;
        if p.get("kind").and_then(Value::as_str) != Some("playlist") {
            return Err(AppError::Invalid("not a SoundCloud set".into()));
        }
        let title = p.get("title").and_then(Value::as_str).unwrap_or("SoundCloud set").to_string();
        let mut items: Vec<Value> = p.get("tracks").and_then(Value::as_array).cloned().unwrap_or_default();
        // Only the first few come complete; fetch the rest by id, 50 at a time.
        let missing: Vec<String> = items.iter().filter(|t| t.get("title").is_none()).filter_map(|t| t.get("id").map(|v| v.to_string())).collect();
        let mut full: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
        for chunk in missing.chunks(50) {
            let got = api(&format!("/tracks?ids={}", chunk.join("%2C")))?;
            for t in got.as_array().into_iter().flatten() {
                if let Some(id) = t.get("id") {
                    full.insert(id.to_string(), t.clone());
                }
            }
        }
        for t in items.iter_mut() {
            if t.get("title").is_none() {
                if let Some(f) = t.get("id").and_then(|id| full.get(&id.to_string())) {
                    *t = f.clone();
                }
            }
        }
        let tracks = items.iter().enumerate().filter_map(|(i, t)| version(t, i)).map(Track::from_version).collect();
        Ok((title, tracks))
    })
    .await
    .map_err(|e| AppError::Network(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn live_soundcloud() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let t0 = std::time::Instant::now();
        warm();
        println!("client id in {:?}", t0.elapsed());
        let t1 = std::time::Instant::now();
        let s = rt.block_on(search("odesza say my name", 10)).unwrap();
        println!("search {:?}: {}", t1.elapsed(), s.iter().take(3).map(|v| format!("{} — {} ({:?}s, preview {})", v.artist, v.title, v.duration, v.preview)).collect::<Vec<_>>().join(" | "));
        assert!(!s.is_empty());
        let t2 = std::time::Instant::now();
        let info = rt.block_on(stream("https://soundcloud.com/daft-punk-id/daft-punk-one-more-time")).unwrap();
        println!("stream {:?}: {:?} {:?} {:?}", t2.elapsed(), info.ext, info.protocol, info.duration);
        let opened = crate::stream::open(&info).unwrap();
        let mut d = crate::audio::prepare(opened, info.duration, 1).unwrap();
        assert!(d.decoder.next().is_some());
        let t3 = std::time::Instant::now();
        let (title, tracks) = rt.block_on(playlist("https://soundcloud.com/odesza/sets/a-moment-apart-1")).unwrap();
        println!("set {:?}: {title} {} tracks, last: {} — {}", t3.elapsed(), tracks.len(), tracks.last().unwrap().artist, tracks.last().unwrap().title);
        assert!(tracks.len() > 10 && tracks.iter().all(|t| t.duration.is_some()));
    }
}
