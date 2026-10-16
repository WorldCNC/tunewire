//! Spotify support. Spotify audio is DRM-protected and yt-dlp has no Spotify
//! music extractor, so playback always goes through a matched YouTube Music /
//! SoundCloud upload. yt-dlp is tried first anyway, so this keeps working if a
//! future yt-dlp gains support.
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
