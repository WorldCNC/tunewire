//! Scrobbling to Last.fm and ListenBrainz.
//!
//! A play is scrobbled when it counts (see `stats::Play::counts`: 30 s, or
//! half of a short song) and the song is longer than 30 s, as both services
//! ask. "Now playing" is sent when a song starts. Scrobbles that can't be
//! sent (offline, service down) wait in scrobbles.json and are retried.
//!
//! Credentials live in the OS keychain: the Last.fm session key and the
//! ListenBrainz user token. The Last.fm API key/secret are built in
//! (TUNEWIRE_LASTFM_KEY / TUNEWIRE_LASTFM_SECRET) or the user's own.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::Track;
use crate::stats::Play;

const LASTFM_API: &str = "https://ws.audioscrobbler.com/2.0/";
const LB_API: &str = "https://api.listenbrainz.org/1";
pub const LASTFM_SESSION: &str = "lastfm_session";
pub const LASTFM_OWN_SECRET: &str = "lastfm_api_secret";
pub const LB_TOKEN: &str = "listenbrainz_token";
const QUEUE_MAX: usize = 2000;

#[derive(Debug, Clone, Default)]
struct Config {
    lastfm: bool,
    listenbrainz: bool,
    /// The user's own Last.fm API key (secret in the keychain); empty = built in.
    lastfm_key: String,
}

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static QUEUE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Scrobble {
    pub artist: String,
    pub track: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration: Option<u32>,
    /// Unix seconds the listen started.
    pub timestamp: u64,
    /// "lastfm" | "listenbrainz"
    pub service: String,
}

pub fn init(dir: &std::path::Path) {
    *DIR.lock().unwrap() = Some(dir.to_path_buf());
    // Anything left from last time.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(10));
        flush_queue();
    });
}

pub fn configure(lastfm: bool, listenbrainz: bool, lastfm_key: &str) {
    *CONFIG.lock().unwrap() = Some(Config { lastfm, listenbrainz, lastfm_key: lastfm_key.trim().to_string() });
}

fn config() -> Config {
    CONFIG.lock().unwrap().clone().unwrap_or_default()
}

/// API key + secret: the user's own pair, else the one built in.
fn lastfm_app(c: &Config) -> Option<(String, String)> {
    if !c.lastfm_key.is_empty() {
        return crate::secrets::get(LASTFM_OWN_SECRET).map(|s| (c.lastfm_key.clone(), s));
    }
    match (option_env!("TUNEWIRE_LASTFM_KEY"), option_env!("TUNEWIRE_LASTFM_SECRET")) {
        (Some(k), Some(s)) if !k.is_empty() && !s.is_empty() => Some((k.to_string(), s.to_string())),
        _ => None,
    }
}

/// Whether Last.fm can be used at all (an API key is available).
pub fn lastfm_available(own_key: &str) -> bool {
    lastfm_app(&Config { lastfm_key: own_key.trim().to_string(), ..Default::default() }).is_some()
}

/// Last.fm's request signature: md5 of the sorted name+value pairs + secret.
pub fn lastfm_sig(params: &BTreeMap<String, String>, secret: &str) -> String {
    let mut s = String::new();
    for (k, v) in params {
        if k != "format" && k != "callback" {
            s.push_str(k);
            s.push_str(v);
        }
    }
    s.push_str(secret);
    format!("{:x}", Md5::digest(s.as_bytes()))
}

fn form_encode(params: &BTreeMap<String, String>) -> String {
    params.iter().map(|(k, v)| format!("{}={}", crate::search::percent_encode(k), crate::search::percent_encode(v))).collect::<Vec<_>>().join("&")
}

fn lastfm_call(method: &str, mut params: BTreeMap<String, String>, post: bool) -> AppResult<Value> {
    let c = config();
    let (key, secret) = lastfm_app(&c).ok_or_else(|| AppError::Unavailable("Last.fm isn't set up in this build".into()))?;
    params.insert("method".into(), method.into());
    params.insert("api_key".into(), key);
    let sig = lastfm_sig(&params, &secret);
    params.insert("api_sig".into(), sig);
    params.insert("format".into(), "json".into());
    let agent = crate::http::agent();
    let res = if post {
        agent.post(LASTFM_API).header("Content-Type", "application/x-www-form-urlencoded").send(form_encode(&params))
    } else {
        agent.get(&format!("{LASTFM_API}?{}", form_encode(&params))).call()
    };
    let mut r = res.map_err(|e| AppError::Network(e.to_string()))?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if let Some(code) = v.get("error").and_then(Value::as_i64) {
        let msg = v.get("message").and_then(Value::as_str).unwrap_or("Last.fm error").to_string();
        // 9: invalid session key (revoked in Last.fm settings).
        if code == 9 {
            let _ = crate::secrets::set(LASTFM_SESSION, "");
        }
        return Err(if matches!(code, 11 | 16 | 29) { AppError::Network(msg) } else { AppError::Unavailable(format!("Last.fm: {msg}")) });
    }
    if status >= 500 {
        return Err(AppError::Network(format!("Last.fm returned HTTP {status}")));
    }
    Ok(v)
}

/// Step 1: a request token and the page where the user approves it.
pub fn lastfm_begin() -> AppResult<(String, String)> {
    let v = lastfm_call("auth.getToken", BTreeMap::new(), false)?;
    let token = v.get("token").and_then(Value::as_str).ok_or_else(|| AppError::Unavailable("Last.fm gave no token".into()))?.to_string();
    let key = lastfm_app(&config()).map(|(k, _)| k).unwrap_or_default();
    Ok((token.clone(), format!("https://www.last.fm/api/auth/?api_key={key}&token={token}")))
}

/// Step 2 (after the user approved): the session key, stored in the keychain.
pub fn lastfm_finish(token: &str) -> AppResult<String> {
    let mut p = BTreeMap::new();
    p.insert("token".to_string(), token.to_string());
    let v = lastfm_call("auth.getSession", p, false)?;
    let s = v.get("session").ok_or_else(|| AppError::Unavailable("not approved yet".into()))?;
    let key = s.get("key").and_then(Value::as_str).ok_or_else(|| AppError::Unavailable("not approved yet".into()))?;
    let name = s.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    crate::secrets::set(LASTFM_SESSION, key)?;
    Ok(name)
}

/// Check a ListenBrainz token and return the user name; stores it on success.
pub fn listenbrainz_connect(token: &str) -> AppResult<String> {
    let mut r = crate::http::agent()
        .get(&format!("{LB_API}/validate-token"))
        .header("Authorization", &format!("Token {}", token.trim()))
        .call()
        .map_err(|e| AppError::Network(e.to_string()))?;
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().unwrap_or_default()).unwrap_or(Value::Null);
    if v.get("valid").and_then(Value::as_bool) != Some(true) {
        return Err(AppError::Invalid("that ListenBrainz token isn't valid".into()));
    }
    crate::secrets::set(LB_TOKEN, token.trim())?;
    Ok(v.get("user_name").and_then(Value::as_str).unwrap_or("").to_string())
}

fn lb_submit(kind: &str, items: &[&Scrobble]) -> AppResult<()> {
    let token = crate::secrets::get(LB_TOKEN).ok_or_else(|| AppError::Unavailable("not connected to ListenBrainz".into()))?;
    let payload: Vec<Value> = items
        .iter()
        .map(|s| {
            let mut info = serde_json::json!({ "media_player": "Tunewire", "submission_client": "Tunewire", "submission_client_version": env!("CARGO_PKG_VERSION") });
            if let Some(d) = s.duration {
                info["duration_ms"] = (d as u64 * 1000).into();
            }
            let mut meta = serde_json::json!({ "artist_name": s.artist, "track_name": s.track, "additional_info": info });
            if !s.album.is_empty() {
                meta["release_name"] = s.album.clone().into();
            }
            if kind == "playing_now" {
                serde_json::json!({ "track_metadata": meta })
            } else {
                serde_json::json!({ "listened_at": s.timestamp, "track_metadata": meta })
            }
        })
        .collect();
    let body = serde_json::json!({ "listen_type": kind, "payload": payload });
    let r = crate::http::agent()
        .post(&format!("{LB_API}/submit-listens"))
        .header("Authorization", &format!("Token {token}"))
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| AppError::Network(e.to_string()))?;
    match r.status().as_u16() {
        200..=299 => Ok(()),
        401 => Err(AppError::Unavailable("ListenBrainz token was rejected".into())),
        400 => Err(AppError::Invalid("ListenBrainz refused the listen".into())),
        c => Err(AppError::Network(format!("ListenBrainz returned HTTP {c}"))),
    }
}

fn lastfm_scrobble(items: &[&Scrobble]) -> AppResult<()> {
    let sk = crate::secrets::get(LASTFM_SESSION).ok_or_else(|| AppError::Unavailable("not connected to Last.fm".into()))?;
    // Up to 50 per request.
    for chunk in items.chunks(50) {
        let mut p = BTreeMap::new();
        p.insert("sk".to_string(), sk.clone());
        for (i, s) in chunk.iter().enumerate() {
            p.insert(format!("artist[{i}]"), s.artist.clone());
            p.insert(format!("track[{i}]"), s.track.clone());
            p.insert(format!("timestamp[{i}]"), s.timestamp.to_string());
            if !s.album.is_empty() {
                p.insert(format!("album[{i}]"), s.album.clone());
            }
            if let Some(d) = s.duration {
                p.insert(format!("duration[{i}]"), d.to_string());
            }
        }
        lastfm_call("track.scrobble", p, true)?;
    }
    Ok(())
}

fn from_track(track: &Track, started_ms: u64, service: &str) -> Option<Scrobble> {
    let artist = crate::radio::primary_artist(&track.artist);
    if artist.trim().is_empty() || track.title.trim().is_empty() {
        return None;
    }
    let album = track.versions.iter().find_map(|v| v.album.clone()).unwrap_or_default();
    Some(Scrobble { artist: artist.trim().to_string(), track: track.title.trim().to_string(), album, duration: track.duration.map(|d| d.round() as u32), timestamp: started_ms / 1000, service: service.into() })
}

/// Which plays are scrobbled (the services' rules).
pub fn eligible(play: &Play) -> bool {
    play.counts() && play.track.duration.map(|d| d > 30.0).unwrap_or(play.secs > 30.0) && !play.est
}

/// A finished listen (from stats).
pub fn on_play(play: &Play) {
    let c = config();
    if !(c.lastfm || c.listenbrainz) || !eligible(play) {
        return;
    }
    let mut new = Vec::new();
    if c.lastfm {
        new.extend(from_track(&play.track, play.t, "lastfm"));
    }
    if c.listenbrainz {
        new.extend(from_track(&play.track, play.t, "listenbrainz"));
    }
    if new.is_empty() {
        return;
    }
    {
        let _g = QUEUE_LOCK.lock().unwrap();
        let mut q = load_queue();
        q.extend(new);
        save_queue(&q);
    }
    std::thread::spawn(flush_queue);
}

/// A song started.
pub fn now_playing(track: &Track) {
    let c = config();
    if !(c.lastfm || c.listenbrainz) {
        return;
    }
    let track = track.clone();
    std::thread::spawn(move || {
        if c.lastfm {
            if let (Some(s), Some(sk)) = (from_track(&track, 0, "lastfm"), crate::secrets::get(LASTFM_SESSION)) {
                let mut p = BTreeMap::new();
                p.insert("sk".to_string(), sk);
                p.insert("artist".to_string(), s.artist);
                p.insert("track".to_string(), s.track);
                if !s.album.is_empty() {
                    p.insert("album".to_string(), s.album);
                }
                let _ = lastfm_call("track.updateNowPlaying", p, true);
            }
        }
        if c.listenbrainz {
            if let Some(s) = from_track(&track, 0, "listenbrainz") {
                let _ = lb_submit("playing_now", &[&s]);
            }
        }
    });
}

fn queue_path() -> Option<PathBuf> {
    DIR.lock().unwrap().as_ref().map(|d| d.join("scrobbles.json"))
}

fn load_queue() -> Vec<Scrobble> {
    queue_path().map(|p| crate::store::load_json(&p)).unwrap_or_default()
}

fn save_queue(q: &[Scrobble]) {
    let start = q.len().saturating_sub(QUEUE_MAX);
    if let (Some(p), Ok(data)) = (queue_path(), serde_json::to_vec(&q[start..])) {
        let _ = crate::store::write_atomic(&p, &data);
    }
}

/// Send what's waiting. Network trouble keeps it queued; a rejection drops it.
pub fn flush_queue() {
    static FLUSHING: Mutex<()> = Mutex::new(());
    let Ok(_f) = FLUSHING.try_lock() else { return };
    let q = {
        let _g = QUEUE_LOCK.lock().unwrap();
        load_queue()
    };
    if q.is_empty() {
        return;
    }
    let mut sent: Vec<Scrobble> = Vec::new();
    for service in ["lastfm", "listenbrainz"] {
        let items: Vec<&Scrobble> = q.iter().filter(|s| s.service == service).collect();
        if items.is_empty() {
            continue;
        }
        let r = if service == "lastfm" { lastfm_scrobble(&items) } else { items.chunks(100).try_for_each(|c| lb_submit("import", c)) };
        match r {
            Ok(()) => sent.extend(items.into_iter().cloned()),
            // Not connected / rejected: these will never go through.
            Err(AppError::Unavailable(_) | AppError::Invalid(_)) => sent.extend(items.into_iter().cloned()),
            Err(_) => {}
        }
    }
    let _g = QUEUE_LOCK.lock().unwrap();
    let rest: Vec<Scrobble> = load_queue().into_iter().filter(|s| !sent.contains(s)).collect();
    save_queue(&rest);
}

/// How many scrobbles are waiting to be sent.
pub fn pending() -> usize {
    load_queue().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lastfm_signature_matches_the_documented_scheme() {
        // api_keyXXXmethodauth.getSessiontokenYYY + secret, md5.
        let mut p = BTreeMap::new();
        p.insert("token".to_string(), "YYY".to_string());
        p.insert("method".to_string(), "auth.getSession".to_string());
        p.insert("api_key".to_string(), "XXX".to_string());
        p.insert("format".to_string(), "json".to_string());
        let expected = format!("{:x}", Md5::digest(b"api_keyXXXmethodauth.getSessiontokenYYYsecret"));
        assert_eq!(lastfm_sig(&p, "secret"), expected);
    }

    #[test]
    fn eligibility_follows_the_services_rules() {
        let t = |d: Option<f64>| Track { key: "a::b".into(), title: "b".into(), artist: "a".into(), duration: d, thumbnail: None, versions: vec![], score: 0.0 };
        let p = |secs: f64, d: Option<f64>| Play { t: 0, secs, est: false, platform: None, track: t(d) };
        assert!(eligible(&p(120.0, Some(200.0))));
        assert!(!eligible(&p(20.0, Some(200.0))), "too short a listen");
        assert!(!eligible(&p(25.0, Some(25.0))), "songs of 30 s or less never scrobble");
        assert!(eligible(&p(40.0, Some(60.0))), "half of a short song");
        let mut est = p(120.0, Some(200.0));
        est.est = true;
        assert!(!eligible(&est), "imported history isn't scrobbled");
    }

    #[test]
    fn scrobble_uses_primary_artist_and_start_time() {
        let mut tr = Track { key: "k".into(), title: "Song".into(), artist: "Main, Guest".into(), duration: Some(201.4), thumbnail: None, versions: vec![], score: 0.0 };
        let s = from_track(&tr, 1_700_000_000_123, "lastfm").unwrap();
        assert_eq!(s.timestamp, 1_700_000_000);
        assert_eq!(s.duration, Some(201));
        assert!(!s.artist.contains("Guest"), "{}", s.artist);
        tr.artist = "  ".into();
        assert!(from_track(&tr, 0, "lastfm").is_none());
    }
}
