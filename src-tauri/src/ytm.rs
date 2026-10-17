//! YouTube Music's own web API (the one music.youtube.com uses). One request
//! returns a whole artist page, album or search shelf with full metadata, in
//! well under a second, where the equivalent yt-dlp listings take several.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track, TrackVersion};

const F_SONGS: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
const F_ALBUMS: &str = "EgWKAQIYAWoMEA4QChADEAQQCRAF";
const F_ARTISTS: &str = "EgWKAQIgAWoMEA4QChADEAQQCRAF";
const F_FEATURED_PLAYLISTS: &str = "EgeKAQQoADgBagwQDhAKEAMQBBAJEAU=";
const F_COMMUNITY_PLAYLISTS: &str = "EgeKAQQoAEABagwQDhAKEAMQBBAJEAU=";

// ---------------------------------------------------------------------------
// Transport + small cache
// ---------------------------------------------------------------------------

fn post_blocking(endpoint: &str, extra: Value) -> AppResult<Value> {
    post_blocking_to("https://music.youtube.com", "WEB_REMIX", "1.20250101.01.00", endpoint, extra)
}

fn post_blocking_to(origin: &str, client: &str, version: &str, endpoint: &str, extra: Value) -> AppResult<Value> {
    let mut body = serde_json::json!({
        "context": { "client": { "clientName": client, "clientVersion": version, "hl": "en", "gl": "US" } }
    });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    let mut r = crate::http::agent()
        .post(&format!("{origin}/youtubei/v1/{endpoint}?prettyPrint=false"))
        .header("Content-Type", "application/json")
        .header("Origin", origin)
        .header("User-Agent", crate::http::USER_AGENT)
        .send(body.to_string())
        .map_err(|e| AppError::Network(e.to_string()))?;
    if !r.status().is_success() {
        return Err(AppError::Network(format!("YouTube Music returned HTTP {}", r.status().as_u16())));
    }
    let text = r.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
    Ok(serde_json::from_str(&text)?)
}

static CACHE: Mutex<Option<HashMap<String, (Instant, Value)>>> = Mutex::new(None);
const TTL: Duration = Duration::from_secs(20 * 60);

/// POST with a 20-minute response cache (pages are revisited a lot).
async fn post(endpoint: &str, extra: Value) -> AppResult<Value> {
    let key = format!("{endpoint}|{extra}");
    if let Some((at, v)) = CACHE.lock().unwrap().as_ref().and_then(|m| m.get(&key).cloned()) {
        if at.elapsed() < TTL {
            return Ok(v);
        }
    }
    let ep = endpoint.to_string();
    let v = tauri::async_runtime::spawn_blocking(move || post_blocking(&ep, extra)).await.map_err(|e| AppError::Network(e.to_string()))??;
    let mut g = CACHE.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 300 {
        m.retain(|_, (at, _)| at.elapsed() < TTL);
    }
    m.insert(key, (Instant::now(), v.clone()));
    Ok(v)
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

fn text(v: Option<&Value>) -> String {
    v.and_then(|v| v.get("runs")).and_then(Value::as_array).map(|a| a.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect()).unwrap_or_default()
}

fn runs(v: Option<&Value>) -> Vec<Value> {
    v.and_then(|v| v.get("runs")).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn best_thumb(v: Option<&Value>) -> Option<String> {
    let v = v?;
    let list = v
        .pointer("/musicThumbnailRenderer/thumbnail/thumbnails")
        .or_else(|| v.pointer("/croppedSquareThumbnailRenderer/thumbnail/thumbnails"))
        .or_else(|| v.pointer("/thumbnails"))
        .and_then(Value::as_array)?;
    list.last().and_then(|t| t.get("url").and_then(Value::as_str)).map(|u| upscale(u, 544))
}

/// googleusercontent thumbnails are sized by URL ("=w120-h120-..."): ask for a bigger one.
fn upscale(url: &str, px: u32) -> String {
    if url.contains("googleusercontent.com") {
        if let Some(i) = url.rfind("=w") {
            return format!("{}=w{px}-h{px}-l90-rj", &url[..i]);
        }
        if let Some(i) = url.rfind("=s") {
            return format!("{}=s{px}", &url[..i]);
        }
    }
    url.to_string()
}

fn page_type(run: &Value) -> &str {
    run.pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType").and_then(Value::as_str).unwrap_or("")
}

fn nav_browse(v: &Value) -> Option<String> {
    v.pointer("/navigationEndpoint/browseEndpoint/browseId").and_then(Value::as_str).map(str::to_string)
}

fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || !s.contains(':') {
        return None;
    }
    let mut total = 0.0;
    for part in s.split(':') {
        total = total * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    Some(total)
}

fn col(item: &Value, i: usize) -> Option<&Value> {
    item.get("flexColumns").and_then(Value::as_array).and_then(|c| c.get(i)).and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text"))
}

fn fixed(item: &Value) -> String {
    item.get("fixedColumns").and_then(Value::as_array).and_then(|c| c.first()).map(|c| text(c.pointer("/musicResponsiveListItemFixedColumnRenderer/text"))).unwrap_or_default()
}

/// A card for an artist, album, playlist or mood (grids and carousels).
#[derive(Debug, Clone, Serialize)]
pub struct Card {
    /// "artist" | "album" | "single" | "ep" | "playlist" | "song" | "video"
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub browse_id: Option<String>,
    pub video_id: Option<String>,
    pub thumbnail: Option<String>,
    /// Songs and videos come with a playable track.
    pub track: Option<Track>,
}

fn kind_of(pt: &str, subtitle: &str) -> String {
    let first = subtitle.split(" • ").next().unwrap_or("").trim().to_lowercase();
    match pt {
        "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" => "artist".into(),
        "MUSIC_PAGE_TYPE_ALBUM" => match first.as_str() {
            "single" => "single".into(),
            "ep" => "ep".into(),
            _ => "album".into(),
        },
        "MUSIC_PAGE_TYPE_PLAYLIST" => "playlist".into(),
        _ => match first.as_str() {
            "song" => "song".into(),
            "video" | "episode" => "video".into(),
            _ => "song".into(),
        },
    }
}

fn song_track(video_id: &str, title: String, artist: String, album: Option<String>, duration: Option<f64>, thumbnail: Option<String>, artist_browse: Option<String>) -> Track {
    Track::from_version(TrackVersion {
        platform: Platform::Ytmusic,
        url: format!("https://music.youtube.com/watch?v={video_id}"),
        id: video_id.to_string(),
        title,
        artist,
        duration,
        thumbnail,
        preview: false,
        partial: false,
        artist_url: artist_browse.map(|b| format!("https://music.youtube.com/channel/{b}")),
        album,
        rank: 0,
        matched_for: None,
        isrc_match: false,
    })
}

/// A row in a list (search results, top songs, album and playlist tracks).
/// `album_artist` fills songs listed without one (album pages).
fn list_item(m: &Value, album_artist: Option<(&str, Option<&str>)>, album: Option<(&str, Option<&str>)>) -> Option<Card> {
    let title = text(col(m, 0));
    if title.is_empty() {
        return None;
    }
    let byline = runs(col(m, 1));
    let sub = text(col(m, 1));
    let video_id = m
        .pointer("/playlistItemData/videoId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| runs(col(m, 0)).first().and_then(|r| r.pointer("/navigationEndpoint/watchEndpoint/videoId")).and_then(Value::as_str).map(str::to_string))
        .or_else(|| m.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId").and_then(Value::as_str).map(str::to_string));
    let own_browse = nav_browse(m);
    let pt = m.pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType").and_then(Value::as_str).unwrap_or("");
    let thumb = best_thumb(m.get("thumbnail"));
    let kind = if video_id.is_some() && own_browse.is_none() { kind_of("", &sub) } else { kind_of(pt, &sub) };

    let track = video_id.as_ref().filter(|_| kind == "song" || kind == "video").map(|vid| {
        let artists: Vec<(String, Option<String>)> = byline
            .iter()
            .filter(|r| matches!(page_type(r), "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL"))
            .filter_map(|r| Some((r.get("text")?.as_str()?.to_string(), nav_browse(r))))
            .collect();
        let mut artist = artists.iter().map(|a| a.0.clone()).collect::<Vec<_>>().join(", ");
        let mut artist_browse = artists.first().and_then(|a| a.1.clone());
        if artist.is_empty() {
            // Plain-text byline: "Song • Artist • Album • 3:21" or "Artist • ..."
            let parts: Vec<&str> = sub.split(" • ").collect();
            artist = parts.iter().find(|p| !matches!(p.trim().to_lowercase().as_str(), "song" | "video" | "episode") && parse_duration(p).is_none() && !p.contains(" plays") && !p.contains(" views")).map(|s| s.trim().to_string()).unwrap_or_default();
            if artist.is_empty() {
                if let Some((a, b)) = album_artist {
                    artist = a.to_string();
                    artist_browse = b.map(str::to_string);
                }
            }
        }
        let album_run = byline.iter().chain(runs(col(m, 2)).iter()).chain(runs(col(m, 3)).iter()).find(|r| page_type(r) == "MUSIC_PAGE_TYPE_ALBUM").and_then(|r| r.get("text").and_then(Value::as_str)).map(str::to_string);
        let duration = parse_duration(&fixed(m)).or_else(|| sub.split(" • ").filter_map(parse_duration).last());
        let album_name = album_run.or_else(|| album.map(|a| a.0.to_string()));
        let thumbnail = thumb.clone().or_else(|| album.and_then(|a| a.1.map(str::to_string))).or_else(|| Some(format!("https://i.ytimg.com/vi/{vid}/hqdefault.jpg")));
        song_track(vid, title.clone(), artist, album_name, duration, thumbnail, artist_browse)
    });
    Some(Card { kind, title, subtitle: sub, browse_id: own_browse, video_id, thumbnail: thumb, track })
}

/// A tile in a carousel or grid.
fn two_row(t: &Value) -> Option<Card> {
    let title = text(t.get("title"));
    if title.is_empty() {
        return None;
    }
    let subtitle = text(t.get("subtitle"));
    let nav_browse = nav_browse(t);
    let pt = t.pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType").and_then(Value::as_str).unwrap_or("");
    let video_id = t.pointer("/navigationEndpoint/watchEndpoint/videoId").and_then(Value::as_str).map(str::to_string);
    let thumbnail = best_thumb(t.get("thumbnailRenderer"));
    let kind = if video_id.is_some() { "video".into() } else { kind_of(pt, &subtitle) };
    let track = video_id.as_ref().map(|vid| {
        let artist = subtitle.split(" • ").find(|p| !p.contains(" views") && !matches!(p.to_lowercase().as_str(), "song" | "video")).unwrap_or("").to_string();
        song_track(vid, title.clone(), artist, None, None, thumbnail.clone(), None)
    });
    Some(Card { kind, title, subtitle, browse_id: nav_browse, video_id, thumbnail, track })
}

fn shelf_items(shelf: &Value) -> Vec<Card> {
    shelf
        .get("contents")
        .or_else(|| shelf.get("items"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|i| {
                    if let Some(t) = i.get("musicTwoRowItemRenderer") {
                        two_row(t)
                    } else {
                        i.get("musicResponsiveListItemRenderer").and_then(|m| list_item(m, None, None))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn search_sections(v: &Value) -> Vec<Value> {
    v.pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").and_then(Value::as_array).cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

async fn search_filtered(query: &str, filter: &str) -> AppResult<Vec<Card>> {
    let v = post("search", serde_json::json!({ "query": query, "params": filter })).await?;
    Ok(search_sections(&v).iter().filter_map(|s| s.get("musicShelfRenderer")).flat_map(shelf_items).collect())
}

/// Songs only (used to match Spotify tracks).
pub async fn search_songs(query: &str) -> AppResult<Vec<TrackVersion>> {
    let cards = search_filtered(query, F_SONGS).await?;
    Ok(cards.into_iter().filter_map(|c| c.track).filter_map(|t| t.versions.into_iter().next()).enumerate().map(|(i, mut v)| {
        v.rank = i;
        v
    }).collect())
}

/// YouTube (not Music) video search, as yt-dlp-style entries for `parse_entry`.
pub async fn search_videos(query: &str, limit: usize) -> AppResult<Vec<Value>> {
    let q = query.to_string();
    let v = tauri::async_runtime::spawn_blocking(move || {
        post_blocking_to("https://www.youtube.com", "WEB", "2.20250101.00.00", "search", serde_json::json!({ "query": q, "params": "EgIQAQ%3D%3D" }))
    })
    .await
    .map_err(|e| AppError::Network(e.to_string()))??;
    let sections = v.pointer("/contents/twoColumnSearchResultsRenderer/primaryContents/sectionListRenderer/contents").and_then(Value::as_array).cloned().unwrap_or_default();
    let out: Vec<Value> = sections
        .iter()
        .filter_map(|s| s.pointer("/itemSectionRenderer/contents").and_then(Value::as_array))
        .flatten()
        .filter_map(|i| i.get("videoRenderer"))
        .filter_map(|r| {
            let id = r.get("videoId")?.as_str()?;
            let owner = r.pointer("/ownerText/runs/0")?;
            let channel_path = owner.pointer("/navigationEndpoint/browseEndpoint/canonicalBaseUrl").and_then(Value::as_str).unwrap_or("");
            let verified = r.get("ownerBadges").and_then(Value::as_array).map(|b| b.iter().any(|x| x.pointer("/metadataBadgeRenderer/style").and_then(Value::as_str).is_some_and(|s| s.contains("VERIFIED")))).unwrap_or(false);
            Some(serde_json::json!({
                "id": id,
                "url": format!("https://www.youtube.com/watch?v={id}"),
                "title": text(r.get("title")),
                "channel": owner.get("text"),
                "channel_url": if channel_path.is_empty() { Value::Null } else { Value::String(format!("https://www.youtube.com{channel_path}")) },
                "channel_is_verified": verified,
                "duration": r.pointer("/lengthText/simpleText").and_then(Value::as_str).and_then(parse_duration),
            }))
        })
        .take(limit)
        .collect();
    if out.is_empty() {
        return Err(AppError::EmptyResults);
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchExtras {
    pub top: Option<Card>,
    pub artists: Vec<Card>,
    pub albums: Vec<Card>,
    pub playlists: Vec<Card>,
}

/// Top result, artists, albums and playlists for the search page.
pub async fn search_extras(query: &str) -> AppResult<SearchExtras> {
    let q = query.to_string();
    let top = async {
        let v = post("search", serde_json::json!({ "query": q })).await?;
        let card = search_sections(&v).into_iter().find_map(|s| s.get("musicCardShelfRenderer").cloned());
        Ok::<_, AppError>(card.and_then(|c| {
            let title_run = runs(c.get("title")).into_iter().next()?;
            let title = title_run.get("text")?.as_str()?.to_string();
            let subtitle = text(c.get("subtitle"));
            let pt = page_type(&title_run).to_string();
            let video_id = title_run.pointer("/navigationEndpoint/watchEndpoint/videoId").and_then(Value::as_str).map(str::to_string);
            let thumbnail = best_thumb(c.get("thumbnail"));
            let kind = if video_id.is_some() { kind_of("", &subtitle) } else { kind_of(&pt, &subtitle) };
            let track = video_id.as_ref().map(|vid| {
                let parts: Vec<&str> = subtitle.split(" • ").collect();
                let artist = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
                let duration = parts.iter().filter_map(|p| parse_duration(p)).last();
                song_track(vid, title.clone(), artist, None, duration, thumbnail.clone(), None)
            });
            Some(Card { kind, title, subtitle, browse_id: nav_browse(&title_run), video_id, thumbnail, track })
        }))
    };
    let (top, artists, albums, featured, community) = tokio::join!(
        top,
        search_filtered(query, F_ARTISTS),
        search_filtered(query, F_ALBUMS),
        search_filtered(query, F_FEATURED_PLAYLISTS),
        search_filtered(query, F_COMMUNITY_PLAYLISTS)
    );
    let mut playlists = featured.unwrap_or_default();
    playlists.truncate(6);
    playlists.extend(community.unwrap_or_default().into_iter().take(10 - playlists.len().min(10)));
    Ok(SearchExtras {
        top: top.ok().flatten(),
        artists: artists.unwrap_or_default().into_iter().take(10).collect(),
        albums: albums.unwrap_or_default().into_iter().take(10).collect(),
        playlists,
    })
}

// ---------------------------------------------------------------------------
// Artist
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Shelf {
    pub title: String,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtistInfo {
    pub name: String,
    pub browse_id: String,
    /// Wide header image.
    pub banner: Option<String>,
    /// Square picture (from the search result, else the banner).
    pub thumbnail: Option<String>,
    pub audience: String,
    pub subscribers: String,
    pub description: String,
    pub top_songs: Vec<Track>,
    /// Playlist with all their songs (browse id).
    pub all_songs: Option<String>,
    pub albums: Vec<Card>,
    pub singles: Vec<Card>,
    pub related: Vec<Card>,
    pub shelves: Vec<Shelf>,
}

fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (crate::search::norm_text(a).replace(' ', ""), crate::search::norm_text(b).replace(' ', ""));
    !a.is_empty() && a == b
}

/// Find an artist's page id by name.
pub async fn find_artist(name: &str) -> AppResult<(String, Option<String>)> {
    let found = search_filtered(name, F_ARTISTS).await?;
    let pick = found.iter().find(|c| same_name(&c.title, name)).or_else(|| found.first());
    pick.and_then(|c| c.browse_id.clone().map(|b| (b, c.thumbnail.clone()))).ok_or_else(|| AppError::NotFound(format!("no artist page for “{name}”")))
}

/// An album's page id from its artist and title ("Go to album").
pub async fn find_album(artist: &str, album: &str) -> AppResult<Option<String>> {
    let found = search_filtered(&format!("{artist} {album}"), F_ALBUMS).await?;
    let want = crate::search::norm_text(album);
    let pick = found.iter().find(|c| crate::search::norm_text(&c.title) == want).or_else(|| found.iter().find(|c| crate::search::norm_text(&c.title).starts_with(&want)));
    Ok(pick.and_then(|c| c.browse_id.clone()))
}

pub async fn artist(browse_id: &str) -> AppResult<ArtistInfo> {
    let v = post("browse", serde_json::json!({ "browseId": browse_id })).await?;
    let header = v.pointer("/header/musicImmersiveHeaderRenderer").or_else(|| v.pointer("/header/musicVisualHeaderRenderer")).ok_or_else(|| AppError::NotFound("not an artist page".into()))?;
    let name = text(header.get("title"));
    let banner = header.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails").and_then(Value::as_array).and_then(|a| a.iter().find(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0) >= 1400).or(a.last())).and_then(|t| t.get("url").and_then(Value::as_str)).map(str::to_string);
    let mut info = ArtistInfo {
        name: name.clone(),
        browse_id: browse_id.to_string(),
        banner,
        thumbnail: None,
        audience: text(header.get("monthlyListenerCount")),
        subscribers: text(header.pointer("/subscriptionButton/subscribeButtonRenderer/subscriberCountText")),
        description: text(header.get("description")),
        top_songs: Vec::new(),
        all_songs: None,
        albums: Vec::new(),
        singles: Vec::new(),
        related: Vec::new(),
        shelves: Vec::new(),
    };
    let sections = v.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").and_then(Value::as_array).cloned().unwrap_or_default();
    for s in &sections {
        if let Some(shelf) = s.get("musicShelfRenderer") {
            info.all_songs = shelf.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId").and_then(Value::as_str).map(str::to_string);
            info.top_songs = shelf
                .get("contents")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|i| i.get("musicResponsiveListItemRenderer")).filter_map(|m| list_item(m, Some((&name, Some(browse_id))), None)).filter_map(|c| c.track).collect())
                .unwrap_or_default();
        } else if let Some(car) = s.get("musicCarouselShelfRenderer") {
            let title = text(car.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title"));
            let cards = shelf_items(car);
            match title.to_lowercase().as_str() {
                "albums" => info.albums = cards,
                "singles" | "singles & eps" | "singles and eps" => info.singles = cards,
                t if t.starts_with("fans might also like") || t.starts_with("fans also like") => info.related = cards,
                _ => info.shelves.push(Shelf { title, cards }),
            }
        } else if let Some(d) = s.get("musicDescriptionShelfRenderer") {
            let full = text(d.get("description"));
            if full.len() > info.description.len() {
                info.description = full;
            }
        }
    }
    Ok(info)
}

// ---------------------------------------------------------------------------
// Albums and playlists
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Collection {
    pub title: String,
    /// "Album" | "Single" | "EP" | "Playlist"
    pub kind: String,
    pub artist: String,
    pub artist_browse: Option<String>,
    pub year: String,
    pub cover: Option<String>,
    pub description: String,
    pub browse_id: String,
    pub tracks: Vec<Track>,
}

/// Album (MPREb_…) or playlist (VL…) page, with up to ~500 tracks.
pub async fn collection(browse_id: &str) -> AppResult<Collection> {
    let v = post("browse", serde_json::json!({ "browseId": browse_id })).await?;
    let tc = v.pointer("/contents/twoColumnBrowseResultsRenderer").ok_or_else(|| AppError::NotFound("page format changed".into()))?;
    let header = tc
        .pointer("/tabs/0/tabRenderer/content/sectionListRenderer/contents/0")
        .and_then(|h| h.get("musicResponsiveHeaderRenderer").or_else(|| h.get("musicEditablePlaylistDetailHeaderRenderer").and_then(|e| e.pointer("/header/musicResponsiveHeaderRenderer"))))
        .cloned()
        .unwrap_or(Value::Null);
    let title = text(header.get("title"));
    let subtitle = text(header.get("subtitle"));
    let strap = runs(header.get("straplineTextOne"));
    let artist = strap.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect::<String>();
    let artist_browse = strap.iter().find_map(nav_browse);
    let cover = best_thumb(header.get("thumbnail"));
    let mut parts = subtitle.split(" • ");
    let kind = parts.next().unwrap_or("Playlist").trim().to_string();
    let year = subtitle.split(" • ").find(|p| p.trim().len() == 4 && p.trim().parse::<u32>().is_ok()).unwrap_or("").trim().to_string();
    let description = header.pointer("/description/musicDescriptionShelfRenderer/description").map(|d| text(Some(d))).unwrap_or_default();
    let is_album = browse_id.starts_with("MPRE");
    let album_ctx = if is_album { Some((title.as_str(), cover.as_deref())) } else { None };
    let artist_ctx = if is_album && !artist.is_empty() { Some((artist.as_str(), artist_browse.as_deref())) } else { None };

    let shelf = tc.pointer("/secondaryContents/sectionListRenderer/contents/0").cloned().unwrap_or(Value::Null);
    let shelf = shelf.get("musicShelfRenderer").or_else(|| shelf.get("musicPlaylistShelfRenderer")).cloned().unwrap_or(Value::Null);
    let mut items: Vec<Value> = shelf.get("contents").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut tracks: Vec<Track> = Vec::new();
    for _ in 0..5 {
        let cont = items.last().and_then(|i| i.pointer("/continuationItemRenderer/continuationEndpoint/continuationCommand/token")).and_then(Value::as_str).map(str::to_string);
        tracks.extend(items.iter().filter_map(|i| i.get("musicResponsiveListItemRenderer")).filter_map(|m| list_item(m, artist_ctx, album_ctx)).filter_map(|c| c.track));
        let Some(token) = cont else { break };
        let next = post("browse", serde_json::json!({ "continuation": token })).await?;
        items = next
            .pointer("/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            break;
        }
    }
    if is_album {
        // One cover for the whole album.
        for t in tracks.iter_mut() {
            t.thumbnail = cover.clone().or(t.thumbnail.take());
        }
    }
    for (i, t) in tracks.iter_mut().enumerate() {
        if let Some(v) = t.versions.first_mut() {
            v.rank = i;
        }
    }
    if tracks.is_empty() {
        return Err(AppError::EmptyResults);
    }
    Ok(Collection { title, kind, artist, artist_browse, year, cover, description, browse_id: browse_id.to_string(), tracks })
}

/// Map a YouTube Music URL to a page id: albums, playlists, channels.
pub fn browse_id_from_url(url: &str) -> Option<String> {
    if let Some(i) = url.find("/browse/") {
        return Some(url[i + 8..].split(['?', '&', '#', '/']).next()?.to_string());
    }
    if let Some(i) = url.find("list=") {
        let id = url[i + 5..].split(['&', '#']).next()?;
        if !id.is_empty() && !id.starts_with("RD") {
            return Some(format!("VL{id}"));
        }
    }
    if let Some(i) = url.find("/channel/") {
        return Some(url[i + 9..].split(['?', '&', '#', '/']).next()?.to_string());
    }
    None
}

// ---------------------------------------------------------------------------
// Moods & genres
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Mood {
    pub title: String,
    pub params: String,
    /// "#rrggbb" stripe color.
    pub color: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MoodSection {
    pub title: String,
    pub items: Vec<Mood>,
}

pub async fn moods() -> AppResult<Vec<MoodSection>> {
    let v = post("browse", serde_json::json!({ "browseId": "FEmusic_moods_and_genres" })).await?;
    let sections = v.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(sections
        .iter()
        .filter_map(|s| s.get("gridRenderer"))
        .map(|g| MoodSection {
            title: text(g.pointer("/header/gridHeaderRenderer/title")),
            items: g
                .get("items")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.get("musicNavigationButtonRenderer"))
                        .filter_map(|b| {
                            let color = b.pointer("/solid/leftStripeColor").and_then(Value::as_u64).map(|c| format!("#{:06x}", c & 0xFF_FFFF)).unwrap_or_else(|| "#555555".into());
                            Some(Mood { title: text(b.get("buttonText")), params: b.pointer("/clickCommand/browseEndpoint/params")?.as_str()?.to_string(), color })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .filter(|s| !s.items.is_empty())
        .collect())
}

/// A mood or genre page: shelves of playlists.
pub async fn mood_page(params: &str) -> AppResult<Vec<Shelf>> {
    let v = post("browse", serde_json::json!({ "browseId": "FEmusic_moods_and_genres_category", "params": params })).await?;
    let sections = v.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(sections
        .iter()
        .filter_map(|s| {
            let (title, body) = if let Some(c) = s.get("musicCarouselShelfRenderer") {
                (text(c.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")), c)
            } else if let Some(g) = s.get("gridRenderer") {
                (text(g.pointer("/header/gridHeaderRenderer/title")), g)
            } else {
                return None;
            };
            let cards = shelf_items(body);
            (!cards.is_empty()).then_some(Shelf { title, cards })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_ids_from_urls() {
        assert_eq!(browse_id_from_url("https://music.youtube.com/browse/MPREb_abc").as_deref(), Some("MPREb_abc"));
        assert_eq!(browse_id_from_url("https://music.youtube.com/playlist?list=OLAK5uy_x&si=1").as_deref(), Some("VLOLAK5uy_x"));
        assert_eq!(browse_id_from_url("https://www.youtube.com/channel/UCabc/releases").as_deref(), Some("UCabc"));
        assert_eq!(browse_id_from_url("https://music.youtube.com/watch?v=x&list=RDAMVMx"), None);
        assert!(upscale("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj", 544).ends_with("=w544-h544-l90-rj"));
    }

    #[test]
    #[ignore]
    fn live_ytm() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let t0 = Instant::now();
        let ex = rt.block_on(search_extras("tame impala")).unwrap();
        println!("extras in {:?}: top={:?} artists={} albums={} playlists={}", t0.elapsed(), ex.top.as_ref().map(|c| (&c.kind, &c.title)), ex.artists.len(), ex.albums.len(), ex.playlists.len());
        assert!(ex.top.is_some() && !ex.artists.is_empty() && !ex.albums.is_empty());
        let t1 = Instant::now();
        let (bid, _) = rt.block_on(find_artist("Tame Impala")).unwrap();
        let a = rt.block_on(artist(&bid)).unwrap();
        println!("artist in {:?}: {} · {} · top {} · albums {} · singles {} · related {} · banner {}", t1.elapsed(), a.name, a.audience, a.top_songs.len(), a.albums.len(), a.singles.len(), a.related.len(), a.banner.is_some());
        for t in a.top_songs.iter().take(3) {
            println!("   {} — {} [{:?}] {:?}", t.title, t.artist, t.versions[0].album, t.duration);
        }
        assert!(a.top_songs.len() >= 5 && !a.albums.is_empty() && !a.related.is_empty());
        let t2 = Instant::now();
        let al = rt.block_on(collection(a.albums[0].browse_id.as_deref().unwrap())).unwrap();
        println!("album in {:?}: {} ({}) by {} {} — {} tracks, first {} {:?}", t2.elapsed(), al.title, al.kind, al.artist, al.year, al.tracks.len(), al.tracks[0].title, al.tracks[0].duration);
        assert!(al.tracks.iter().all(|t| t.artist == al.artist && t.duration.is_some()));
        if let Some(all) = &a.all_songs {
            let pl = rt.block_on(collection(all)).unwrap();
            println!("all songs: {} tracks", pl.tracks.len());
            assert!(pl.tracks.len() > 100, "continuations");
        }
        let m = rt.block_on(moods()).unwrap();
        println!("moods: {:?}", m.iter().map(|s| (&s.title, s.items.len())).collect::<Vec<_>>());
        let page = rt.block_on(mood_page(&m[0].items[0].params)).unwrap();
        println!("mood page {}: {:?}", m[0].items[0].title, page.iter().map(|s| (&s.title, s.cards.len())).collect::<Vec<_>>());
        let songs = rt.block_on(search_songs("GBARL9300135")).unwrap();
        assert_eq!(songs[0].title, "Never Gonna Give You Up");
    }
}
