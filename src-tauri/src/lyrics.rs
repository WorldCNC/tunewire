//! Lyrics from LRCLIB (https://lrclib.net): free, no API key, time-synced
//! LRC when available. One of the few requests that doesn't go through yt-dlp.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
pub struct Lyrics {
    pub synced: Option<String>,
    pub plain: Option<String>,
    pub instrumental: bool,
    pub source: &'static str,
}

fn cache() -> &'static Mutex<HashMap<String, Option<Lyrics>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<Lyrics>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

fn parse(v: &Value) -> Option<Lyrics> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).filter(|x| !x.trim().is_empty());
    let l = Lyrics { synced: s("syncedLyrics"), plain: s("plainLyrics"), instrumental: v.get("instrumental").and_then(Value::as_bool).unwrap_or(false), source: "LRCLIB" };
    (l.synced.is_some() || l.plain.is_some() || l.instrumental).then_some(l)
}

fn fetch(artist: &str, title: &str, album: Option<&str>, duration: Option<f64>) -> AppResult<Option<Lyrics>> {
    let get = |url: String| -> AppResult<Option<Value>> {
        let mut r = crate::http::agent().get(&url).header("User-Agent", "Tunewire (desktop music player)").call().map_err(|e| AppError::Network(e.to_string()))?;
        if r.status().as_u16() == 404 {
            return Ok(None);
        }
        if !r.status().is_success() {
            return Err(AppError::Network(format!("LRCLIB returned {}", r.status())));
        }
        let text = r.body_mut().read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
        serde_json::from_str::<Value>(&text).map(Some).map_err(|e| AppError::Network(e.to_string()))
    };
    // Exact match first (needs duration to be within ±2 s on LRCLIB's side).
    let mut url = format!("https://lrclib.net/api/get?artist_name={}&track_name={}", enc(artist), enc(title));
    if let Some(al) = album {
        url.push_str(&format!("&album_name={}", enc(al)));
    }
    if let Some(d) = duration {
        url.push_str(&format!("&duration={}", d.round() as i64));
    }
    if let Some(v) = get(url)? {
        if let Some(l) = parse(&v) {
            return Ok(Some(l));
        }
    }
    // Fuzzy search, preferring synced lyrics with a close duration.
    let url = format!("https://lrclib.net/api/search?track_name={}&artist_name={}", enc(title), enc(artist));
    let Some(Value::Array(items)) = get(url)? else {
        return Ok(None);
    };
    let close = |it: &Value| match (duration, it.get("duration").and_then(Value::as_f64)) {
        (Some(d), Some(x)) => (d - x).abs() <= 4.0,
        _ => true,
    };
    let best = items.iter().filter(|it| close(it)).max_by_key(|it| it.get("syncedLyrics").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false) as u8);
    Ok(best.and_then(parse))
}

pub async fn get(artist: String, title: String, album: Option<String>, duration: Option<f64>) -> AppResult<Option<Lyrics>> {
    let title = crate::search::strip_noise(&title);
    let artist = artist.split([',', '&']).next().unwrap_or(&artist).trim().to_string();
    let key = format!("{}::{}", crate::search::norm_text(&artist), crate::search::norm_text(&title));
    if let Some(hit) = cache().lock().unwrap().get(&key) {
        return Ok(hit.clone());
    }
    let res = tauri::async_runtime::spawn_blocking(move || fetch(&artist, &title, album.as_deref(), duration)).await.map_err(|e| AppError::Network(e.to_string()))??;
    cache().lock().unwrap().insert(key, res.clone());
    Ok(res)
}
