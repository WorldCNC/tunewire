//! Live benchmarks of everything that talks to a platform (run by hand):
//!   cargo test --lib bench -- --ignored --nocapture --test-threads=1

#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::models::Platform;
use crate::search::SearchCache;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

fn ms(d: Duration) -> String {
    format!("{:>6} ms", d.as_millis())
}

/// Stream extraction through the yt-dlp worker with different options.
#[test]
#[ignore]
fn bench_extract() {
    let rt = rt();
    let _g = rt.enter();
    let bin = crate::ytdlp::binary();
    rt.block_on(crate::ytdlp_worker::warm(bin.clone()));
    std::thread::sleep(Duration::from_secs(2));
    let format = crate::ytdlp::AUDIO_FORMAT;
    let yt = ["4NRXx6U8ABQ", "JGwWNGJdvx8", "kJQP7kiw5Fk", "fJ9rUzIMcZQ", "hTWKbfoikeg", "YQHsXMglC9A", "09R8_2nJtjg", "RgKAFK5djSk", "lp-EO5I60KA", "60ItHLz5WEA", "OPf0YbXqDm0", "pRpeEdMmmQ0"];
    let variants: Vec<(&str, serde_json::Value)> = vec![
        ("default (skip hls,dash)", serde_json::json!({ "youtube": { "skip": ["hls", "dash"] } })),
        ("default (2)", serde_json::json!({ "youtube": { "skip": ["hls", "dash"] } })),
    ];
    let mut i = 0;
    for (name, args) in variants {
        let mut times = Vec::new();
        let mut ok = 0;
        for _ in 0..2 {
            let url = format!("https://music.youtube.com/watch?v={}", yt[i % yt.len()]);
            i += 1;
            let t0 = Instant::now();
            let r = rt.block_on(crate::ytdlp_worker::extract(&bin, &url, serde_json::json!({ "format": format, "noplaylist": true, "extractor_args": args })));
            times.push(t0.elapsed());
            if let Ok(Some(v)) = &r {
                if v.get("url").is_some() {
                    ok += 1;
                }
            } else {
                println!("   {name}: {:?}", r.as_ref().err());
            }
        }
        println!("YT extract  {name:<34} {} {}  ({ok}/2 ok)", ms(times[0]), ms(times[1]));
    }
    for url in ["https://soundcloud.com/daft-punk-id/daft-punk-one-more-time", "https://soundcloud.com/odesza/say-my-name-feat-zyra"] {
        let t0 = Instant::now();
        let r = rt.block_on(crate::ytdlp_worker::extract(&bin, url, serde_json::json!({ "format": format, "noplaylist": true })));
        println!("SC extract  {:<34} {}  ok={}", url.rsplit('/').next().unwrap(), ms(t0.elapsed()), matches!(r, Ok(Some(_))));
    }
}

/// Song search per platform (fresh cache, different queries).
#[test]
#[ignore]
fn bench_search() {
    let rt = rt();
    let _g = rt.enter();
    rt.block_on(crate::ytdlp_worker::warm(crate::ytdlp::binary()));
    std::thread::sleep(Duration::from_secs(2));
    let queries = ["radiohead nude", "frank ocean nights", "fleetwood mac dreams", "phoebe bridgers motion sickness", "kaytranada glowed up"];
    for (src, plats) in [("ytmusic", vec![Platform::Ytmusic]), ("youtube", vec![Platform::Youtube]), ("soundcloud", vec![Platform::Soundcloud]), ("spotify", vec![Platform::Spotify]), ("all", Platform::ALL.to_vec())] {
        let mut row = Vec::new();
        for q in queries.iter().take(3) {
            let cache = Arc::new(SearchCache::default());
            let t0 = Instant::now();
            let r = rt.block_on(crate::search::search(format!("{q} {src}"), src.into(), 12, plats.clone(), cache, true, true));
            row.push(format!("{} ({})", ms(t0.elapsed()), r.map(|r| r.tracks.len()).unwrap_or(0)));
        }
        println!("search {src:<10} {}", row.join(" | "));
    }
    let t0 = Instant::now();
    let n = rt.block_on(crate::ytm::search_songs("frank ocean pink white")).map(|v| v.len()).unwrap_or(0);
    println!("ytm api songs        {} ({n})", ms(t0.elapsed()));
}

/// Opening a resolved stream: connect, probe, first audio.
#[test]
#[ignore]
fn bench_open() {
    let rt = rt();
    let _g = rt.enter();
    rt.block_on(crate::ytdlp_worker::warm(crate::ytdlp::binary()));
    for url in ["https://music.youtube.com/watch?v=hTWKbfoikeg", "https://www.youtube.com/watch?v=YQHsXMglC9A", "https://soundcloud.com/odesza/say-my-name-feat-zyra"] {
        let t0 = Instant::now();
        let info = rt.block_on(crate::ytdlp::extract_stream(url)).expect("extract");
        let t1 = t0.elapsed();
        let opened = crate::stream::open(&info).expect("open");
        let t2 = t0.elapsed();
        let mut t = crate::audio::prepare(opened, info.duration, 1).expect("decoder");
        let t3 = t0.elapsed();
        let _ = t.decoder.next();
        let t4 = t0.elapsed();
        println!("{:<55} extract {} | open {} | probe {} | first {} | total {} [{:?} {:?}]", url, ms(t1), ms(t2 - t1), ms(t3 - t2), ms(t4 - t3), ms(t4), info.ext, info.protocol);
    }
}

/// Playlists, artist/album pages, lyrics, radio.
#[test]
#[ignore]
fn bench_pages() {
    let rt = rt();
    let _g = rt.enter();
    rt.block_on(crate::ytdlp_worker::warm(crate::ytdlp::binary()));
    std::thread::sleep(Duration::from_secs(2));
    let time = |name: &str, f: &dyn Fn() -> String| {
        let t0 = Instant::now();
        let r = f();
        println!("{name:<34} {}  {r}", ms(t0.elapsed()));
    };
    time("YT playlist (yt-dlp flat)", &|| rt.block_on(crate::ytdlp::flat("https://www.youtube.com/playlist?list=PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI", Some(1000))).map(|v| format!("{} entries", v.get("entries").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0))).unwrap_or_else(|e| e.to_string()));
    time("YTM playlist (api)", &|| rt.block_on(crate::ytm::collection("VLPLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI")).map(|c| format!("{} tracks", c.tracks.len())).unwrap_or_else(|e| e.to_string()));
    time("SC set (yt-dlp flat)", &|| rt.block_on(crate::ytdlp::flat("https://soundcloud.com/odesza/sets/a-moment-apart-1", Some(1000))).map(|v| format!("{} entries", v.get("entries").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0))).unwrap_or_else(|e| e.to_string()));
    time("Spotify playlist (pathfinder)", &|| crate::spotify::fetch_playlist_full("37i9dQZF1DX4o1oenSJRJd").map(|(_, t)| format!("{} tracks", t.len())).unwrap_or_else(|e| e.to_string()));
    time("lyrics (LRCLIB)", &|| rt.block_on(crate::lyrics::get("Daft Punk".into(), "Digital Love".into(), None, Some(301.0))).map(|l| format!("found={}", l.is_some())).unwrap_or_else(|e| e.to_string()));
    let cache = Arc::new(SearchCache::default());
    time("artist page (api)", &|| rt.block_on(async { let (b, _) = crate::ytm::find_artist("Radiohead").await?; crate::ytm::artist(&b).await }).map(|a| format!("{} top", a.top_songs.len())).unwrap_or_else(|e| e.to_string()));
    time("search extras (api)", &|| rt.block_on(crate::ytm::search_extras("radiohead")).map(|x| format!("{} albums", x.albums.len())).unwrap_or_else(|e| e.to_string()));
    let seed = rt.block_on(crate::search::search("tame impala let it happen".into(), "ytmusic".into(), 3, vec![Platform::Ytmusic], cache.clone(), false, false)).unwrap().tracks.remove(0);
    time("radio (related)", &|| rt.block_on(crate::radio::related(&seed, &Default::default(), 30, &cache)).map(|v| format!("{} songs", v.len())).unwrap_or_else(|e| e.to_string()));
}

/// A Spotify song that hasn't been matched yet: find its recording, then its stream.
#[test]
#[ignore]
fn bench_spotify_play() {
    let rt = rt();
    let _g = rt.enter();
    rt.block_on(crate::ytdlp_worker::warm(crate::ytdlp::binary()));
    rt.block_on(crate::ytdlp::warm_youtube());
    crate::spotify::warm();
    for q in ["arctic monkeys do i wanna know", "sza kill bill", "tyler the creator see you again"] {
        let sp = rt.block_on(crate::spotify::search(q, 1)).unwrap().remove(0);
        let t0 = Instant::now();
        let m = rt.block_on(crate::matcher::match_spotify(&sp)).expect("match");
        let t1 = t0.elapsed();
        let info = rt.block_on(crate::ytdlp::stream_info(&m.url)).expect("stream");
        let t2 = t0.elapsed();
        let opened = crate::stream::open(&info).expect("open");
        let mut d = crate::audio::prepare(opened, info.duration, 1).expect("decoder");
        let _ = d.decoder.next();
        println!("{:<40} match {} (isrc {}) | stream {} | playing at {}", sp.title, ms(t1), m.isrc_match, ms(t2 - t1), ms(t0.elapsed()));
    }
}

#[test]
#[ignore]
fn bench_spotify_cold_parts() {
    let rt = rt();
    let _g = rt.enter();
    let t = Instant::now();
    crate::spotify::warm();
    println!("token            {}", ms(t.elapsed()));
    let t = Instant::now();
    let isrc = rt.block_on(crate::spotify::isrc("4uLU6hMCjMI75M1A2tKUQC"));
    println!("isrc (1st)       {} {:?}", ms(t.elapsed()), isrc.as_ref().ok());
    let t = Instant::now();
    let _ = rt.block_on(crate::spotify::isrc("7MXVkk9YMctZqd1Srtv4MB"));
    println!("isrc (2nd)       {}", ms(t.elapsed()));
    let t = Instant::now();
    let _ = rt.block_on(crate::ytm::search_songs("GBARL9300135"));
    println!("ytm search (1st) {}", ms(t.elapsed()));
    let t = Instant::now();
    let _ = rt.block_on(crate::ytm::search_songs("USUM71703861"));
    println!("ytm search (2nd) {}", ms(t.elapsed()));
}

/// Dump ~1000 real songs (real cover URLs) for the memory test page.
#[test]
#[ignore]
fn dump_big_library() {
    let rt = rt();
    let _g = rt.enter();
    let mut all = Vec::new();
    for q in ["top hits 2024", "rock classics", "hip hop hits", "chill indie", "edm hits", "rnb classics", "pop 2010s", "lofi beats"] {
        if let Ok(x) = rt.block_on(crate::ytm::search_extras(q)) {
            for p in x.playlists.iter().take(2) {
                if let Some(b) = &p.browse_id {
                    if let Ok(c) = rt.block_on(crate::ytm::collection(b)) {
                        all.extend(c.tracks);
                    }
                }
            }
        }
        if all.len() > 1100 { break; }
    }
    all.truncate(1000);
    let path = std::env::var("BIG_OUT").unwrap();
    std::fs::write(&path, format!("window.__BIG = {};", serde_json::to_string(&all).unwrap())).unwrap();
    println!("wrote {} tracks, {} distinct covers", all.len(), all.iter().filter_map(|t| t.thumbnail.clone()).collect::<std::collections::HashSet<_>>().len());
}
