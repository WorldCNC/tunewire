//! Listening stats: a play log of how long each song was actually listened to
//! (plays.jsonl, one line per listen), and the summaries built from it.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

use crate::models::{Platform, Track};
use crate::radio::primary_artist;
use crate::state::PlaybackState;
use crate::store::now_ms;

/// Shorter listens are skips and aren't logged at all.
const MIN_LOGGED: f64 = 10.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Play {
    /// When the listen started (ms).
    pub t: u64,
    /// Seconds actually listened.
    pub secs: f64,
    /// Imported from the old history list, so `secs` is a guess.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub est: bool,
    pub platform: Option<Platform>,
    pub track: Track,
}

impl Play {
    /// Counts as a play: 30 s, or half the song for short ones.
    pub fn counts(&self) -> bool {
        self.secs >= 30.0 || self.track.duration.map(|d| self.secs >= d * 0.5).unwrap_or(false)
    }
}

struct Listen {
    uid: u64,
    track: Track,
    platform: Option<Platform>,
    started: u64,
    secs: f64,
    last: Instant,
}

static LISTEN: Mutex<Option<Listen>> = Mutex::new(None);
static PLAYS: Mutex<Option<Vec<Play>>> = Mutex::new(None);
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

fn log_path(dir: &Path) -> PathBuf {
    dir.join("plays.jsonl")
}

/// Load the log (importing the old history once, so stats aren't empty on day one).
pub fn init(dir: &Path, history: &[crate::store::HistoryEntry]) {
    *DIR.lock().unwrap() = Some(dir.to_path_buf());
    let path = log_path(dir);
    if !path.exists() && !history.is_empty() {
        let mut out = String::new();
        for h in history.iter().rev() {
            let secs = h.track.duration.unwrap_or(180.0).min(300.0);
            let p = Play { t: h.played_at, secs, est: true, platform: h.platform, track: slim(&h.track, h.platform) };
            if let Ok(line) = serde_json::to_string(&p) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        let _ = std::fs::write(&path, out);
    }
}

fn load() -> Vec<Play> {
    let Some(dir) = DIR.lock().unwrap().clone() else { return Vec::new() };
    std::fs::read_to_string(log_path(&dir))
        .map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

/// All logged plays, oldest first.
pub fn plays() -> Vec<Play> {
    let mut g = PLAYS.lock().unwrap();
    g.get_or_insert_with(load).clone()
}

/// Only the version that was played (keeps the log small).
fn slim(track: &Track, platform: Option<Platform>) -> Track {
    let mut t = track.clone();
    t.score = 0.0;
    if let Some(p) = platform {
        let keep: Vec<_> = t.versions.iter().filter(|v| v.platform == p || v.matched_for == Some(p)).cloned().collect();
        if !keep.is_empty() {
            t.versions = keep;
        }
    }
    t.versions.truncate(2);
    t
}

fn append(l: Listen) {
    if l.secs < MIN_LOGGED {
        return;
    }
    let play = Play { t: l.started, secs: (l.secs * 10.0).round() / 10.0, est: false, platform: l.platform, track: slim(&l.track, l.platform) };
    let Some(dir) = DIR.lock().unwrap().clone() else { return };
    if let Ok(line) = serde_json::to_string(&play) {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(log_path(&dir));
        if let Ok(mut f) = file {
            let _ = writeln!(f, "{line}");
        }
    }
    if let Some(v) = PLAYS.lock().unwrap().as_mut() {
        v.push(play);
    }
}

/// Called on every engine tick: counts listened time for the current song.
pub fn on_tick(ps: &PlaybackState) {
    let mut g = LISTEN.lock().unwrap();
    let cur = ps.current.as_ref();
    if g.as_ref().map(|l| l.uid) != cur.map(|c| c.uid) {
        if let Some(old) = g.take() {
            append(old);
        }
        if let Some(c) = cur {
            *g = Some(Listen { uid: c.uid, track: c.track.clone(), platform: ps.version.as_ref().map(|v| v.platform), started: now_ms(), secs: 0.0, last: Instant::now() });
        }
    }
    if let Some(l) = g.as_mut() {
        if ps.status == "playing" {
            // Capped, so a suspended laptop doesn't count as hours of listening.
            l.secs += l.last.elapsed().as_secs_f64().min(2.0);
        }
        l.last = Instant::now();
        if l.platform.is_none() {
            l.platform = ps.version.as_ref().map(|v| v.platform);
        }
    }
}

/// Write the current listen (app exit).
pub fn flush() {
    if let Some(l) = LISTEN.lock().unwrap().take() {
        append(l);
    }
}

// ---------------------------------------------------------------------------
// Summaries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TopTrack {
    pub track: Track,
    pub plays: usize,
    pub minutes: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopArtist {
    pub name: String,
    pub plays: usize,
    pub minutes: f64,
    pub thumbnail: Option<String>,
    /// Their most played song in the range.
    pub top_track: Option<Track>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub label: String,
    pub minutes: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Stats {
    pub range: String,
    pub from: u64,
    pub to: u64,
    pub minutes: f64,
    pub plays: usize,
    pub skips: usize,
    pub tracks: usize,
    pub artists: usize,
    /// Artists first heard in this range.
    pub new_artists: usize,
    pub days_active: usize,
    pub streak: usize,
    pub longest_streak: usize,
    pub top_tracks: Vec<TopTrack>,
    pub top_artists: Vec<TopArtist>,
    /// Minutes per local hour of day (0-23).
    pub hours: Vec<f64>,
    /// Minutes per weekday, Monday first.
    pub weekdays: Vec<f64>,
    pub series: Vec<Bucket>,
    pub platforms: Vec<Bucket>,
    pub busiest_day: Option<Bucket>,
    /// Part of the data is estimated (imported from the old history).
    pub estimated: bool,
    /// Years that have any plays (for the Wrapped picker).
    pub years: Vec<i32>,
    pub first_play: Option<u64>,
}

fn local(t: u64) -> DateTime<Local> {
    Local.timestamp_millis_opt(t as i64).single().unwrap_or_else(Local::now)
}

fn start_of_day(d: NaiveDate) -> u64 {
    d.and_hms_opt(0, 0, 0).and_then(|n| Local.from_local_datetime(&n).earliest()).map(|d| d.timestamp_millis() as u64).unwrap_or(0)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `range`: "4w", "6m", "year" (with `year`, default this one) or "all".
pub fn summarize(range: &str, year: Option<i32>) -> Stats {
    let all = plays();
    let now = Local::now();
    let today = now.date_naive();
    let (from, to) = match range {
        "4w" => (start_of_day(today - chrono::Duration::days(27)), u64::MAX),
        "6m" => (start_of_day(today - chrono::Duration::days(182)), u64::MAX),
        "year" => {
            let y = year.unwrap_or(now.year());
            (start_of_day(NaiveDate::from_ymd_opt(y, 1, 1).unwrap()), start_of_day(NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap()))
        }
        _ => (0, u64::MAX),
    };
    let mut s = Stats { range: range.to_string(), from, to: to.min(now_ms()), hours: vec![0.0; 24], weekdays: vec![0.0; 7], ..Default::default() };
    let mut years: Vec<i32> = all.iter().map(|p| local(p.t).year()).collect::<HashSet<_>>().into_iter().collect();
    years.sort_unstable_by(|a, b| b.cmp(a));
    s.years = years;
    s.first_play = all.first().map(|p| p.t);

    // Artists heard before the range starts (for "new artists").
    let before: HashSet<String> = all.iter().filter(|p| p.t < from && p.counts()).map(|p| primary_artist(&p.track.artist)).collect();

    struct Acc {
        track: Track,
        plays: usize,
        secs: f64,
    }
    let mut tracks: HashMap<String, Acc> = HashMap::new();
    let mut artists: HashMap<String, (String, usize, f64, Option<String>)> = HashMap::new();
    let mut days: HashMap<NaiveDate, f64> = HashMap::new();
    let mut platforms: HashMap<&'static str, f64> = HashMap::new();
    let mut series: Vec<(String, f64)> = Vec::new();
    let mut series_index: HashMap<String, usize> = HashMap::new();
    // Pre-fill the series so empty periods show as gaps, not missing bars.
    let series_key = |d: &DateTime<Local>| -> String {
        match range {
            "4w" => format!("{} {}", MONTHS[d.month0() as usize], d.day()),
            "6m" => {
                let monday = d.date_naive() - chrono::Duration::days(d.weekday().num_days_from_monday() as i64);
                format!("{} {}", MONTHS[monday.month0() as usize], monday.day())
            }
            "year" => MONTHS[d.month0() as usize].to_string(),
            _ => format!("{} {}", MONTHS[d.month0() as usize], d.year()),
        }
    };
    {
        let mut push = |k: String| {
            if !series_index.contains_key(&k) {
                series_index.insert(k.clone(), series.len());
                series.push((k, 0.0));
            }
        };
        match range {
            "4w" => (0..28).rev().for_each(|i| push(series_key(&(now - chrono::Duration::days(i))))),
            "6m" => (0..27).rev().for_each(|i| push(series_key(&(now - chrono::Duration::weeks(i))))),
            "year" => MONTHS.iter().for_each(|m| push(m.to_string())),
            _ => {
                if let Some(first) = all.first() {
                    let mut d = local(first.t).date_naive().with_day(1).unwrap();
                    while d <= today {
                        push(format!("{} {}", MONTHS[d.month0() as usize], d.year()));
                        d = if d.month() == 12 { NaiveDate::from_ymd_opt(d.year() + 1, 1, 1).unwrap() } else { NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1).unwrap() };
                    }
                }
            }
        }
    }

    for p in all.iter().filter(|p| p.t >= from && p.t < to) {
        let d = local(p.t);
        s.minutes += p.secs / 60.0;
        s.estimated |= p.est;
        if p.counts() {
            s.plays += 1;
        } else {
            s.skips += 1;
        }
        s.hours[d.hour() as usize] += p.secs / 60.0;
        s.weekdays[d.weekday().num_days_from_monday() as usize] += p.secs / 60.0;
        *days.entry(d.date_naive()).or_default() += p.secs / 60.0;
        if let Some(pl) = p.platform {
            *platforms.entry(pl.label()).or_default() += p.secs / 60.0;
        }
        let k = series_key(&d);
        let i = *series_index.entry(k.clone()).or_insert_with(|| {
            series.push((k, 0.0));
            series.len() - 1
        });
        series[i].1 += p.secs / 60.0;

        let a = tracks.entry(p.track.key.clone()).or_insert_with(|| Acc { track: p.track.clone(), plays: 0, secs: 0.0 });
        a.secs += p.secs;
        if p.counts() {
            a.plays += 1;
        }
        if a.track.thumbnail.is_none() {
            a.track.thumbnail = p.track.thumbnail.clone();
        }
        if !p.track.artist.is_empty() {
            let display = p.track.artist.split([',', '&']).next().unwrap_or(&p.track.artist).split(" feat").next().unwrap_or("").trim().to_string();
            let e = artists.entry(primary_artist(&p.track.artist)).or_insert_with(|| (display, 0, 0.0, None));
            e.2 += p.secs;
            if p.counts() {
                e.1 += 1;
            }
            if e.3.is_none() {
                e.3 = p.track.thumbnail.clone();
            }
        }
    }

    s.tracks = tracks.values().filter(|a| a.plays > 0).count();
    s.artists = artists.values().filter(|a| a.1 > 0).count();
    s.new_artists = artists.iter().filter(|(k, a)| a.1 > 0 && !before.contains(*k)).count();
    s.days_active = days.len();

    let mut top: Vec<Acc> = tracks.into_values().filter(|a| a.plays > 0).collect();
    top.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.secs.total_cmp(&a.secs)));
    let best_by_artist: HashMap<String, Track> = top.iter().rev().map(|a| (primary_artist(&a.track.artist), a.track.clone())).collect();
    s.top_tracks = top.into_iter().take(50).map(|a| TopTrack { track: a.track, plays: a.plays, minutes: a.secs / 60.0 }).collect();

    let mut arts: Vec<(String, (String, usize, f64, Option<String>))> = artists.into_iter().filter(|a| a.1 .1 > 0).collect();
    arts.sort_by(|a, b| b.1 .2.total_cmp(&a.1 .2));
    s.top_artists = arts
        .into_iter()
        .take(30)
        .map(|(k, (name, plays, secs, thumb))| {
            let top_track = best_by_artist.get(&k).cloned();
            TopArtist { name, plays, minutes: secs / 60.0, thumbnail: top_track.as_ref().and_then(|t| t.thumbnail.clone()).or(thumb), top_track }
        })
        .collect();

    // Streaks of consecutive listening days.
    let mut sorted: Vec<NaiveDate> = days.keys().copied().collect();
    sorted.sort_unstable();
    let mut run = 0;
    let mut prev: Option<NaiveDate> = None;
    for d in &sorted {
        run = if prev.map(|p| *d - p == chrono::Duration::days(1)).unwrap_or(false) { run + 1 } else { 1 };
        s.longest_streak = s.longest_streak.max(run);
        prev = Some(*d);
    }
    let all_days: HashSet<NaiveDate> = all.iter().map(|p| local(p.t).date_naive()).collect();
    let mut d = if all_days.contains(&today) { today } else { today - chrono::Duration::days(1) };
    while all_days.contains(&d) {
        s.streak += 1;
        d -= chrono::Duration::days(1);
    }

    s.busiest_day = days.iter().max_by(|a, b| a.1.total_cmp(b.1)).map(|(d, m)| Bucket { label: format!("{} {} {}", MONTHS[d.month0() as usize], d.day(), d.year()), minutes: *m });
    let mut pl: Vec<Bucket> = platforms.into_iter().map(|(k, m)| Bucket { label: k.to_string(), minutes: m }).collect();
    pl.sort_by(|a, b| b.minutes.total_cmp(&a.minutes));
    s.platforms = pl;
    s.series = series.into_iter().map(|(label, minutes)| Bucket { label, minutes }).collect();
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TrackVersion;

    fn play(t: u64, secs: f64, title: &str, artist: &str) -> Play {
        let v = TrackVersion {
            platform: Platform::Ytmusic, url: format!("u/{title}"), id: String::new(), title: title.into(), artist: artist.into(),
            duration: Some(200.0), thumbnail: None, preview: false, partial: false, artist_url: None, album: None, rank: 0, matched_for: None, isrc_match: false,
        };
        Play { t, secs, est: false, platform: Some(Platform::Ytmusic), track: Track::from_version(v) }
    }

    #[test]
    fn summary_counts_plays_and_artists() {
        let now = now_ms();
        let day = 86_400_000;
        let v = vec![
            play(now - 3 * day, 200.0, "A1", "Alpha"),
            play(now - 2 * day, 190.0, "A1", "Alpha"),
            play(now - day, 15.0, "B1", "Beta"), // a skip
            play(now - 1000, 120.0, "B2", "Beta, Gamma"),
        ];
        *PLAYS.lock().unwrap() = Some(v);
        let s = summarize("4w", None);
        assert_eq!(s.plays, 3);
        assert_eq!(s.skips, 1);
        assert_eq!(s.top_tracks[0].track.title, "A1");
        assert_eq!(s.top_tracks[0].plays, 2);
        assert_eq!(s.top_artists[0].name, "Alpha");
        assert_eq!(s.artists, 2);
        assert_eq!(s.series.len(), 28);
        assert!(s.streak >= 3);
        assert!((s.minutes - 525.0 / 60.0).abs() < 1e-6);
    }
}
