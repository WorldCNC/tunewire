//! Offline integration tests: a fake `yt-dlp` script and a local HTTP server
//! (with Range support) stand in for the real sites, so search, grouping,
//! error mapping, streaming, seeking and local playback run without network.
//! Run with `cargo test`.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::decoder::AudioDecoder;
use crate::error::AppError;
use crate::models::Platform;
use crate::search::SearchCache;
use crate::{search, stream, ytdlp};

const RATE: u32 = 44_100;
const SECS: u32 = 6;

/// 16-bit stereo WAV: 440 Hz left, 660 Hz right.
fn tone_wav() -> Vec<u8> {
    let frames = RATE * SECS;
    let data_len = frames * 4;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&RATE.to_le_bytes());
    v.extend_from_slice(&(RATE * 4).to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let t = i as f32 / RATE as f32;
        for f in [440.0f32, 660.0] {
            let s = ((t * f * std::f32::consts::TAU).sin() * 12_000.0) as i16;
            v.extend_from_slice(&s.to_le_bytes());
        }
    }
    v
}

/// Minimal HTTP/1.1 file server with Range support, throttled so downloads
/// are still in progress when the decoder seeks ahead.
fn serve(body: Arc<Vec<u8>>) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let body = body.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut range = None;
                let mut line = String::new();
                while r.read_line(&mut line).unwrap_or(0) > 0 {
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("range: bytes=") {
                        let mut it = v.trim().split('-');
                        let a: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                        let b: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(body.len() - 1);
                        range = Some((a, b.min(body.len() - 1)));
                    }
                    line.clear();
                }
                let mut w = conn;
                let (status, a, b) = match range {
                    Some((a, b)) if a < body.len() => ("206 Partial Content", a, b),
                    Some(_) => {
                        let _ = w.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n");
                        return;
                    }
                    None => ("200 OK", 0, body.len() - 1),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                    b - a + 1,
                    body.len()
                );
                let _ = w.write_all(head.as_bytes());
                for chunk in body[a..=b].chunks(64 * 1024) {
                    if w.write_all(chunk).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
            });
        }
    });
    port
}

/// Fake yt-dlp: answers by looking at its arguments.
fn fake_ytdlp(dir: &std::path::Path, port: u16) -> PathBuf {
    let entry = |id: &str, title: &str, chan: &str, dur: f64, url: &str| {
        serde_json::json!({ "_type": "url", "id": id, "title": title, "channel": chan, "uploader": chan, "duration": dur, "url": url, "webpage_url": url })
    };
    let sc = serde_json::json!({ "title": "q", "entries": [
        { "_type": "url", "id": "1", "title": "One More Time", "uploader": "Daft Punk", "duration": 321.5, "webpage_url": "https://soundcloud.com/daftpunk/one-more-time", "url": "https://api.soundcloud.com/tracks/1" },
        { "_type": "url", "id": "2", "title": "Daft Punk - One More Time (Valmer Edit) [HZRX]", "uploader": "HOUZ", "duration": 313.0, "webpage_url": "https://soundcloud.com/houz/omt-edit", "url": "https://api.soundcloud.com/tracks/2" },
        { "_type": "url", "id": "3", "title": "One More Time", "uploader": "Daft Punk", "duration": 30.0, "webpage_url": "https://soundcloud.com/daftpunk/omt-preview", "url": "https://api.soundcloud.com/tracks/3" }
    ]});
    let yt = serde_json::json!({ "title": "q", "entries": [
        entry("FGBhQbmPwH8", "Daft Punk - One More Time (Official Video)", "Daft Punk", 322.0, "https://www.youtube.com/watch?v=FGBhQbmPwH8"),
        entry("A2VpR8HahKc", "Daft Punk - One More Time (Official Audio)", "Daft Punk", 321.0, "https://www.youtube.com/watch?v=A2VpR8HahKc"),
        entry("fa5IWHDbftI", "One More Time", "Daft Punk", 320.0, "https://www.youtube.com/watch?v=fa5IWHDbftI")
    ]});
    let ytm = serde_json::json!({ "title": "q - songs", "entries": [
        { "_type": "url", "id": "fa5IWHDbftI", "title": "One More Time", "url": "https://music.youtube.com/watch?v=fa5IWHDbftI" },
        { "_type": "url", "id": "wU26xVT_vBU", "title": "One More Time (Radio Edit)", "url": "https://music.youtube.com/watch?v=wU26xVT_vBU" }
    ]});
    let stream = serde_json::json!({ "title": "Tone", "artist": "Test", "duration": SECS, "ext": "wav", "acodec": "pcm_s16le", "protocol": "http",
        "url": format!("http://127.0.0.1:{port}/tone.wav"), "http_headers": { "User-Agent": "test" } });
    let write = |name: &str, v: &serde_json::Value| std::fs::write(dir.join(name), v.to_string()).unwrap();
    write("sc.json", &sc);
    write("yt.json", &yt);
    write("ytm.json", &ytm);
    write("stream.json", &stream);
    let script = format!(
        r#"#!/bin/sh
D="{d}"
case "$*" in
  *--version*) echo 2026.01.01 ;;
  *broken*) echo "ERROR: [youtube] broken: Video unavailable" >&2; exit 1 ;;
  *geo*) echo "ERROR: The uploader has not made this video available in your country" >&2; exit 1 ;;
  *scsearch*) cat "$D/sc.json" ;;
  *ytsearch*) cat "$D/yt.json" ;;
  *music.youtube.com/search*) cat "$D/ytm.json" ;;
  *" -f "*) cat "$D/stream.json" ;;
  *) echo "ERROR: Unsupported URL: $*" >&2; exit 1 ;;
esac
"#,
        d = dir.display()
    );
    let path = dir.join("yt-dlp");
    std::fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn audible(dec: &mut AudioDecoder, secs: f32) -> (usize, usize) {
    let n = (secs * dec.sample_rate() as f32) as usize * dec.channels() as usize;
    let v: Vec<f32> = dec.by_ref().take(n).collect();
    (v.len(), v.iter().filter(|s| s.abs() > 0.05).count())
}

/// One test so the global yt-dlp path isn't raced by parallel tests.
#[test]
fn end_to_end_offline() {
    let dir = std::env::temp_dir().join(format!("tunewire-itest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    stream::set_cache_dir(dir.join("cache"));
    let wav = Arc::new(tone_wav());
    let port = serve(wav.clone());
    ytdlp::set_binary(&fake_ytdlp(&dir, port).to_string_lossy());
    let rt = tokio::runtime::Runtime::new().unwrap();

    // 1. Health check sees the (fake) binary.
    let h = rt.block_on(ytdlp::health_check());
    assert!(h.ok, "{}", h.message);
    assert_eq!(h.version.as_deref(), Some("2026.01.01"));

    // 2. Search across sources: duplicates collapse, versions group, previews lose.
    let cache = Arc::new(SearchCache::default());
    let pref = vec![Platform::Ytmusic, Platform::Soundcloud, Platform::Youtube];
    let res = rt.block_on(search::search("daft punk one more time".into(), "all".into(), 10, pref.clone(), cache.clone(), true, false)).unwrap();
    let top = &res.tracks[0];
    assert_eq!(top.title, "One More Time");
    let plats: Vec<Platform> = top.versions.iter().map(|v| v.platform).collect();
    assert_eq!(plats, vec![Platform::Ytmusic, Platform::Soundcloud, Platform::Youtube], "{:#?}", res.tracks);
    assert!(top.versions.iter().all(|v| !v.preview), "preview should lose to the full upload");
    assert!(res.tracks.iter().any(|t| t.title.contains("Valmer Edit") && t.artist == "Daft Punk"), "edit stays separate, artist split from title");
    assert!(res.tracks.iter().filter(|t| t.title == "One More Time").count() == 1, "same song listed once");
    // YouTube Music entries borrowed artist + duration from YouTube.
    let ytm = top.versions.iter().find(|v| v.platform == Platform::Ytmusic).unwrap();
    assert_eq!(ytm.artist, "Daft Punk");
    assert_eq!(ytm.duration, Some(320.0));

    // 3. Second search is served from cache.
    let again = rt.block_on(search::search("daft punk one more time".into(), "all".into(), 10, pref.clone(), cache.clone(), true, false)).unwrap();
    assert!(again.cached);

    // 4. YouTube Music only: hidden YouTube search still enriches, but isn't shown.
    let only = rt.block_on(search::search("daft punk one more time".into(), "ytmusic".into(), 10, pref.clone(), Arc::new(SearchCache::default()), true, false)).unwrap();
    assert!(only.tracks.iter().all(|t| t.versions.iter().all(|v| v.platform == Platform::Ytmusic)));
    assert_eq!(only.tracks[0].artist, "Daft Punk");

    // 5. Error mapping.
    assert!(matches!(rt.block_on(ytdlp::extract_stream("https://www.youtube.com/watch?v=broken")), Err(AppError::Unavailable(_))));
    assert!(matches!(rt.block_on(ytdlp::extract_stream("https://www.youtube.com/watch?v=geo")), Err(AppError::RegionLocked)));
    assert!(matches!(rt.block_on(ytdlp::flat("https://example.com/nothing", None)), Err(AppError::Unsupported(_))));

    // 6. Stream through the fake extractor → local server → decoder, with a
    //    seek past what has downloaded (forces a Range jump).
    let info = rt.block_on(ytdlp::extract_stream("https://www.youtube.com/watch?v=tone")).unwrap();
    assert_eq!(info.ext.as_deref(), Some("wav"));
    let opened = stream::open(&info).unwrap();
    let mut dec = AudioDecoder::new(opened.reader, opened.hint.as_deref()).unwrap();
    assert_eq!((dec.channels(), dec.sample_rate()), (2, RATE));
    let (n, loud) = audible(&mut dec, 0.5);
    assert!(n > 0 && loud > n / 3, "decoded audio should be a clear tone");
    dec.try_seek(Duration::from_secs(5)).unwrap();
    let (n, loud) = audible(&mut dec, 0.5);
    assert!(n > 0 && loud > n / 3, "audio after seeking ahead");
    // Reading to the end terminates.
    let rest = dec.by_ref().count();
    assert!(rest < (RATE as usize * 2), "only ~0.5 s should remain, got {rest} samples");

    // 7. Local file playback (same reader, no network).
    let local = dir.join("tone.wav");
    std::fs::write(&local, &*wav).unwrap();
    let opened = stream::open_local(&local).unwrap();
    let mut dec = AudioDecoder::new(opened.reader, opened.hint.as_deref()).unwrap();
    assert!(dec.total_duration().map(|d| (d.as_secs_f64() - SECS as f64).abs() < 0.1).unwrap_or(false));
    dec.try_seek(Duration::from_secs(3)).unwrap();
    assert!(audible(&mut dec, 0.2).1 > 0);

    // 8. Missing yt-dlp is reported clearly.
    ytdlp::set_binary(&dir.join("does-not-exist").to_string_lossy());
    assert!(matches!(rt.block_on(ytdlp::flat("ytsearch1:x", None)), Err(AppError::YtDlpMissing)));
    assert!(!rt.block_on(ytdlp::health_check()).ok);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reader_serves_partial_file_while_downloading() {
    // Read through a slow server byte-for-byte and compare with the source.
    let body = Arc::new((0..2_000_000u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>());
    let port = serve(body.clone());
    let opened = stream::open_progressive(&format!("http://127.0.0.1:{port}/blob"), &Default::default(), None, None).unwrap();
    let mut r = opened.reader;
    use std::io::{Seek, SeekFrom};
    // Jump far ahead first (random access), then read everything from the start.
    r.seek(SeekFrom::Start(1_900_000)).unwrap();
    let mut tail = vec![0u8; 50_000];
    r.read_exact(&mut tail).unwrap();
    assert_eq!(&tail[..], &body[1_900_000..1_950_000]);
    r.seek(SeekFrom::Start(0)).unwrap();
    let mut all = Vec::new();
    r.read_to_end(&mut all).unwrap();
    assert_eq!(all.len(), body.len());
    assert!(all == *body);
}

/// A seekable source behind a header (like Spotify's decrypted Ogg files),
/// served through the disk buffer: byte-exact, seekable ahead, decodable.
#[test]
fn source_adapter_skips_header_and_seeks() {
    /// Slow reader so the decoder seeks ahead of the copy thread.
    struct Slow(std::io::Cursor<Vec<u8>>);
    impl Read for Slow {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(Duration::from_millis(2));
            let n = b.len().min(16 * 1024);
            self.0.read(&mut b[..n])
        }
    }
    impl std::io::Seek for Slow {
        fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
            self.0.seek(p)
        }
    }
    let wav = tone_wav();
    let mut data = vec![0xAAu8; 167];
    data.extend_from_slice(&wav);
    let opened = stream::open_source(Slow(std::io::Cursor::new(data)), 167, wav.len() as u64, Some("wav".into())).unwrap();
    let mut dec = AudioDecoder::new(opened.reader, opened.hint.as_deref()).unwrap();
    assert_eq!((dec.channels(), dec.sample_rate()), (2, RATE));
    assert!(audible(&mut dec, 0.3).1 > 0);
    dec.try_seek(Duration::from_secs(5)).unwrap();
    let (n, loud) = audible(&mut dec, 0.5);
    assert!(n > 0 && loud > n / 3, "audio after seeking ahead");
    let path = opened.shared.wait_complete(Duration::from_secs(20)).unwrap();
    assert!(std::fs::read(path).unwrap() == wav, "buffer must equal the source minus its header");
}

/// Real-time engine test on the default output device. Run with a null ALSA
/// device: `ALSA_CONFIG_PATH=<file with pcm.!default {type null}> cargo test -- --ignored engine`
#[test]
#[ignore]
fn engine_gapless_and_crossfade() {
    use crate::audio::{prepare, AudioHandle, EngineEvent};
    use std::sync::mpsc;
    let dir = std::env::temp_dir().join(format!("tunewire-engine-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("tone.wav");
    std::fs::write(&wav, tone_wav()).unwrap();
    let load = |token| prepare(stream::open_local(&wav).unwrap(), None, token).unwrap();

    let (tx, rx) = mpsc::channel::<String>();
    let audio = AudioHandle::spawn(0.5, move |ev| {
        let s = match ev {
            EngineEvent::Tick => return,
            EngineEvent::Advanced { token } => format!("advanced {token}"),
            EngineEvent::Ended { token } => format!("ended {token}"),
            EngineEvent::Error { token, error } => format!("error {token} {error}"),
        };
        let _ = tx.send(s);
    });
    let t0 = std::time::Instant::now();
    // Gapless: track 1 → track 2 switch with no overlap.
    audio.configure(0.0, true);
    audio.play(load(1));
    std::thread::sleep(Duration::from_millis(300));
    audio.preload(load(2));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(audio.status().preloaded, Some(2));
    let ev = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let switched = t0.elapsed().as_secs_f32();
    assert_eq!(ev, "advanced 2");
    assert!((5.5..7.0).contains(&switched), "gapless switch at {switched}s");
    assert_eq!(audio.status().token, 2);

    // Seek within track 2, then crossfade into track 3.
    audio.seek(1.0);
    std::thread::sleep(Duration::from_millis(300));
    let pos = audio.status().position;
    assert!((1.0..2.0).contains(&pos), "position after seek {pos}");
    audio.configure(2.0, true);
    audio.preload(load(3));
    let t1 = std::time::Instant::now();
    let ev = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(ev, "advanced 3");
    let at = t1.elapsed().as_secs_f32();
    // Track 2 had ~4.7 s left; crossfade starts 2 s before its end.
    assert!((2.0..3.6).contains(&at), "crossfade started after {at}s");

    // No preload → plain end of track.
    let ev = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(ev, "ended 3");
    let _ = std::fs::remove_dir_all(&dir);
}
