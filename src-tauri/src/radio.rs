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

/// Up to `limit` songs related to `track`, skipping keys in `exclude`.
pub async fn related(track: &Track, exclude: &HashSet<String>, limit: usize, cache: &Arc<SearchCache>) -> AppResult<Vec<Track>> {
    let id = seed_id(track, cache).await?;
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
