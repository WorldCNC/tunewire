//! Offline performance benchmark (no network), comparable across versions:
//!   cargo test --release --lib perfbench -- --ignored --nocapture

#![cfg(test)]

use std::time::Instant;

use crate::models::{Platform, Track, TrackVersion};
use crate::store::{HistoryEntry, Playlist, Store};

fn version(i: usize, platform: Platform) -> TrackVersion {
    let artists = ["Velvet Static", "Mira Vale", "The Low Tides", "Kai Rowan", "Juno Park"];
    let mut v: TrackVersion = serde_json::from_value(serde_json::json!({
        "platform": platform, "url": format!("https://example.com/{i}/{platform:?}"), "id": format!("{i}"),
        "title": format!("Song {} (Official Video)", i % 60), "artist": artists[i % artists.len()],
        "duration": 180.0 + (i % 7) as f64, "thumbnail": null,
    }))
    .unwrap();
    v.rank = i;
    v
}

fn track(i: usize) -> Track {
    Track::from_version(version(i, Platform::Ytmusic))
}

fn time<T>(label: &str, n: usize, mut f: impl FnMut() -> T) -> f64 {
    let t0 = Instant::now();
    for _ in 0..n {
        std::hint::black_box(f());
    }
    let per = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("{label:<52} {per:>9.3} ms");
    per
}

#[test]
#[ignore]
fn perfbench() {
    println!();
    // Search: grouping 160 raw results from 4 platforms into songs.
    let raw: Vec<TrackVersion> = (0..160).map(|i| version(i, [Platform::Ytmusic, Platform::Soundcloud, Platform::Youtube, Platform::Spotify][i % 4])).collect();
    let pref = Platform::ALL.to_vec();
    time("search: group 160 results", 200, || crate::search::group_results("song", raw.clone(), &pref));

    // Listening stats over a 30 000-play history.
    let dir = std::env::temp_dir().join(format!("tw-perf-{}", crate::store::now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    let now = crate::store::now_ms();
    let history: Vec<HistoryEntry> = (0..30_000).map(|i| HistoryEntry { track: track(i % 900), platform: Some(Platform::Ytmusic), played_at: now - (i as u64) * 600_000 }).collect();
    crate::stats::init(&dir, &history);
    time("stats: load the play log (first use)", 1, || crate::stats::plays().len());
    time("stats: get the play log (Home, For you)", 200, || crate::stats::plays().len());
    time("stats: summarize all time", 20, || crate::stats::summarize("all", None).plays);
    time("stats: summarize 4 weeks", 20, || crate::stats::summarize("4w", None).plays);

    // Saving a library with 3 000 songs in playlists.
    let store = Store::open(dir.join("lib")).unwrap();
    store
        .update_library(|lib| {
            for p in 0..10 {
                lib.playlists.push(Playlist { id: format!("p{p}"), name: format!("List {p}"), tracks: (0..300).map(|i| track(p * 300 + i)).collect(), source_url: None, created: 0, updated: 0, description: String::new(), cover: None, pinned: false, folder: None, added: Default::default(), cloud_id: None, cloud_role: None });
            }
            Ok(())
        })
        .unwrap();
    time("library: save (3 000 songs)", 30, || store.update_library(|_| Ok(())).unwrap());

    // Concurrent saves: does the file on disk end up with every change?
    let store = std::sync::Arc::new(Store::open(dir.join("race")).unwrap());
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let s = store.clone();
            std::thread::spawn(move || {
                for i in 0..40 {
                    let _ = s.update_library(|lib| {
                        lib.favorites.push(track(t * 1000 + i));
                        Ok(())
                    });
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let disk: crate::store::Library = crate::store::load_json(&dir.join("race/library.json"));
    println!("{:<52} {:>9}", "library: concurrent saves kept on disk (of 320)", disk.favorites.len());
    let _ = std::fs::remove_dir_all(&dir);
}
