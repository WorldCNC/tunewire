//! Multi-source search: normalization, fuzzy matching, per-platform
//! deduplication, cross-platform grouping, ranking and a short-lived cache.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{Platform, Track, TrackVersion};
use crate::ytdlp;

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// Words that mark a *different recording/arrangement*. Two titles that differ
/// in these are never merged ("Song" vs "Song (Remix)").
const VARIANT_MARKERS: &[&str] = &[
    "remix", "rmx", "edit", "mix", "acoustic", "instrumental", "cover", "extended", "vip", "slowed",
    "sped", "nightcore", "bootleg", "flip", "rework", "karaoke", "reverb", "8d", "mashup", "unplugged",
    "orchestral", "piano", "demo",
];

/// Words that carry no identity — packaging noise added by uploaders.
const NOISE_WORDS: &[&str] = &[
    "official", "video", "audio", "lyrics", "lyric", "visualizer", "visualiser", "remaster",
    "remastered", "live", "hd", "hq", "4k", "1080p", "mv", "m", "v", "explicit", "clean", "version",
    "music", "full", "song", "color", "coded", "free", "download", "out", "now", "premiere", "topic",
    "original", "single", "album", "stereo", "mono", "ost", "soundtrack", "from", "the", "release",
    "ncs", "copyright", "high", "quality", "new", "subtitulado", "letra", "legendado", "dir", "by",
];

/// Lowercase, strip punctuation/diacritic-ish symbols and collapse spaces.
/// Unicode letters and digits (Hebrew, Cyrillic, CJK…) are preserved.
pub fn norm_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars().flat_map(char::to_lowercase) {
        let ch = fold_char(ch);
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if ch == '\'' || ch == '’' || ch == '`' {
            // "don't" → "dont"
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim_end().to_string()
}

/// Cheap accent folding for the most common Latin diacritics.
fn fold_char(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'ñ' | 'ń' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'ś' | 'š' | 'ß' => 's',
        'ź' | 'ž' | 'ż' => 'z',
        'ł' => 'l',
        _ => c,
    }
}

fn tokens(s: &str) -> Vec<String> {
    norm_text(s).split_whitespace().map(str::to_string).collect()
}

fn has_marker(toks: &[String]) -> bool {
    toks.iter().any(|t| VARIANT_MARKERS.contains(&t.as_str()))
}

fn is_feat_token(t: &str) -> bool {
    matches!(t, "feat" | "ft" | "featuring" | "with" | "prod" | "produced")
}

/// Is a bracketed/dash-separated segment pure packaging noise?
fn is_noise_segment(seg: &str, square: bool) -> bool {
    let toks = tokens(seg);
    if toks.is_empty() {
        return true;
    }
    if is_feat_token(&toks[0]) {
        return true;
    }
    if has_marker(&toks) {
        return false;
    }
    let all_noise = toks
        .iter()
        .all(|t| NOISE_WORDS.contains(&t.as_str()) || t.chars().all(|c| c.is_ascii_digit()));
    let any_strong = toks.iter().any(|t| {
        matches!(
            t.as_str(),
            "official" | "lyrics" | "lyric" | "video" | "audio" | "visualizer" | "remaster" | "remastered" | "live" | "hd" | "4k" | "mv"
        )
    });
    // Square brackets are almost always label tags ("[NCS Release]", "[HZRX]").
    all_noise || any_strong || square
}

/// Remove packaging noise from a title while preserving its original casing.
pub fn strip_noise(title: &str) -> String {
    // 1. bracketed groups
    let mut out = String::with_capacity(title.len());
    let chars: Vec<char> = title.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let close = match chars[i] {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '【' => Some('】'),
            '「' => Some('」'),
            _ => None,
        };
        if let Some(close) = close {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == close) {
                let inner: String = chars[i + 1..i + 1 + rel].iter().collect();
                if !is_noise_segment(&inner, chars[i] == '[' || chars[i] == '【') {
                    out.push(chars[i]);
                    out.push_str(&inner);
                    out.push(close);
                }
                i += rel + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }

    // 2. trailing " - Remastered 2011" / " | Official Video" segments
    let mut segs: Vec<&str> = split_seps(&out);
    while segs.len() > 1 && is_noise_segment(segs[segs.len() - 1], false) {
        segs.pop();
    }
    let mut s = segs.join(" - ");

    // 3. bare "feat. X" / "ft X" up to end of string
    let lower = s.to_lowercase();
    for pat in [" feat. ", " feat ", " ft. ", " ft ", " featuring "] {
        if let Some(pos) = lower.find(pat) {
            s.truncate(pos);
            break;
        }
    }

    // 4. bare trailing noise words ("Song Official Video")
    let mut words: Vec<&str> = s.split_whitespace().collect();
    while words.len() > 1 {
        let w = norm_text(words[words.len() - 1]);
        if matches!(w.as_str(), "official" | "video" | "audio" | "lyrics" | "lyric" | "hd" | "4k" | "hq" | "mv" | "visualizer") {
            words.pop();
        } else {
            break;
        }
    }
    let s = words.join(" ");
    let s = s.trim_matches(|c: char| c.is_whitespace() || c == '-' || c == '|' || c == '–' || c == '—');
    if s.is_empty() {
        title.trim().to_string()
    } else {
        s.to_string()
    }
}

fn split_seps(s: &str) -> Vec<&str> {
    let mut parts = vec![s];
    for sep in [" - ", " – ", " — ", " | ", " ~ "] {
        parts = parts.into_iter().flat_map(|p| p.split(sep)).collect();
    }
    parts.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// Strip auto-generated channel decorations: "Artist - Topic", "ArtistVEVO", "Artist Official".
pub fn clean_uploader(u: &str) -> String {
    let mut s = u.trim().to_string();
    for suf in [" - Topic", " Topic", "VEVO", "Vevo", " Official", " (Official)", " Music"] {
        if s.len() > suf.len() + 1 && s.ends_with(suf) {
            s.truncate(s.len() - suf.len());
        }
    }
    s.trim().to_string()
}

/// Split "Artist - Title" uploads; fall back to the uploader for the artist.
pub fn split_artist_title(raw_title: &str, uploader: &str) -> (String, String) {
    for sep in [" - ", " – ", " — "] {
        if let Some(pos) = raw_title.find(sep) {
            let left = raw_title[..pos].trim();
            let right = raw_title[pos + sep.len()..].trim();
            // "Song - Remastered" is not "Artist - Song"
            if !left.is_empty() && !right.is_empty() && !is_noise_segment(&strip_noise(right), false) {
                return (left.to_string(), right.to_string());
            }
        }
    }
    (clean_uploader(uploader), raw_title.trim().to_string())
}

fn artist_tokens(artist: &str) -> HashSet<String> {
    tokens(artist)
        .into_iter()
        .filter(|t| !matches!(t.as_str(), "x" | "and" | "feat" | "ft" | "featuring" | "vs" | "the" | "with" | "official"))
        .collect()
}

fn primary_artist(artist: &str) -> String {
    let lower = artist.to_lowercase();
    let mut end = lower.len();
    for sep in [",", " & ", " x ", " feat", " ft.", " ft ", " and ", " vs", " / "] {
        if let Some(p) = lower.find(sep) {
            end = end.min(p);
        }
    }
    norm_text(&artist[..end.min(artist.len())])
}

/// Stable identity key for a logical track.
pub fn track_key(artist: &str, title: &str) -> String {
    format!("{}::{}", primary_artist(artist), norm_text(&strip_noise(title)))
}

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Norm {
    title: String,
    title_tokens: Vec<String>,
    artist: HashSet<String>,
    markers: HashSet<String>,
    /// All words of the segments that name a variant: "(Valmer Edit)" → {valmer, edit}.
    variant: HashSet<String>,
    duration: Option<f64>,
    preview: bool,
}

/// Words of every bracketed / dash-separated segment containing a variant marker.
fn variant_words(raw_title: &str) -> HashSet<String> {
    let mut segs: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for ch in raw_title.chars() {
        match ch {
            '(' | '[' | '{' => {
                if depth == 0 {
                    segs.push(std::mem::take(&mut cur));
                }
                depth += 1;
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    segs.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(ch),
        }
    }
    segs.push(cur);
    let mut out = HashSet::new();
    for seg in segs.iter().flat_map(|s| split_seps(s).into_iter().map(str::to_string).collect::<Vec<_>>()) {
        let toks = tokens(&seg);
        if has_marker(&toks) {
            out.extend(toks.into_iter().filter(|t| !NOISE_WORDS.contains(&t.as_str()) && !is_feat_token(t)));
        }
    }
    out
}

impl Norm {
    fn of(v: &TrackVersion) -> Norm {
        let title = norm_text(&strip_noise(&v.title));
        let title_tokens: Vec<String> = title.split_whitespace().map(str::to_string).collect();
        let markers = title_tokens
            .iter()
            .filter(|t| VARIANT_MARKERS.contains(&t.as_str()))
            .cloned()
            .collect();
        Norm {
            title,
            title_tokens,
            artist: artist_tokens(&v.artist),
            markers,
            variant: variant_words(&v.title),
            duration: v.duration,
            preview: v.preview,
        }
    }
}

fn jaccard<T: Eq + std::hash::Hash>(a: &HashSet<T>, b: &HashSet<T>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}

fn title_similarity(a: &Norm, b: &Norm) -> f64 {
    let sa: HashSet<&String> = a.title_tokens.iter().collect();
    let sb: HashSet<&String> = b.title_tokens.iter().collect();
    jaccard(&sa, &sb).max(strsim::normalized_levenshtein(&a.title, &b.title))
}

fn artists_match(a: &Norm, b: &Norm) -> bool {
    if a.artist.is_empty() || b.artist.is_empty() {
        return false;
    }
    let inter = a.artist.intersection(&b.artist).count() as f64;
    let min = a.artist.len().min(b.artist.len()) as f64;
    if inter / min >= 0.5 {
        return true;
    }
    // Squashed names: "daftpunk" vs "daft punk"
    let ja: String = {
        let mut v: Vec<&String> = a.artist.iter().collect();
        v.sort();
        v.into_iter().cloned().collect()
    };
    let jb: String = {
        let mut v: Vec<&String> = b.artist.iter().collect();
        v.sort();
        v.into_iter().cloned().collect()
    };
    strsim::normalized_levenshtein(&ja, &jb) >= 0.85
}

/// The heart of deduplication: are these two uploads the same logical song?
fn same_track(a: &Norm, b: &Norm) -> bool {
    if a.markers != b.markers || a.variant != b.variant {
        return false;
    }
    let tsim = title_similarity(a, b);
    if tsim < 0.8 {
        return false;
    }
    let artist_ok = if a.artist.is_empty() || b.artist.is_empty() {
        tsim >= 0.95
    } else {
        artists_match(a, b)
    };
    if !artist_ok {
        return false;
    }
    if let (Some(da), Some(db)) = (a.duration, b.duration) {
        if !a.preview && !b.preview {
            let tol = 7.0f64.max(da.max(db) * 0.04);
            if (da - db).abs() > tol {
                return false;
            }
        }
    }
    true
}

/// Same song title and version (the artist is trusted, e.g. an ISRC match).
pub(crate) fn same_title(a: &TrackVersion, b: &TrackVersion) -> bool {
    let (na, nb) = (Norm::of(a), Norm::of(b));
    na.markers == nb.markers && na.variant == nb.variant && title_similarity(&na, &nb) >= 0.8
}

/// Same recording: title, version, artist and length all agree.
pub(crate) fn same_recording(a: &TrackVersion, b: &TrackVersion) -> bool {
    same_track(&Norm::of(a), &Norm::of(b))
}

fn relevance(query: &[String], n: &Norm) -> f64 {
    if query.is_empty() {
        return 0.0;
    }
    let hay: Vec<&String> = n.title_tokens.iter().chain(n.artist.iter()).collect();
    let matched = query
        .iter()
        .filter(|q| {
            hay.iter().any(|t| {
                *t == *q
                    || (q.len() >= 3 && t.starts_with(q.as_str()))
                    || (q.len() > 3 && strsim::normalized_levenshtein(q, t) >= 0.8)
            })
        })
        .count() as f64;
    let mut score = matched / query.len() as f64;
    // Penalize remixes etc. the user did not ask for.
    let unasked = n.markers.iter().filter(|m| !query.contains(m)).count();
    score -= 0.15 * unasked as f64;
    if n.preview {
        score -= 0.6;
    }
    score
}

// ---------------------------------------------------------------------------
// Grouping + ranking
// ---------------------------------------------------------------------------

pub fn pref_index(pref: &[Platform], p: Platform) -> usize {
    pref.iter().position(|&x| x == p).unwrap_or(pref.len())
}

/// Order versions by user preference (previews always last).
pub fn sort_versions(versions: &mut [TrackVersion], pref: &[Platform]) {
    versions.sort_by_key(|v| (v.preview, pref_index(pref, v.platform)));
}

/// Collapse same-platform duplicates and merge cross-platform versions.
pub fn group_results(query: &str, mut all: Vec<TrackVersion>, pref: &[Platform]) -> Vec<Track> {
    let qtok = tokens(query);
    // Interleave by rank so every platform's top hits seed groups first.
    all.sort_by_key(|v| (v.rank, pref_index(pref, v.platform)));

    struct Group {
        norms: Vec<Norm>,
        versions: Vec<TrackVersion>,
        best_rank: usize,
    }
    let mut groups: Vec<Group> = Vec::new();

    'outer: for v in all {
        let n = Norm::of(&v);
        for g in groups.iter_mut() {
            if g.norms.iter().any(|gn| same_track(gn, &n)) {
                if let Some(existing) = g.versions.iter_mut().find(|e| e.platform == v.platform) {
                    // Same song, same platform → duplicate. Keep the better upload.
                    if existing.preview && !v.preview {
                        *existing = v;
                    }
                } else {
                    g.best_rank = g.best_rank.min(v.rank);
                    g.norms.push(n);
                    g.versions.push(v);
                }
                continue 'outer;
            }
        }
        groups.push(Group { best_rank: v.rank, norms: vec![n], versions: vec![v] });
    }

    let plen = pref.len().max(1) as f64;
    let mut tracks: Vec<Track> = groups
        .into_iter()
        .map(|mut g| {
            let rel = g.norms.iter().map(|n| relevance(&qtok, n)).fold(f64::MIN, f64::max);
            sort_versions(&mut g.versions, pref);
            let best_pref = g.versions.iter().map(|v| pref_index(pref, v.platform)).min().unwrap_or(pref.len());
            let all_preview = g.versions.iter().all(|v| v.preview);
            let score = rel * 2.0
                + 0.6 / (1.0 + g.best_rank as f64)
                + 0.25 * (plen - best_pref as f64).max(0.0) / plen
                + 0.08 * (g.versions.len() as f64 - 1.0)
                - if all_preview { 0.5 } else { 0.0 };
            build_track(g.versions, score)
        })
        .collect();
    tracks.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    tracks
}

/// Pick the best metadata among a group's versions for display.
pub fn build_track(versions: Vec<TrackVersion>, score: f64) -> Track {
    // YouTube Music has the cleanest titles; SoundCloud/YouTube carry real
    // uploader names (YouTube Music flat results only have inferred artists).
    let title_order = [Platform::Spotify, Platform::Ytmusic, Platform::Soundcloud, Platform::Youtube];
    let artist_order = [Platform::Spotify, Platform::Soundcloud, Platform::Youtube, Platform::Ytmusic];
    let pick = |order: &[Platform]| {
        order
            .iter()
            .find_map(|p| versions.iter().find(|v| v.platform == *p && !v.artist.is_empty()))
            .unwrap_or(&versions[0])
    };
    let title = strip_noise(&pick(&title_order).title);
    let artist = pick(&artist_order).artist.clone();
    let duration = versions.iter().filter(|v| !v.preview).find_map(|v| v.duration).or(versions[0].duration);
    let thumbnail = versions
        .iter()
        .find(|v| matches!(v.platform, Platform::Ytmusic | Platform::Youtube) && v.thumbnail.is_some())
        .or_else(|| versions.iter().find(|v| v.thumbnail.is_some()))
        .and_then(|v| v.thumbnail.clone());
    Track { key: track_key(&artist, &title), title, artist, duration, thumbnail, versions, score }
}

// ---------------------------------------------------------------------------
// yt-dlp entry parsing
// ---------------------------------------------------------------------------

fn sv(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty())
}

fn yt_thumb(id: &str) -> Option<String> {
    (!id.is_empty()).then(|| format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"))
}

/// Artwork URL at a reasonable size. SoundCloud lists sizes up to "original",
/// which can be a multi-megabyte file (too heavy for the UI, rejected by
/// Discord), so prefer its 500×500 / 300×300 renditions.
pub fn pick_thumbnail(e: &Value) -> Option<String> {
    let thumbs = e.get("thumbnails").and_then(Value::as_array);
    if let Some(list) = thumbs {
        for want in ["t500x500", "t300x300", "large"] {
            if let Some(u) = list.iter().find(|t| t.get("id").and_then(Value::as_str) == Some(want)).and_then(|t| sv(t, "url")) {
                return Some(u);
            }
        }
    }
    let fix = |u: String| u.replace("-original.", "-t500x500.");
    sv(e, "thumbnail").map(fix).or_else(|| {
        thumbs.and_then(|a| a.iter().rev().filter(|t| t.get("id").and_then(Value::as_str) != Some("original")).find_map(|t| sv(t, "url"))).map(fix)
    })
}

/// Convert one flat-playlist entry into a TrackVersion for the given platform.
/// "https://soundcloud.com/artist/some-track-name" → "Some Track Name"
fn title_from_slug(url: &str) -> Option<String> {
    let slug = url.split(['?', '#']).next()?.trim_end_matches('/').rsplit('/').next()?;
    let words: Vec<String> = slug.split(['-', '_']).filter(|w| !w.is_empty()).map(title_case).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

pub fn parse_entry(e: &Value, platform: Platform, rank: usize) -> Option<TrackVersion> {
    let mut partial = false;
    let raw_title = match sv(e, "track").or_else(|| sv(e, "title")) {
        Some(t) => t,
        None => {
            partial = true;
            title_from_slug(&sv(e, "webpage_url").or_else(|| sv(e, "url"))?)?
        }
    };
    if raw_title == "[Deleted video]" || raw_title == "[Private video]" {
        return None;
    }
    let id = sv(e, "id").unwrap_or_default();
    let duration = e.get("duration").and_then(Value::as_f64);
    let uploader = sv(e, "artist")
        .or_else(|| e.get("artists").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).map(str::to_string))
        .or_else(|| sv(e, "channel"))
        .or_else(|| sv(e, "uploader"))
        .unwrap_or_default();
    // A real "artist" field (YouTube Music full extraction) is authoritative;
    // otherwise uploads often look like "Artist - Title".
    let (artist, title) = if sv(e, "artist").is_some() {
        (clean_uploader(&uploader), raw_title.clone())
    } else {
        split_artist_title(&raw_title, &uploader)
    };

    let (url, thumbnail) = match platform {
        Platform::Soundcloud => {
            let url = sv(e, "webpage_url").or_else(|| sv(e, "url"))?;
            (url, pick_thumbnail(e))
        }
        Platform::Youtube | Platform::Ytmusic => {
            let url = if !id.is_empty() {
                if platform == Platform::Ytmusic {
                    format!("https://music.youtube.com/watch?v={id}")
                } else {
                    format!("https://www.youtube.com/watch?v={id}")
                }
            } else {
                sv(e, "url")?
            };
            (url, yt_thumb(&id).or_else(|| pick_thumbnail(e)))
        }
        Platform::Spotify | Platform::Local => (sv(e, "webpage_url").or_else(|| sv(e, "url"))?, pick_thumbnail(e)),
    };
    let preview = platform == Platform::Soundcloud && duration.map(|d| (d - 30.0).abs() < 0.6).unwrap_or(false);
    let artist_url = sv(e, "uploader_url").or_else(|| sv(e, "channel_url"));
    let album = sv(e, "album");
    Some(TrackVersion { platform, url, id, title, artist, duration, thumbnail, preview, partial, artist_url, album, rank, matched_for: None, isrc_match: false })
}

pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn search_target(platform: Platform, query: &str, limit: usize) -> (String, Option<usize>) {
    match platform {
        Platform::Soundcloud => (format!("scsearch{limit}:{query}"), None),
        Platform::Youtube => (format!("ytsearch{limit}:{query}"), None),
        Platform::Ytmusic => (
            format!("https://music.youtube.com/search?q={}#songs", percent_encode(query)),
            Some(limit),
        ),
        Platform::Spotify | Platform::Local => unreachable!("not a yt-dlp search"),
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_MAX: usize = 128;

#[derive(Default)]
pub struct SearchCache {
    pub(crate) map: Mutex<HashMap<String, (Instant, Vec<TrackVersion>)>>,
}

impl SearchCache {
    fn get(&self, key: &str) -> Option<Vec<TrackVersion>> {
        let map = self.map.lock().unwrap();
        map.get(key).filter(|(t, _)| t.elapsed() < CACHE_TTL).map(|(_, v)| v.clone())
    }

    fn put(&self, key: String, v: Vec<TrackVersion>) {
        let mut map = self.map.lock().unwrap();
        map.retain(|_, (t, _)| t.elapsed() < CACHE_TTL);
        if map.len() >= CACHE_MAX {
            if let Some(oldest) = map.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone()) {
                map.remove(&oldest);
            }
        }
        map.insert(key, (Instant::now(), v));
    }
}

// ---------------------------------------------------------------------------
// Public search API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub tracks: Vec<Track>,
    pub warnings: Vec<String>,
    pub cached: bool,
}

/// Native web APIs on/off (offline tests turn them off to exercise yt-dlp).
pub static NATIVE_APIS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn native_apis() -> bool {
    NATIVE_APIS.load(std::sync::atomic::Ordering::Relaxed)
}

async fn search_platform(platform: Platform, query: String, limit: usize, cache: &SearchCache) -> AppResult<(Vec<TrackVersion>, bool)> {
    let key = format!("{platform:?}|{}|{limit}", norm_text(&query));
    if let Some(hit) = cache.get(&key) {
        return Ok((hit, true));
    }
    if platform == Platform::Local {
        return Ok((crate::localfiles::search(&query, limit), false));
    }
    if platform == Platform::Spotify {
        // Spotify's own web search (instant); spotDL only if that fails.
        let versions = match crate::spotify::search(&query, limit).await {
            Ok(v) => v,
            Err(e) if crate::spotdl::available().await => {
                let v = crate::spotdl::save(&query).await.map_err(|_| e)?;
                v
            }
            Err(e) => return Err(e),
        };
        if versions.is_empty() {
            return Err(AppError::EmptyResults);
        }
        cache.put(key, versions.clone());
        return Ok((versions, false));
    }
    // The platforms' own web APIs first (~0.5 s, full metadata); yt-dlp if they fail.
    let native: AppResult<Vec<TrackVersion>> = match platform {
        _ if !native_apis() => Err(AppError::Unsupported("native APIs off".into())),
        Platform::Ytmusic => crate::ytm::search_songs(&query).await.map(|mut v| {
            v.truncate(limit);
            v
        }),
        Platform::Youtube => crate::ytm::search_videos(&query, limit).await.map(|es| es.iter().enumerate().filter_map(|(i, e)| parse_entry(e, platform, i)).collect()),
        Platform::Soundcloud => crate::soundcloud::search(&query, limit).await,
        _ => Err(AppError::Unsupported("no native search".into())),
    };
    if let Some(versions) = native.ok().filter(|v| !v.is_empty()) {
        cache.put(key, versions.clone());
        return Ok((versions, false));
    }
    let (target, end) = search_target(platform, &query, limit);
    let json = ytdlp::flat(&target, end).await?;
    let entries = json.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
    let versions: Vec<TrackVersion> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| parse_entry(e, platform, i))
        .collect();
    cache.put(key, versions.clone());
    Ok((versions, false))
}

/// YouTube Music flat results carry no artist/duration. Borrow them from the
/// YouTube results when the same video id appears there, otherwise infer the
/// artist from query words that are not part of the title.
fn enrich_ytmusic(ytm: &mut [TrackVersion], yt: &[TrackVersion], query: &str) {
    let by_id: HashMap<&str, &TrackVersion> = yt.iter().map(|v| (v.id.as_str(), v)).collect();
    for v in ytm.iter_mut() {
        if let Some(src) = by_id.get(v.id.as_str()) {
            if v.artist.is_empty() {
                v.artist = src.artist.clone();
            }
            v.duration = v.duration.or(src.duration);
            v.artist_url = v.artist_url.clone().or(src.artist_url.clone());
        }
        if v.artist.is_empty() {
            let title_toks: HashSet<String> = tokens(&v.title).into_iter().collect();
            let rest: Vec<&str> = query
                .split_whitespace()
                .filter(|w| {
                    let n = norm_text(w);
                    !n.is_empty() && !title_toks.contains(&n)
                })
                .collect();
            if !rest.is_empty() && rest.len() <= 4 {
                v.artist = rest.iter().map(|w| title_case(w)).collect::<Vec<_>>().join(" ");
            }
            // Guessed metadata: completed in the background (see enrich_ytmusic_bg).
            v.partial = true;
        } else if v.duration.is_none() {
            v.partial = true;
        }
    }
}

fn single_platform(source: &str, spotify: bool) -> AppResult<Platform> {
    match source {
        "soundcloud" => Ok(Platform::Soundcloud),
        "ytmusic" => Ok(Platform::Ytmusic),
        "youtube" => Ok(Platform::Youtube),
        "local" => Ok(Platform::Local),
        "spotify" if spotify => Ok(Platform::Spotify),
        "spotify" => Err(AppError::Unsupported("Spotify support is turned off (Settings → Spotify)".into())),
        other => Err(AppError::Invalid(format!("unknown source '{other}'"))),
    }
}

fn collect_results(query: &str, results: Vec<(Platform, AppResult<(Vec<TrackVersion>, bool)>)>, pref: &[Platform], hidden: Option<Platform>) -> AppResult<SearchResponse> {
    let mut warnings = Vec::new();
    let mut first_err = None;
    let mut cached = true;
    let mut by_platform: HashMap<Platform, Vec<TrackVersion>> = HashMap::new();
    for (p, r) in results {
        match r {
            Ok((v, c)) => {
                cached &= c;
                by_platform.insert(p, v);
            }
            Err(e) => {
                warnings.push(format!("{}: {}", p.label(), e));
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    if by_platform.is_empty() {
        return Err(first_err.unwrap_or(AppError::EmptyResults));
    }
    let yt = by_platform.get(&Platform::Youtube).cloned().unwrap_or_default();
    if let Some(ytm) = by_platform.get_mut(&Platform::Ytmusic) {
        enrich_ytmusic(ytm, &yt, query);
    }
    if let Some(h) = hidden {
        by_platform.remove(&h);
        warnings.retain(|w| !w.starts_with(h.label()));
    }
    let all: Vec<TrackVersion> = by_platform.into_values().flatten().collect();
    if all.is_empty() {
        return Err(AppError::EmptyResults);
    }
    Ok(SearchResponse { tracks: group_results(query, all, pref), warnings, cached })
}

fn title_case(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) if w.chars().all(|x| !x.is_uppercase()) => f.to_uppercase().chain(c).collect(),
        _ => w.to_string(),
    }
}

pub async fn search(
    query: String,
    source: String,
    limit: usize,
    pref: Vec<Platform>,
    cache: std::sync::Arc<SearchCache>,
    include_youtube: bool,
    spotify: bool,
) -> AppResult<SearchResponse> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Err(AppError::Invalid("type something to search".into()));
    }
    let limit = limit.clamp(1, 40);
    let platforms = if source == "all" || source.is_empty() {
        let mut v = vec![Platform::Ytmusic, Platform::Soundcloud];
        if include_youtube {
            v.push(Platform::Youtube);
        }
        if spotify {
            v.push(Platform::Spotify);
        }
        if crate::localfiles::has_tracks() {
            v.push(Platform::Local);
        }
        v
    } else {
        vec![single_platform(&source, spotify)?]
    };
    // Each platform search runs in its own task; the yt-dlp semaphore caps
    // how many processes actually run at once.
    let mut tasks = Vec::new();
    let platforms_len = platforms.len();
    // YouTube Music results lack artist/duration; a parallel YouTube search
    // usually contains the same videos with that metadata.
    let hidden: Option<Platform> = None;
    let mut platforms = platforms;
    if let Some(h) = hidden {
        platforms.push(h);
    }
    for p in platforms {
        let (q, cache) = (q.clone(), cache.clone());
        let all = platforms_len > 1;
        tasks.push((p, tauri::async_runtime::spawn(async move {
            if p == Platform::Spotify && all {
                // Never let a slow (rate-limited) Spotify lookup hold up the other sources.
                tokio::time::timeout(std::time::Duration::from_secs(15), search_platform(p, q, limit, &cache))
                    .await
                    .unwrap_or_else(|_| Err(AppError::Timeout("Spotify (spotDL) is slow right now — add your own Spotify app credentials in Settings".into())))
            } else {
                search_platform(p, q, limit, &cache).await
            }
        })));
    }
    let mut results = Vec::new();
    for (p, t) in tasks {
        let r = t.await.unwrap_or_else(|e| Err(AppError::YtDlp(e.to_string())));
        results.push((p, r));
    }
    collect_results(&q, results, &pref, hidden)
}

/// Versions whose metadata was guessed (YouTube Music) and should be completed.
pub fn needs_completion(tracks: &[Track]) -> Vec<String> {
    tracks
        .iter()
        .flat_map(|t| t.versions.iter())
        .filter(|v| v.partial && v.platform == Platform::Ytmusic)
        .map(|v| v.url.clone())
        .take(8)
        .collect()
}

/// Patch cached search results with completed metadata.
pub fn apply_metadata(cache: &SearchCache, metas: &[crate::ytdlp::Meta]) {
    let mut map = cache.map.lock().unwrap();
    for (_, (_, versions)) in map.iter_mut() {
        for v in versions.iter_mut() {
            if let Some(m) = metas.iter().find(|m| m.url == v.url) {
                m.apply(v);
            }
        }
    }
}

/// Find a playable equivalent of `artist - title` on non-Spotify platforms,
/// trying them in preference order. Used for Spotify and as a last-resort
/// fallback when every known version fails.
pub async fn resolve_equivalent(
    artist: &str,
    title: &str,
    duration: Option<f64>,
    pref: &[Platform],
    exclude: &[Platform],
    cache: &std::sync::Arc<SearchCache>,
) -> AppResult<TrackVersion> {
    let target = TrackVersion {
        platform: Platform::Spotify,
        url: String::new(),
        id: String::new(),
        title: title.to_string(),
        artist: artist.to_string(),
        duration,
        thumbnail: None,
        preview: false,
        partial: false,
        artist_url: None,
        album: None,
        rank: 0,
        matched_for: None,
        isrc_match: false,
    };
    let tn = Norm::of(&target);
    let query = format!("{artist} {}", strip_noise(title)).trim().to_string();
    let mut order: Vec<Platform> = pref
        .iter()
        .copied()
        .filter(|p| !matches!(p, Platform::Spotify | Platform::Local) && !exclude.contains(p))
        .collect();
    for p in [Platform::Ytmusic, Platform::Soundcloud, Platform::Youtube] {
        if !order.contains(&p) && !exclude.contains(&p) {
            order.push(p);
        }
    }
    // Search every candidate platform concurrently, then judge in preference order.
    let tasks: Vec<_> = order
        .into_iter()
        .map(|p| {
            let (q, cache) = (query.clone(), cache.clone());
            (p, tauri::async_runtime::spawn(async move { search_platform(p, q, 8, &cache).await }))
        })
        .collect();
    let mut last_err = AppError::EmptyResults;
    for (p, task) in tasks {
        match task.await.unwrap_or_else(|e| Err(AppError::YtDlp(e.to_string()))) {
            Ok((mut found, _)) => {
                if p == Platform::Ytmusic {
                    enrich_ytmusic(&mut found, &[], &query);
                }
                // Strict match first, then a looser title-only match.
                let strict = found.iter().find(|v| !v.preview && same_track(&tn, &Norm::of(v)));
                let loose = found.iter().find(|v| {
                    let n = Norm::of(v);
                    !v.preview && n.markers == tn.markers && n.variant == tn.variant && title_similarity(&tn, &n) >= 0.85
                });
                if let Some(v) = strict.or(loose) {
                    return Ok(v.clone());
                }
            }
            Err(e) => last_err = e,
        }
    }
    Err(match last_err {
        AppError::EmptyResults => AppError::Unavailable(format!("no playable match for “{artist} – {title}”")),
        e => e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(p: Platform, title: &str, artist: &str, d: Option<f64>, rank: usize) -> TrackVersion {
        TrackVersion { platform: p, url: format!("u{rank}{title}"), id: String::new(), title: title.into(), artist: artist.into(), duration: d, thumbnail: None, preview: false, partial: false, artist_url: None, album: None, rank, matched_for: None, isrc_match: false }
    }

    #[test]
    fn strips_noise() {
        assert_eq!(strip_noise("One More Time (Official Video)"), "One More Time");
        assert_eq!(strip_noise("Song [NCS Release]"), "Song");
        assert_eq!(strip_noise("Song (Valmer Edit) [HZRX]"), "Song (Valmer Edit)");
        assert_eq!(strip_noise("Song - Remastered 2011"), "Song");
        assert_eq!(strip_noise("Song ft. Someone"), "Song");
        assert_eq!(strip_noise("Song (feat. Someone) (Lyrics)"), "Song");
        assert_eq!(strip_noise("(Don't Fear) The Reaper"), "(Don't Fear) The Reaper");
    }

    #[test]
    fn splits_artist() {
        let (a, t) = split_artist_title("Daft Punk - One More Time (Official Video)", "Daft Punk");
        assert_eq!((a.as_str(), t.as_str()), ("Daft Punk", "One More Time (Official Video)"));
        assert_eq!(clean_uploader("Daft Punk - Topic"), "Daft Punk");
        assert_eq!(clean_uploader("RickAstleyVEVO"), "RickAstley");
    }

    #[test]
    fn groups_and_dedupes() {
        let all = vec![
            v(Platform::Youtube, "One More Time (Official Video)", "Daft Punk", Some(322.0), 0),
            v(Platform::Youtube, "One More Time (Official Audio)", "Daft Punk", Some(321.0), 1),
            v(Platform::Soundcloud, "One More Time", "Daft Punk", Some(322.6), 0),
            v(Platform::Ytmusic, "One More Time", "Daft Punk", Some(320.0), 0),
            v(Platform::Soundcloud, "One More Time (Valmer Edit)", "Daft Punk", Some(313.0), 1),
            v(Platform::Ytmusic, "One More Time (Radio Edit)", "Daft Punk", Some(235.0), 1),
        ];
        let pref = [Platform::Ytmusic, Platform::Soundcloud, Platform::Youtube];
        let g = group_results("daft punk one more time", all, &pref);
        let main = &g[0];
        assert_eq!(main.title, "One More Time");
        assert_eq!(main.versions.len(), 3, "{:#?}", g);
        assert_eq!(main.versions[0].platform, Platform::Ytmusic);
        assert_eq!(g.len(), 3, "{:#?}", g);
    }

    #[test]
    fn slug_titles() {
        assert_eq!(title_from_slug("https://soundcloud.com/daftpunk/revolution-909?x=1").as_deref(), Some("Revolution 909"));
        let e = serde_json::json!({ "url": "https://soundcloud.com/daftpunk/da-funk" });
        let v = parse_entry(&e, Platform::Soundcloud, 0).unwrap();
        assert!(v.partial);
        assert_eq!(v.title, "Da Funk");
    }

    #[test]
    fn different_edits_not_merged() {
        let all = vec![
            v(Platform::Soundcloud, "One More Time (Valmer Edit)", "Daft Punk", None, 0),
            v(Platform::Ytmusic, "One More Time (Radio Edit)", "Daft Punk", None, 0),
            v(Platform::Youtube, "One More Time (Radio Edit) [Official Audio]", "Daft Punk", None, 1),
        ];
        let g = group_results("one more time", all, &Platform::ALL);
        assert_eq!(g.len(), 2, "{g:#?}");
        assert_eq!(g.iter().map(|t| t.versions.len()).max(), Some(2));
    }

    #[test]
    fn different_artists_not_merged() {
        let all = vec![
            v(Platform::Youtube, "Hello", "Adele", Some(295.0), 0),
            v(Platform::Soundcloud, "Hello", "Lionel Richie", Some(250.0), 0),
        ];
        assert_eq!(group_results("hello", all, &Platform::ALL).len(), 2);
    }
}
