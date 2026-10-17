//! Reading a Spotify library through Spotify's own client service (the
//! connection the desktop app uses, via librespot), not the public Web API.
//!
//! The import signs in with Spotify's desktop-app client id so nobody needs
//! developer keys, but every librespot-based app in the world shares that id
//! and its Web API quota, so the Web API answers "429 Too Many Requests".
//! Here the same sign-in opens a client session instead: the rootlist (your
//! playlists), playlists, Liked Songs (the "collection" context) and track
//! metadata all come from the client service.
//!
//! This session is the import's own: it isn't saved and doesn't turn on
//! Spotify playback (which needs Premium); free accounts work.

use std::time::Duration;

use librespot_core::authentication::Credentials;
use librespot_core::{Session, SessionConfig, SpotifyUri};
use librespot_metadata::Metadata;
use protobuf::Message;

use crate::error::{AppError, AppResult};
use crate::imports::SpotifyPlaylistInfo;
use crate::models::{Platform, Track, TrackVersion};

/// Parallel metadata requests (the client service is happy with a few).
const PARALLEL: usize = 12;

pub async fn session(access_token: &str) -> AppResult<Session> {
    let session = Session::new(SessionConfig::default(), None);
    tokio::time::timeout(Duration::from_secs(30), session.connect(Credentials::with_access_token(access_token), false))
        .await
        .map_err(|_| AppError::Timeout("Spotify didn't answer".into()))?
        .map_err(|e| AppError::Network(format!("couldn't connect to Spotify: {e}")))?;
    Ok(session)
}

fn image_url(picture: &[u8]) -> Option<String> {
    (picture.len() == 20).then(|| format!("https://i.scdn.co/image/{}", picture.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

/// Your playlists (followed ones too), in your order; folders are flattened.
pub async fn rootlist(session: &Session) -> AppResult<Vec<SpotifyPlaylistInfo>> {
    let mut out = Vec::new();
    let mut from = 0;
    loop {
        let bytes = session.spclient().get_rootlist(from, Some(200)).await.map_err(|e| AppError::Extraction(format!("Spotify playlist list: {e}")))?;
        let msg = librespot_protocol::playlist4_external::SelectedListContent::parse_from_bytes(&bytes).map_err(|e| AppError::Extraction(format!("Spotify playlist list: {e}")))?;
        let contents = msg.contents.get_or_default();
        let items: Vec<String> = contents.items.iter().filter_map(|i| i.uri.clone()).collect();
        if items.is_empty() {
            break;
        }
        // meta_items line up with the items (folder markers included).
        for (k, uri) in items.iter().enumerate() {
            let Some(id) = uri.strip_prefix("spotify:playlist:") else { continue };
            let meta = contents.meta_items.get(k);
            let attrs = meta.map(|m| m.attributes.get_or_default());
            out.push(SpotifyPlaylistInfo {
                id: id.to_string(),
                name: attrs.and_then(|a| a.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Playlist".into()),
                owner: meta.and_then(|m| m.owner_username.clone()).unwrap_or_default(),
                total: meta.and_then(|m| m.length).unwrap_or(0).max(0) as usize,
                thumbnail: attrs.and_then(|a| a.picture.as_deref()).and_then(image_url),
            });
        }
        from += items.len();
        let total = msg.length.unwrap_or(0).max(0) as usize;
        if from >= total || from > 2000 {
            break;
        }
    }
    // Names missing from the list: ask each playlist (rare).
    for p in out.iter_mut().filter(|p| p.name == "Playlist") {
        if let Ok(uri) = SpotifyUri::from_uri(&format!("spotify:playlist:{}", p.id)) {
            if let Ok(pl) = librespot_metadata::Playlist::get(session, &uri).await {
                p.name = pl.name().to_string();
                p.total = pl.length.max(0) as usize;
            }
        }
    }
    Ok(out)
}

/// Song URIs of a playlist.
pub async fn playlist_uris(session: &Session, id: &str) -> AppResult<Vec<SpotifyUri>> {
    let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{id}")).map_err(|_| AppError::Invalid("bad playlist id".into()))?;
    let pl = librespot_metadata::Playlist::get(session, &uri).await.map_err(|e| AppError::Extraction(format!("Spotify playlist: {e}")))?;
    Ok(pl.tracks().filter(|u| matches!(u, SpotifyUri::Track { .. })).cloned().collect())
}

/// Liked Songs (the collection), newest first.
pub async fn liked_uris(session: &Session) -> AppResult<Vec<SpotifyUri>> {
    let user = session.username();
    let ctx = session.spclient().get_context(&format!("spotify:user:{user}:collection")).await.map_err(|e| AppError::Extraction(format!("Spotify Liked Songs: {e}")))?;
    let mut uris = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for page in &ctx.pages {
        collect(&page.tracks, &mut uris);
        if page.tracks.is_empty() {
            pending.extend(page.page_url.clone());
        }
        pending.extend(page.next_page_url.clone());
    }
    // Further pages, as many as there are (bounded).
    let mut seen = std::collections::HashSet::new();
    while let Some(url) = pending.pop() {
        if !seen.insert(url.clone()) || seen.len() > 200 {
            continue;
        }
        let Ok(bytes) = session.spclient().get_next_page(&url).await else { continue };
        let Ok(text) = String::from_utf8(bytes.to_vec()) else { continue };
        let Ok(page) = protobuf_json_mapping::parse_from_str::<librespot_protocol::context_page::ContextPage>(&text) else { continue };
        collect(&page.tracks, &mut uris);
        pending.extend(page.next_page_url.clone());
    }
    Ok(uris)
}

fn collect(tracks: &[librespot_protocol::context_track::ContextTrack], out: &mut Vec<SpotifyUri>) {
    for t in tracks {
        if let Some(u) = t.uri.as_deref().filter(|u| u.starts_with("spotify:track:")).and_then(|u| SpotifyUri::from_uri(u).ok()) {
            out.push(u);
        }
    }
}

/// Titles, artists, albums and covers for track URIs (several at a time).
/// Songs Spotify can't describe are left out. `progress(done, total)`.
pub async fn tracks(session: &Session, uris: Vec<SpotifyUri>, progress: impl Fn(usize, usize)) -> Vec<Track> {
    use futures_util::stream::{self, StreamExt};
    let total = uris.len();
    let mut done = 0;
    let mut results: Vec<(usize, Option<Track>)> = Vec::with_capacity(total);
    let mut stream = stream::iter(uris.into_iter().enumerate()).map(|(i, uri)| {
        let session = session.clone();
        async move { (i, librespot_metadata::Track::get(&session, &uri).await.ok().and_then(|t| to_track(&t))) }
    }).buffer_unordered(PARALLEL);
    while let Some(r) = stream.next().await {
        results.push(r);
        done += 1;
        if done % 25 == 0 || done == total {
            progress(done, total);
        }
    }
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().filter_map(|(_, t)| t).collect()
}

fn to_track(t: &librespot_metadata::Track) -> Option<Track> {
    let SpotifyUri::Track { id } = &t.id else { return None };
    let id = id.to_base62().ok()?;
    let artist = t.artists.iter().map(|a| a.name.as_str()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ");
    let thumbnail = t.album.covers.iter().max_by_key(|c| c.width).and_then(|c| c.id.to_base16().ok()).map(|h| format!("https://i.scdn.co/image/{h}"));
    Some(Track::from_version(TrackVersion {
        platform: Platform::Spotify,
        url: format!("https://open.spotify.com/track/{id}"),
        id,
        title: t.name.clone(),
        artist,
        duration: (t.duration > 0).then(|| t.duration as f64 / 1000.0),
        thumbnail,
        preview: false,
        partial: false,
        artist_url: None,
        album: (!t.album.name.is_empty()).then(|| t.album.name.clone()),
        rank: 0,
        matched_for: None,
        isrc_match: false,
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cover_ids_become_image_urls() {
        let pic: Vec<u8> = (0..20).collect();
        assert_eq!(super::image_url(&pic).as_deref(), Some("https://i.scdn.co/image/000102030405060708090a0b0c0d0e0f10111213"));
        assert_eq!(super::image_url(&[1, 2, 3]), None);
    }
}
