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
