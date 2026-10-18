//! Discovery helpers that aren't search: suggestions, charts, lyrics
//! translation and cover colors. All keyless public endpoints.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::http::{agent, USER_AGENT};
use crate::search::percent_encode;

fn get_text(url: &str) -> AppResult<String> {
    let mut r = agent().get(url).header("User-Agent", USER_AGENT).call().map_err(|e| AppError::Network(e.to_string()))?;
    if !r.status().is_success() {
        return Err(AppError::Network(format!("HTTP {}", r.status().as_u16())));
    }
    r.body_mut().with_config().limit(8 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError::Network(e.to_string()))?
}

// ---------------------------------------------------------------------------
// Search suggestions (YouTube's autocomplete)
// ---------------------------------------------------------------------------

pub async fn suggestions(q: String) -> AppResult<Vec<String>> {
    let q = q.trim().to_string();
    if q.is_empty() || q.len() > 200 {
        return Ok(Vec::new());
    }
    blocking(move || {
        let url = format!("https://suggestqueries.google.com/complete/search?client=firefox&ds=yt&oe=utf8&q={}", percent_encode(&q));
        let v: Value = serde_json::from_str(&get_text(&url)?)?;
        Ok(v.get(1).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).take(8).map(str::to_string).collect()).unwrap_or_default())
    })
    .await
}

// ---------------------------------------------------------------------------
// Charts (YouTube Music charts page via its public browse endpoint)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ChartPlaylist {
    pub title: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub group: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChartArtist {
    pub name: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub rank: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Charts {
    pub country: String,
    pub playlists: Vec<ChartPlaylist>,
    pub artists: Vec<ChartArtist>,
}

pub const CHART_COUNTRIES: &[(&str, &str)] = &[
    ("ZZ", "Global"),
    ("AR", "Argentina"),
    ("AU", "Australia"),
    ("AT", "Austria"),
    ("BE", "Belgium"),
    ("BO", "Bolivia"),
    ("BR", "Brazil"),
    ("CA", "Canada"),
    ("CL", "Chile"),
    ("CO", "Colombia"),
    ("CR", "Costa Rica"),
    ("CZ", "Czechia"),
    ("DK", "Denmark"),
    ("DO", "Dominican Republic"),
    ("EC", "Ecuador"),
    ("EG", "Egypt"),
    ("SV", "El Salvador"),
    ("EE", "Estonia"),
    ("FI", "Finland"),
    ("FR", "France"),
    ("DE", "Germany"),
    ("GT", "Guatemala"),
    ("HN", "Honduras"),
    ("HU", "Hungary"),
    ("IS", "Iceland"),
    ("IN", "India"),
    ("ID", "Indonesia"),
    ("IE", "Ireland"),
    ("IL", "Israel"),
    ("IT", "Italy"),
    ("JP", "Japan"),
    ("KE", "Kenya"),
    ("LU", "Luxembourg"),
    ("MX", "Mexico"),
    ("NL", "Netherlands"),
    ("NZ", "New Zealand"),
    ("NI", "Nicaragua"),
    ("NG", "Nigeria"),
    ("NO", "Norway"),
    ("PA", "Panama"),
    ("PY", "Paraguay"),
    ("PE", "Peru"),
    ("PL", "Poland"),
    ("PT", "Portugal"),
    ("RO", "Romania"),
    ("RS", "Serbia"),
    ("ZA", "South Africa"),
    ("KR", "South Korea"),
    ("ES", "Spain"),
    ("SE", "Sweden"),
    ("CH", "Switzerland"),
    ("TZ", "Tanzania"),
    ("TR", "Turkey"),
    ("UG", "Uganda"),
    ("UA", "Ukraine"),
    ("AE", "United Arab Emirates"),
    ("GB", "United Kingdom"),
    ("US", "United States"),
    ("UY", "Uruguay"),
    ("ZW", "Zimbabwe"),
];

static CHARTS_CACHE: Mutex<Option<HashMap<String, (std::time::Instant, Charts)>>> = Mutex::new(None);

fn runs_text(v: &Value) -> String {
    v.get("runs").and_then(Value::as_array).map(|r| r.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).collect::<String>()).unwrap_or_default()
}

fn best_thumb(v: &Value) -> Option<String> {
    fn find(v: &Value) -> Option<&Vec<Value>> {
        match v {
            Value::Object(m) => m.get("thumbnails").and_then(Value::as_array).or_else(|| m.values().find_map(find)),
            Value::Array(a) => a.iter().find_map(find),
            _ => None,
        }
    }
    find(v)?.iter().rev().find_map(|t| t.get("url").and_then(Value::as_str)).map(str::to_string)
}

fn browse_id(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => m.get("browseEndpoint").and_then(|b| b.get("browseId")).and_then(Value::as_str).map(str::to_string).or_else(|| m.values().find_map(browse_id)),
        Value::Array(a) => a.iter().find_map(browse_id),
        _ => None,
    }
}

fn collect_shelves<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            if let Some(s) = m.get("musicCarouselShelfRenderer") {
                out.push(s);
            }
            for x in m.values() {
                collect_shelves(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_shelves(x, out)),
        _ => {}
    }
}

pub fn parse_charts(country: &str, v: &Value) -> Charts {
    let mut shelves = Vec::new();
    collect_shelves(v, &mut shelves);
    let mut playlists = Vec::new();
    let mut artists = Vec::new();
    for sh in shelves {
        let group = sh.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title").map(runs_text).unwrap_or_default();
        for item in sh.get("contents").and_then(Value::as_array).into_iter().flatten() {
            if let Some(r) = item.get("musicTwoRowItemRenderer") {
                let Some(id) = browse_id(r.get("navigationEndpoint").unwrap_or(r)) else {
                    continue;
                };
                let Some(list) = id.strip_prefix("VL") else {
                    continue;
                };
                playlists.push(ChartPlaylist {
                    title: r.get("title").map(runs_text).unwrap_or_default(),
                    url: format!("https://music.youtube.com/playlist?list={list}"),
                    thumbnail: best_thumb(r.get("thumbnailRenderer").unwrap_or(r)),
                    group: group.clone(),
                });
            } else if let Some(r) = item.get("musicResponsiveListItemRenderer") {
                let name = r.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text").map(runs_text).unwrap_or_default();
                let Some(id) = browse_id(r.get("navigationEndpoint").unwrap_or(r)) else {
                    continue;
                };
                if name.is_empty() || !id.starts_with("UC") {
                    continue;
                }
                artists.push(ChartArtist { rank: artists.len() + 1, name, url: format!("https://www.youtube.com/channel/{id}"), thumbnail: best_thumb(r.get("thumbnail").unwrap_or(r)) });
            }
        }
    }
    Charts { country: country.to_string(), playlists, artists }
}

pub async fn charts(country: String) -> AppResult<Charts> {
    let country = if CHART_COUNTRIES.iter().any(|(c, _)| *c == country) { country } else { "ZZ".into() };
    if let Some((t, c)) = CHARTS_CACHE.lock().unwrap().as_ref().and_then(|m| m.get(&country).cloned()) {
        if t.elapsed().as_secs() < 3600 {
            return Ok(c);
        }
    }
    let cc = country.clone();
    let charts = blocking(move || {
        let body = serde_json::json!({
            "browseId": "FEmusic_charts",
            "formData": { "selectedValues": [cc] },
            "context": { "client": { "clientName": "WEB_REMIX", "clientVersion": "1.20250101.01.00", "hl": "en", "gl": "US" } }
        });
        let mut r = agent()
            .post("https://music.youtube.com/youtubei/v1/browse?prettyPrint=false")
            .header("Content-Type", "application/json")
            .header("Origin", "https://music.youtube.com")
            .header("User-Agent", USER_AGENT)
            .send(body.to_string())
            .map_err(|e| AppError::Network(e.to_string()))?;
        if !r.status().is_success() {
            return Err(AppError::Network(format!("YouTube Music charts returned HTTP {}", r.status().as_u16())));
        }
        let text = r.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
        let v: Value = serde_json::from_str(&text)?;
        Ok(parse_charts(&cc, &v))
    })
    .await?;
    if charts.playlists.is_empty() && charts.artists.is_empty() {
        return Err(AppError::Extraction("YouTube Music charts page format changed".into()));
    }
    CHARTS_CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(country, (std::time::Instant::now(), charts.clone()));
    Ok(charts)
}

// ---------------------------------------------------------------------------
// Lyrics translation (Google Translate's keyless web endpoint)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Translation {
    pub lines: Vec<String>,
    pub source_lang: Option<String>,
}

pub async fn translate(lines: Vec<String>, target: String) -> AppResult<Translation> {
    let target: String = target.chars().filter(|c| c.is_ascii_alphabetic() || *c == '-').take(8).collect();
    let target = if target.is_empty() { "en".into() } else { target };
    let text = lines.join("\n");
    if text.len() > 20_000 {
        return Err(AppError::Invalid("lyrics too long to translate".into()));
    }
    blocking(move || {
        let url = format!("https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl={target}&dt=t");
        let mut r = agent()
            .post(&url)
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/x-www-form-urlencoded;charset=utf-8")
            .send(format!("q={}", percent_encode(&text)))
            .map_err(|e| AppError::Network(e.to_string()))?;
        if !r.status().is_success() {
            return Err(AppError::Network(format!("translation failed (HTTP {})", r.status().as_u16())));
        }
        let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| AppError::Network(e.to_string()))?)?;
        let joined: String = v.get(0).and_then(Value::as_array).map(|segs| segs.iter().filter_map(|s| s.get(0).and_then(Value::as_str)).collect()).unwrap_or_default();
        Ok(Translation { lines: joined.split('\n').map(|l| l.trim().to_string()).collect(), source_lang: v.get(2).and_then(Value::as_str).map(str::to_string) })
    })
    .await
}

// ---------------------------------------------------------------------------
// Cover color (accent tint)
// ---------------------------------------------------------------------------

static COLOR_CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// Most characteristic vivid color of an image: hue histogram weighted by
/// saturation × brightness, averaged within the winning hue bucket.
pub fn dominant_color(bytes: &[u8]) -> Option<String> {
    let img = image::load_from_memory(bytes).ok()?.thumbnail(48, 48).to_rgb8();
    const BUCKETS: usize = 24;
    let mut weight = [0f64; BUCKETS];
    let mut sum = [[0f64; 3]; BUCKETS];
    for p in img.pixels() {
        let [r, g, b] = p.0.map(|c| c as f64 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        if max < 0.15 || delta < 0.08 {
            continue; // near black / grey
        }
        let sat = delta / max;
        let hue = if max == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        } / 6.0;
        let k = ((hue * BUCKETS as f64) as usize).min(BUCKETS - 1);
        let w = sat * sat * max;
        weight[k] += w;
        sum[k][0] += r * w;
        sum[k][1] += g * w;
        sum[k][2] += b * w;
    }
    let (k, w) = weight.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    if *w < 0.5 {
        return None; // greyscale cover
    }
    let c = sum[k].map(|x| ((x / w) * 255.0).round().clamp(0.0, 255.0) as u8);
    Some(format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
}

/// The ambient background: the cover shrunk to a soft, muted 24 px picture.
/// Scaled up by the page it looks like a heavy blur, but costs no live
/// blur filter (that was the GPU's biggest memory use).
pub fn wash(bytes: &[u8]) -> Option<String> {
    use base64::Engine;
    let img = image::load_from_memory(bytes).ok()?;
    let small = image::imageops::resize(&img.to_rgb8(), 24, 24, image::imageops::FilterType::Triangle);
    let mut soft = image::imageops::blur(&small, 2.2);
    for p in soft.pixels_mut() {
        // Desaturate a little: a hint of the cover, not a poster.
        let [r, g, b] = p.0.map(|c| c as f32);
        let l = 0.3 * r + 0.59 * g + 0.11 * b;
        p.0 = [r, g, b].map(|c| (l + (c - l) * 0.6).clamp(0.0, 255.0) as u8);
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(soft).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

static WASH_CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

pub async fn cover_wash(url: String) -> AppResult<Option<String>> {
    if !url.starts_with("https://") {
        return Ok(None);
    }
    if let Some(hit) = WASH_CACHE.lock().unwrap().as_ref().and_then(|m| m.get(&url).cloned()) {
        return Ok(hit);
    }
    let u = url.clone();
    let out = blocking(move || {
        let mut r = agent().get(&u).header("User-Agent", USER_AGENT).call().map_err(|e| AppError::Network(e.to_string()))?;
        if !r.status().is_success() {
            return Ok(None);
        }
        let bytes = r.body_mut().with_config().limit(8 * 1024 * 1024).read_to_vec().map_err(|e| AppError::Network(e.to_string()))?;
        Ok(wash(&bytes))
    })
    .await?;
    let mut cache = WASH_CACHE.lock().unwrap();
    let m = cache.get_or_insert_with(Default::default);
    if m.len() > 60 {
        m.clear();
    }
    m.insert(url, out.clone());
    Ok(out)
}

pub async fn cover_color(url: String) -> AppResult<Option<String>> {
    if !url.starts_with("https://") {
        return Ok(None);
    }
    if let Some(hit) = COLOR_CACHE.lock().unwrap().as_ref().and_then(|m| m.get(&url).cloned()) {
        return Ok(hit);
    }
    let u = url.clone();
    let color = blocking(move || {
        let mut r = agent().get(&u).header("User-Agent", USER_AGENT).call().map_err(|e| AppError::Network(e.to_string()))?;
        if !r.status().is_success() {
            return Ok(None);
        }
        let bytes = r.body_mut().with_config().limit(8 * 1024 * 1024).read_to_vec().map_err(|e| AppError::Network(e.to_string()))?;
        Ok(dominant_color(&bytes))
    })
    .await?;
    let mut cache = COLOR_CACHE.lock().unwrap();
    let m = cache.get_or_insert_with(Default::default);
    if m.len() > 500 {
        m.clear();
    }
    m.insert(url, color.clone());
    Ok(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominant_color_picks_the_vivid_part() {
        // Mostly dark grey with a red block: red should win.
        let mut img = image::RgbImage::from_pixel(40, 40, image::Rgb([30, 30, 30]));
        for x in 0..15 {
            for y in 0..15 {
                img.put_pixel(x, y, image::Rgb([220, 30, 40]));
            }
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let c = dominant_color(&png).unwrap();
        let r = u8::from_str_radix(&c[1..3], 16).unwrap();
        let g = u8::from_str_radix(&c[3..5], 16).unwrap();
        assert!(r > 180 && g < 80, "{c}");
        // Pure grey: no accent.
        let grey = image::RgbImage::from_pixel(20, 20, image::Rgb([128, 128, 128]));
        let mut png = Vec::new();
        grey.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!(dominant_color(&png), None);
    }

    #[test]
    #[ignore]
    fn live_explore() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        println!("{:?}", rt.block_on(suggestions("daft pu".into())).unwrap());
        println!("{:?}", rt.block_on(suggestions("עומר אד".into())).unwrap());
        for cc in ["ZZ", "IL", "US"] {
            let c = rt.block_on(charts(cc.into())).unwrap();
            println!("{cc}: {} playlists {:?}; {} artists, top {:?}", c.playlists.len(), c.playlists.iter().map(|p| &p.title).collect::<Vec<_>>(), c.artists.len(), c.artists.first().map(|a| &a.name));
            assert!(!c.playlists.is_empty());
        }
        let t = rt.block_on(translate(vec!["Je ne regrette rien".into(), "Non, rien de rien".into()], "en".into())).unwrap();
        println!("{t:?}");
        assert_eq!(t.lines.len(), 2);
        let col = rt.block_on(cover_color("https://i.ytimg.com/vi/NMRhx71bGo4/hqdefault.jpg".into())).unwrap();
        println!("cover color {col:?}");
    }
}
