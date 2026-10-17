//! Spotify support. Spotify audio is DRM-protected and yt-dlp has no Spotify
//! music extractor, so playback goes through a matched upload: first the exact
//! recording found by its ISRC on YouTube Music (see `isrc`), otherwise a
//! title/artist/duration match on YouTube Music / SoundCloud / YouTube.
//!
//! Track metadata (title/artist/duration) comes from Spotify's public embed
//! page (no API key, no OAuth, no client ID). This is the one place the app
//! fetches something without yt-dlp. It is metadata only and can be turned off
//! in Settings ("Spotify metadata bridge").

use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, RemotePlaylist, Track, TrackVersion};
use crate::search::build_track;

pub fn parse_url(url: &str) -> Option<(String, String)> {
    let u = url.trim();
    if let Some(rest) = u.strip_prefix("spotify:") {
        let mut it = rest.split(':');
        return Some((it.next()?.to_string(), it.next()?.to_string()));
    }
    let path = u.split("spotify.com/").nth(1)?;
    let path = path.split(['?', '#']).next()?;
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty() && !s.starts_with("intl-") && *s != "embed").collect();
    let kind = *segs.first()?;
    let id = *segs.get(1)?;
    matches!(kind, "track" | "playlist" | "album" | "episode" | "show" | "artist").then(|| (kind.to_string(), id.to_string()))
}

fn fetch_embed(kind: &str, id: &str) -> AppResult<Value> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::Invalid("malformed Spotify id".into()));
    }
    let url = format!("https://open.spotify.com/embed/{kind}/{id}");
    let mut resp = crate::http::agent().get(&url).header("User-Agent", crate::http::USER_AGENT).call().map_err(|e| AppError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(AppError::Unavailable(format!("Spotify returned HTTP {}", resp.status().as_u16())));
    }
    let html = resp
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| AppError::Network(e.to_string()))?;
    let marker = "<script id=\"__NEXT_DATA__\" type=\"application/json\">";
    let start = html.find(marker).ok_or_else(|| AppError::Extraction("Spotify page format changed".into()))? + marker.len();
    let end = html[start..].find("</script>").ok_or_else(|| AppError::Extraction("Spotify page format changed".into()))? + start;
    let data: Value = serde_json::from_str(&html[start..end])?;
    data.pointer("/props/pageProps/state/data/entity")
        .cloned()
        .ok_or_else(|| AppError::Unavailable("Spotify returned no data (private or region-locked?)".into()))
}

fn version(title: &str, artist: &str, duration_ms: Option<f64>, uri: &str, thumb: Option<String>) -> TrackVersion {
    let id = uri.rsplit(':').next().unwrap_or("").to_string();
    TrackVersion {
        platform: Platform::Spotify,
        url: format!("https://open.spotify.com/track/{id}"),
        id,
        title: title.to_string(),
        artist: artist.to_string(),
        duration: duration_ms.map(|d| d / 1000.0),
        thumbnail: thumb,
        preview: false,
        partial: false,
        artist_url: None,
        album: None,
        rank: 0,
        matched_for: None,
        isrc_match: false,
    }
}

fn cover(entity: &Value) -> Option<String> {
    entity
        .pointer("/coverArt/sources")
        .or_else(|| entity.pointer("/visualIdentity/image"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find_map(|s| s.get("url").and_then(Value::as_str)))
        .map(str::to_string)
}

/// Spotify URL → tracks (metadata only; versions have platform = Spotify).
pub async fn load(url: &str) -> AppResult<RemotePlaylist> {
    let (kind, id) = parse_url(url).ok_or_else(|| AppError::Invalid("not a Spotify link".into()))?;
    // Playlists: the web player's paginated query has every track (the embed
    // page stops at 50). The embed page stays as the fallback.
    if kind == "playlist" {
        let i2 = id.clone();
        if let Ok(Ok((title, tracks))) = tauri::async_runtime::spawn_blocking(move || fetch_playlist_full(&i2)).await {
            if !tracks.is_empty() {
                return Ok(RemotePlaylist { title, url: url.to_string(), platform: Some(Platform::Spotify), tracks });
            }
        }
    }
    let (k2, i2) = (kind.clone(), id.clone());
    let entity = tauri::async_runtime::spawn_blocking(move || fetch_embed(&k2, &i2))
        .await
        .map_err(|e| AppError::Network(e.to_string()))??;

    let name = entity.get("name").or_else(|| entity.get("title")).and_then(Value::as_str).unwrap_or("Spotify").to_string();
    let thumb = cover(&entity);
    let tracks: Vec<Track> = if kind == "track" {
        let artist = entity
            .get("artists")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        let dur = entity.get("duration").and_then(Value::as_f64);
        let uri = entity.get("uri").and_then(Value::as_str).unwrap_or("");
        vec![build_track(vec![version(&name, &artist, dur, uri, thumb.clone())], 0.0)]
    } else {
        entity
            .get("trackList")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|t| {
                        let title = t.get("title").and_then(Value::as_str)?;
                        let artist = t.get("subtitle").and_then(Value::as_str).unwrap_or("");
                        let uri = t.get("uri").and_then(Value::as_str).unwrap_or("");
                        if !uri.starts_with("spotify:track:") {
                            return None;
                        }
                        let dur = t.get("duration").and_then(Value::as_f64);
                        Some(build_track(vec![version(title, artist, dur, uri, thumb.clone())], 0.0))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    if tracks.is_empty() {
        return Err(AppError::EmptyResults);
    }
    Ok(RemotePlaylist { title: name, url: url.to_string(), platform: Some(Platform::Spotify), tracks })
}

#[cfg(test)]
mod tests {
    use super::parse_url;
    #[test]
    fn parses() {
        assert_eq!(parse_url("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=x"), Some(("track".into(), "4uLU6hMCjMI75M1A2tKUQC".into())));
        assert_eq!(parse_url("https://open.spotify.com/intl-de/playlist/37i9dQZF1DXcBWIGoYBM5M"), Some(("playlist".into(), "37i9dQZF1DXcBWIGoYBM5M".into())));
        assert_eq!(parse_url("spotify:album:abc"), Some(("album".into(), "abc".into())));
    }
}

// ---------------------------------------------------------------------------
// Search (no API key): the anonymous token from the embed page, sent to the
// same GraphQL endpoint the Spotify web player uses for its search page.
// ---------------------------------------------------------------------------

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Query id of the web player's `searchDesktop` operation. Refreshed from
/// Spotify's JavaScript when it changes; this value is the fallback.
const SEARCH_HASH_FALLBACK: &str = "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03";

static TOKEN: Mutex<Option<(String, u64)>> = Mutex::new(None);
static SEARCH_HASH: Mutex<Option<String>> = Mutex::new(None);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn get_text(url: &str) -> AppResult<String> {
    let mut r = crate::http::agent().get(url).header("User-Agent", crate::http::USER_AGENT).call().map_err(|e| AppError::Network(e.to_string()))?;
    if !r.status().is_success() {
        return Err(AppError::Network(format!("Spotify returned HTTP {}", r.status().as_u16())));
    }
    r.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))
}

/// Anonymous access token embedded in any embed page (cached until expiry).
/// Fetch the anonymous token ahead of the first search.
pub fn warm() {
    let _ = token(false);
}

fn token(force: bool) -> AppResult<String> {
    if !force {
        if let Some((t, exp)) = TOKEN.lock().unwrap().as_ref() {
            if *exp > now_ms() + 60_000 {
                return Ok(t.clone());
            }
        }
    }
    let html = get_text("https://open.spotify.com/embed/track/4uLU6hMCjMI75M1A2tKUQC")?;
    let marker = "<script id=\"__NEXT_DATA__\" type=\"application/json\">";
    let start = html.find(marker).ok_or_else(|| AppError::Extraction("Spotify embed page format changed".into()))? + marker.len();
    let end = html[start..].find("</script>").ok_or_else(|| AppError::Extraction("Spotify embed page format changed".into()))? + start;
    let data: Value = serde_json::from_str(&html[start..end])?;
    let session = data.pointer("/props/pageProps/state/settings/session").ok_or_else(|| AppError::Extraction("no Spotify session token".into()))?;
    let tok = session.get("accessToken").and_then(Value::as_str).ok_or_else(|| AppError::Extraction("no Spotify session token".into()))?.to_string();
    let exp = session.get("accessTokenExpirationTimestampMs").and_then(Value::as_u64).unwrap_or(now_ms() + 30 * 60_000);
    *TOKEN.lock().unwrap() = Some((tok.clone(), exp));
    Ok(tok)
}

/// Find the current `searchDesktop` query id in the web player's search chunk.
fn discover_search_hash() -> AppResult<String> {
    let page = get_text("https://open.spotify.com/search")?;
    let main = page
        .split('"')
        .find(|s| s.starts_with("https://open.spotifycdn.com/cdn/build/web-player/web-player.") && s.ends_with(".js"))
        .ok_or_else(|| AppError::Extraction("web player script not found".into()))?;
    let js = get_text(main)?;
    // Chunk id of the search route, then its content hash in the chunk map.
    let name_pos = js.find(":\"xpui-routes-search\"").ok_or_else(|| AppError::Extraction("search chunk not found".into()))?;
    let id: String = js[..name_pos].chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
    let needle = format!("{id}:\"");
    let hash = js
        .match_indices(&needle)
        .filter_map(|(i, _)| {
            let rest = &js[i + needle.len()..];
            let h: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            (h.len() >= 8 && rest[h.len()..].starts_with('"')).then_some(h)
        })
        .next()
        .ok_or_else(|| AppError::Extraction("search chunk hash not found".into()))?;
    let chunk = get_text(&format!("https://open.spotifycdn.com/cdn/build/web-player/xpui-routes-search.{hash}.js"))?;
    let key = "\"searchDesktop\",\"query\",\"";
    let at = chunk.find(key).ok_or_else(|| AppError::Extraction("searchDesktop query not found".into()))? + key.len();
    Ok(chunk[at..at + 64].to_string())
}

fn search_hash(refresh: bool) -> String {
    if !refresh {
        if let Some(h) = SEARCH_HASH.lock().unwrap().clone() {
            return h;
        }
    }
    let h = if refresh { discover_search_hash().unwrap_or_else(|_| SEARCH_HASH_FALLBACK.into()) } else { SEARCH_HASH_FALLBACK.into() };
    *SEARCH_HASH.lock().unwrap() = Some(h.clone());
    h
}

fn query(term: &str, limit: usize, token: &str, hash: &str) -> AppResult<(u16, Value)> {
    let body = serde_json::json!({
        "variables": {
            "searchTerm": term, "offset": 0, "limit": limit.clamp(1, 50), "numberOfTopResults": 5,
            "includeAudiobooks": false, "includeArtistHasConcertsField": false, "includePreReleases": false,
            "includeLocalConcertsField": false, "includeAuthors": false
        },
        "operationName": "searchDesktop",
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } }
    });
    let mut r = crate::http::agent()
        .post("https://api-partner.spotify.com/pathfinder/v2/query")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json;charset=UTF-8")
        .header("app-platform", "WebPlayer")
        .header("Origin", "https://open.spotify.com")
        .header("User-Agent", crate::http::USER_AGENT)
        .send(body.to_string())
        .map_err(|e| AppError::Network(e.to_string()))?;
    let status = r.status().as_u16();
    let text = r.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn parse_tracks(v: &Value) -> Vec<TrackVersion> {
    let items = v.pointer("/data/searchV2/tracksV2/items").and_then(Value::as_array).cloned().unwrap_or_default();
    items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let t = it.pointer("/item/data")?;
            let id = t.get("id").and_then(Value::as_str)?;
            let title = t.get("name").and_then(Value::as_str)?;
            let artist = t
                .pointer("/artists/items")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.pointer("/profile/name").and_then(Value::as_str)).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            let album = t.pointer("/albumOfTrack/name").and_then(Value::as_str).map(str::to_string);
            // ~300 px cover (good for rows and Discord).
            let thumbnail = t.pointer("/albumOfTrack/coverArt/sources").and_then(Value::as_array).and_then(|s| {
                s.iter()
                    .min_by_key(|x| (x.get("width").and_then(Value::as_i64).unwrap_or(0) - 300).abs())
                    .and_then(|x| x.get("url").and_then(Value::as_str))
                    .map(str::to_string)
            });
            Some(TrackVersion {
                platform: Platform::Spotify,
                url: format!("https://open.spotify.com/track/{id}"),
                id: id.to_string(),
                title: title.to_string(),
                artist,
                duration: t.pointer("/duration/totalMilliseconds").and_then(Value::as_f64).map(|ms| ms / 1000.0),
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
        .collect()
}

fn search_blocking(term: &str, limit: usize) -> AppResult<Vec<TrackVersion>> {
    let mut tok = token(false)?;
    let mut hash = search_hash(false);
    for attempt in 0..3 {
        let (status, v) = query(term, limit, &tok, &hash)?;
        let unknown_query = v.to_string().contains("PersistedQueryNotFound");
        match status {
            200 if !unknown_query && v.pointer("/data/searchV2").is_some() => return Ok(parse_tracks(&v)),
            401 => tok = token(true)?,
            _ if unknown_query || status == 400 || status == 404 => hash = search_hash(true),
            429 => return Err(AppError::Network("Spotify is rate limiting searches; try again in a moment".into())),
            s if attempt == 2 => return Err(AppError::Network(format!("Spotify search failed (HTTP {s})"))),
            _ => {}
        }
    }
    Err(AppError::Extraction("Spotify search is unavailable".into()))
}

/// Spotify track search (instant, no account or API key).
pub async fn search(term: &str, limit: usize) -> AppResult<Vec<TrackVersion>> {
    let term = term.to_string();
    tauri::async_runtime::spawn_blocking(move || search_blocking(&term, limit))
        .await
        .map_err(|e| AppError::Network(e.to_string()))?
}

#[cfg(test)]
mod live_search_tests {
    #[test]
    #[ignore]
    fn spotify_web_search() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let t0 = std::time::Instant::now();
        let r = rt.block_on(super::search("separatist", 10)).unwrap();
        println!("{} results in {:?}", r.len(), t0.elapsed());
        for v in r.iter().take(5) {
            println!("  {} — {} ({:?}s) {:?}", v.title, v.artist, v.duration, v.album);
        }
        assert!(!r.is_empty());
        let t1 = std::time::Instant::now();
        let r2 = rt.block_on(super::search("daft punk one more time", 10)).unwrap();
        println!("second search: {} results in {:?}; top: {} — {}", r2.len(), t1.elapsed(), r2[0].title, r2[0].artist);
        let fresh = super::discover_search_hash().unwrap();
        println!("discovered hash {fresh}");
        assert_eq!(fresh.len(), 64);
    }
}

// ---------------------------------------------------------------------------
// Playback matching: the track's ISRC (the recording's industry ID) from the
// web player's metadata endpoint, with the same anonymous token. Searching it
// on YouTube Music finds the exact official audio, not a cover or live cut.
// ---------------------------------------------------------------------------

static ISRC_CACHE: Mutex<Option<std::collections::HashMap<String, Option<String>>>> = Mutex::new(None);

/// Base62 track id → 32-hex "gid" used by the metadata endpoint.
fn gid(id: &str) -> Option<String> {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut n: u128 = 0;
    for c in id.bytes() {
        let d = ALPHABET.iter().position(|&a| a == c)? as u128;
        n = n.checked_mul(62)?.checked_add(d)?;
    }
    (id.len() == 22).then(|| format!("{n:032x}"))
}

fn isrc_blocking(id: &str) -> AppResult<Option<String>> {
    let gid = gid(id).ok_or_else(|| AppError::Invalid("malformed Spotify id".into()))?;
    let url = format!("https://spclient.wg.spotify.com/metadata/4/track/{gid}?market=from_token");
    let mut tok = token(false)?;
    for _ in 0..2 {
        let mut r = crate::http::agent()
            .get(&url)
            .header("Authorization", format!("Bearer {tok}"))
            .header("Accept", "application/json")
            .header("App-Platform", "WebPlayer")
            .header("User-Agent", crate::http::USER_AGENT)
            .call()
            .map_err(|e| AppError::Network(e.to_string()))?;
        match r.status().as_u16() {
            200 => {
                let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| AppError::Network(e.to_string()))?)?;
                return Ok(v
                    .get("external_id")
                    .and_then(Value::as_array)
                    .and_then(|a| a.iter().find(|x| x.get("type").and_then(Value::as_str) == Some("isrc")))
                    .and_then(|x| x.get("id").and_then(Value::as_str))
                    .filter(|s| s.len() == 12 && s.chars().all(|c| c.is_ascii_alphanumeric()))
                    .map(str::to_ascii_uppercase));
            }
            401 => tok = token(true)?,
            s => return Err(AppError::Network(format!("Spotify metadata returned HTTP {s}"))),
        }
    }
    Err(AppError::Network("Spotify metadata unavailable".into()))
}

/// ISRC of a Spotify track (cached; `None` when Spotify lists none).
pub async fn isrc(track_id: &str) -> AppResult<Option<String>> {
    if let Some(hit) = ISRC_CACHE.lock().unwrap().as_ref().and_then(|m| m.get(track_id).cloned()) {
        return Ok(hit);
    }
    let id = track_id.to_string();
    let found = tauri::async_runtime::spawn_blocking(move || isrc_blocking(&id))
        .await
        .map_err(|e| AppError::Network(e.to_string()))??;
    ISRC_CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(track_id.to_string(), found.clone());
    Ok(found)
}

#[cfg(test)]
mod isrc_tests {
    #[test]
    fn gid_conversion() {
        assert_eq!(super::gid("4uLU6hMCjMI75M1A2tKUQC").as_deref(), Some("93bc414a606747b2b612491ef83d5a3e"));
        assert_eq!(super::gid("short"), None);
    }

    #[test]
    #[ignore]
    fn live_isrc() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(rt.block_on(super::isrc("4uLU6hMCjMI75M1A2tKUQC")).unwrap().as_deref(), Some("GBARL9300135"));
    }
}

// ---------------------------------------------------------------------------
// Full playlists (web player's `fetchPlaylist` query, anonymous token)
// ---------------------------------------------------------------------------

const FETCH_PLAYLIST_FALLBACK: &str = "21ea11a6ac75a5f3e01fd365d29c18e0a4a8b7dc09f89bbb5d73910190c6acaf";
static PLAYLIST_HASH: Mutex<Option<String>> = Mutex::new(None);

/// Query id of a GraphQL operation defined in the web player's main script.
fn discover_op_hash(op: &str) -> AppResult<String> {
    let page = get_text("https://open.spotify.com/search")?;
    let main = page
        .split('"')
        .find(|s| s.starts_with("https://open.spotifycdn.com/cdn/build/web-player/web-player.") && s.ends_with(".js"))
        .ok_or_else(|| AppError::Extraction("web player script not found".into()))?;
    let js = get_text(main)?;
    let key = format!("\"{op}\",\"query\",\"");
    let at = js.find(&key).ok_or_else(|| AppError::Extraction(format!("{op} query not found")))? + key.len();
    let h = &js[at..(at + 64).min(js.len())];
    if h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(h.to_string())
    } else {
        Err(AppError::Extraction(format!("{op} query id malformed")))
    }
}

fn pathfinder(op: &str, hash: &str, variables: Value, tok: &str) -> AppResult<(u16, Value)> {
    let body = serde_json::json!({ "variables": variables, "operationName": op, "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } } });
    let mut r = crate::http::agent()
        .post("https://api-partner.spotify.com/pathfinder/v2/query")
        .header("Authorization", format!("Bearer {tok}"))
        .header("Content-Type", "application/json;charset=UTF-8")
        .header("app-platform", "WebPlayer")
        .header("Origin", "https://open.spotify.com")
        .header("User-Agent", crate::http::USER_AGENT)
        .send(body.to_string())
        .map_err(|e| AppError::Network(e.to_string()))?;
    let status = r.status().as_u16();
    let text = r.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn playlist_item_track(it: &Value) -> Option<Track> {
    let t = it.pointer("/itemV2/data")?;
    if t.get("__typename").and_then(Value::as_str) != Some("Track") {
        return None;
    }
    let uri = t.get("uri").and_then(Value::as_str)?;
    let title = t.get("name").and_then(Value::as_str)?;
    let artist = t
        .pointer("/artists/items")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.pointer("/profile/name").and_then(Value::as_str)).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let dur = t.pointer("/trackDuration/totalMilliseconds").and_then(Value::as_f64);
    let thumb = t.pointer("/albumOfTrack/coverArt/sources").and_then(Value::as_array).and_then(|s| {
        s.iter()
            .min_by_key(|x| (x.get("width").and_then(Value::as_i64).unwrap_or(0) - 300).abs())
            .and_then(|x| x.get("url").and_then(Value::as_str))
            .map(str::to_string)
    });
    let mut v = version(title, &artist, dur, uri, thumb);
    v.album = t.pointer("/albumOfTrack/name").and_then(Value::as_str).map(str::to_string);
    Some(build_track(vec![v], 0.0))
}

/// Every track of a playlist, 100 per request.
pub(crate) fn fetch_playlist_full(id: &str) -> AppResult<(String, Vec<Track>)> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::Invalid("malformed Spotify id".into()));
    }
    let mut tok = token(false)?;
    let mut hash = PLAYLIST_HASH.lock().unwrap().clone().unwrap_or_else(|| FETCH_PLAYLIST_FALLBACK.into());
    let mut title = String::from("Spotify playlist");
    let mut tracks = Vec::new();
    let mut offset = 0usize;
    let mut retried = false;
    loop {
        let vars = serde_json::json!({ "uri": format!("spotify:playlist:{id}"), "offset": offset, "limit": 100, "enableWatchFeedEntrypoint": false });
        let (status, v) = pathfinder("fetchPlaylist", &hash, vars, &tok)?;
        let unknown = v.to_string().contains("PersistedQueryNotFound");
        match status {
            401 if !retried => {
                tok = token(true)?;
                retried = true;
                continue;
            }
            _ if (unknown || status == 400 || status == 404) && !retried => {
                hash = discover_op_hash("fetchPlaylist")?;
                *PLAYLIST_HASH.lock().unwrap() = Some(hash.clone());
                retried = true;
                continue;
            }
            200 => {}
            s => return Err(AppError::Network(format!("Spotify playlist request failed (HTTP {s})"))),
        }
        let pl = v.pointer("/data/playlistV2").ok_or_else(|| AppError::Unavailable("Spotify returned no playlist (private?)".into()))?;
        if let Some(n) = pl.get("name").and_then(Value::as_str) {
            title = n.to_string();
        }
        let items = pl.pointer("/content/items").and_then(Value::as_array).cloned().unwrap_or_default();
        let total = pl.pointer("/content/totalCount").and_then(Value::as_u64).unwrap_or(0) as usize;
        tracks.extend(items.iter().filter_map(playlist_item_track));
        offset += items.len();
        if items.is_empty() || offset >= total || offset >= 10_000 {
            break;
        }
    }
    Ok((title, tracks))
}

// ---------------------------------------------------------------------------
// A user's public playlists (official Web API; needs the user's own app
// credentials from Settings → Spotify, client-credentials flow)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct UserPlaylist {
    pub name: String,
    pub url: String,
    pub tracks: u64,
    pub thumbnail: Option<String>,
    pub owner: String,
}

fn app_token(client_id: &str, secret: &str) -> AppResult<String> {
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{secret}"));
    let mut r = crate::http::agent()
        .post("https://accounts.spotify.com/api/token")
        .header("Authorization", format!("Basic {basic}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send("grant_type=client_credentials")
        .map_err(|e| AppError::Network(e.to_string()))?;
    let status = r.status().as_u16();
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| AppError::Network(e.to_string()))?).unwrap_or(Value::Null);
    match (status, v.get("access_token").and_then(Value::as_str)) {
        (200, Some(t)) => Ok(t.to_string()),
        _ => Err(AppError::Invalid("Spotify rejected the app credentials (Settings → Spotify)".into())),
    }
}

pub fn user_id_from_url(url: &str) -> Option<String> {
    let u = url.trim();
    let id = if let Some(rest) = u.strip_prefix("spotify:user:") {
        rest
    } else {
        u.split("spotify.com/").nth(1)?.split(['?', '#']).next()?.split('/').filter(|s| !s.starts_with("intl-")).skip_while(|s| *s != "user").nth(1)?
    };
    (!id.is_empty() && id.len() < 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))).then(|| id.to_string())
}

pub async fn user_playlists(user: String, client_id: String, secret: String) -> AppResult<Vec<UserPlaylist>> {
    if client_id.trim().is_empty() || secret.trim().is_empty() {
        return Err(AppError::Invalid(
            "listing a profile's playlists needs your own free Spotify app: add its Client ID and secret in Settings → Spotify".into(),
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let tok = app_token(client_id.trim(), secret.trim())?;
        let mut out = Vec::new();
        let mut next = Some(format!("https://api.spotify.com/v1/users/{user}/playlists?limit=50"));
        while let Some(url) = next.take() {
            let mut r = crate::http::agent().get(&url).header("Authorization", format!("Bearer {tok}")).call().map_err(|e| AppError::Network(e.to_string()))?;
            let status = r.status().as_u16();
            if status == 404 {
                return Err(AppError::NotFound("Spotify user".into()));
            }
            if status != 200 {
                return Err(AppError::Network(format!("Spotify returned HTTP {status}")));
            }
            let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| AppError::Network(e.to_string()))?)?;
            for p in v.get("items").and_then(Value::as_array).into_iter().flatten() {
                let (Some(name), Some(id)) = (p.get("name").and_then(Value::as_str), p.get("id").and_then(Value::as_str)) else { continue };
                out.push(UserPlaylist {
                    name: name.to_string(),
                    url: format!("https://open.spotify.com/playlist/{id}"),
                    tracks: p.pointer("/tracks/total").and_then(Value::as_u64).unwrap_or(0),
                    thumbnail: p.pointer("/images/0/url").and_then(Value::as_str).map(str::to_string),
                    owner: p.pointer("/owner/display_name").and_then(Value::as_str).unwrap_or("").to_string(),
                });
            }
            next = v.get("next").and_then(Value::as_str).filter(|_| out.len() < 1000).map(str::to_string);
        }
        Ok(out)
    })
    .await
    .map_err(|e| AppError::Network(e.to_string()))?
}

#[cfg(test)]
mod import_tests {
    #[test]
    fn user_ids() {
        assert_eq!(super::user_id_from_url("https://open.spotify.com/user/abc123?si=x").as_deref(), Some("abc123"));
        assert_eq!(super::user_id_from_url("https://open.spotify.com/intl-de/user/a.b_c").as_deref(), Some("a.b_c"));
        assert_eq!(super::user_id_from_url("spotify:user:someone").as_deref(), Some("someone"));
        assert_eq!(super::user_id_from_url("https://open.spotify.com/playlist/x"), None);
    }

    #[test]
    #[ignore]
    fn live_full_playlist() {
        // A big public playlist (> 100 tracks) to exercise paging.
        let (title, tracks) = super::fetch_playlist_full("37i9dQZF1DX4o1oenSJRJd").unwrap();
        println!("{title}: {} tracks; first {} — {}", tracks.len(), tracks[0].title, tracks[0].artist);
        assert!(tracks.len() > 100);
        println!("discovered {}", super::discover_op_hash("fetchPlaylist").unwrap());
    }
}
