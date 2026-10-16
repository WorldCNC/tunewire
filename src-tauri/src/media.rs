//! OS media controls: keyboard/headset media keys and the system "now playing"
//! overlay (Windows SMTC, MPRIS on Linux, Now Playing on macOS).
//!
//! The controls live on the main thread (Windows and macOS need that); updates
//! are posted there and only sent when something actually changed.

use std::cell::RefCell;
use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};
use tauri::{AppHandle, Manager};

use crate::commands::playback;
use crate::state::{AppState, PlaybackState};

thread_local! {
    static CONTROLS: RefCell<Option<MediaControls>> = const { RefCell::new(None) };
    /// (metadata signature, playback signature) last sent.
    static LAST: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
}

/// Call from `setup` (main thread).
pub fn init(app: &tauri::App) {
    #[cfg(target_os = "windows")]
    let hwnd = match app.get_webview_window("main").and_then(|w| w.hwnd().ok()) {
        Some(h) => Some(h.0 as *mut std::ffi::c_void),
        None => return,
    };
    #[cfg(not(target_os = "windows"))]
    let hwnd = None;

    let config = PlatformConfig { display_name: "Tunewire", dbus_name: "tunewire", hwnd };
    let mut controls = match MediaControls::new(config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tunewire: media controls unavailable: {e:?}");
            return;
        }
    };
    let handle = app.handle().clone();
    if let Err(e) = controls.attach(move |ev| on_event(&handle, ev)) {
        eprintln!("tunewire: media controls unavailable: {e:?}");
        return;
    }
    CONTROLS.with(|c| *c.borrow_mut() = Some(controls));
}

fn on_event(app: &AppHandle, ev: MediaControlEvent) {
    let st = app.state::<AppState>();
    let pos = || st.audio.status().position;
    match ev {
        MediaControlEvent::Play => playback::resume(app.clone(), st.clone()),
        MediaControlEvent::Pause => playback::pause(app.clone(), st.clone()),
        MediaControlEvent::Toggle => playback::toggle_pause(app.clone(), st.clone()),
        MediaControlEvent::Next => playback::next_track(app.clone()),
        MediaControlEvent::Previous => playback::previous_track(app.clone(), st.clone()),
        MediaControlEvent::Stop => playback::stop(app.clone(), st.clone()),
        MediaControlEvent::Seek(dir) => playback::seek(app.clone(), st.clone(), shifted(pos(), dir, 10.0)),
        MediaControlEvent::SeekBy(dir, by) => playback::seek(app.clone(), st.clone(), shifted(pos(), dir, by.as_secs_f64())),
        MediaControlEvent::SetPosition(MediaPosition(p)) => playback::seek(app.clone(), st.clone(), p.as_secs_f64()),
        MediaControlEvent::SetVolume(v) => {
            if let Ok(v) = playback::set_volume(app.clone(), st.clone(), v as f32) {
                let _ = tauri::Emitter::emit(app, "volume-changed", v);
            }
        }
        MediaControlEvent::Raise => crate::show_main(app),
        MediaControlEvent::Quit => app.exit(0),
        MediaControlEvent::OpenUri(_) => {}
    }
}

fn shifted(pos: f64, dir: SeekDirection, by: f64) -> f64 {
    match dir {
        SeekDirection::Forward => pos + by,
        SeekDirection::Backward => (pos - by).max(0.0),
    }
}

/// Mirror the player state to the OS (cheap when nothing changed).
pub fn update(app: &AppHandle, ps: &PlaybackState) {
    let (meta, play) = describe(ps);
    let _ = app.run_on_main_thread(move || {
        CONTROLS.with(|c| {
            let mut c = c.borrow_mut();
            let Some(controls) = c.as_mut() else { return };
            LAST.with(|last| {
                let mut last = last.borrow_mut();
                if let Some(m) = &meta {
                    if last.0 != m.sig {
                        last.0 = m.sig.clone();
                        let _ = controls.set_metadata(MediaMetadata {
                            title: Some(&m.title),
                            artist: Some(&m.artist),
                            album: Some(&m.album),
                            cover_url: m.cover.as_deref(),
                            duration: m.duration.map(Duration::from_secs_f64),
                        });
                    }
                }
                if last.1 != play.0 {
                    last.1 = play.0.clone();
                    let _ = controls.set_playback(play.1.clone());
                }
            });
        });
    });
}

struct Meta {
    sig: String,
    title: String,
    artist: String,
    album: String,
    cover: Option<String>,
    duration: Option<f64>,
}

fn describe(ps: &PlaybackState) -> (Option<Meta>, (String, MediaPlayback)) {
    let meta = ps.current.as_ref().map(|item| {
        let v = ps.version.as_ref();
        let cover = v
            .and_then(|v| v.thumbnail.clone())
            .or(item.track.thumbnail.clone())
            .filter(|u| u.starts_with("http"))
            .map(|u| u.replace("-original.", "-t500x500.").replace("/mqdefault.jpg", "/hqdefault.jpg"));
        let duration = ps.duration.or(item.track.duration);
        Meta {
            sig: format!("{}|{:?}|{:?}|{:?}", item.uid, v.map(|v| &v.url), cover, duration.map(|d| d as i64)),
            title: item.track.title.clone(),
            artist: item.track.artist.clone(),
            album: v.and_then(|v| v.album.clone()).unwrap_or_default(),
            cover,
            duration,
        }
    });
    let progress = Some(MediaPosition(Duration::from_secs_f64(ps.position.max(0.0))));
    let playback = match ps.status {
        "playing" => MediaPlayback::Playing { progress },
        "paused" | "loading" => MediaPlayback::Paused { progress },
        _ if ps.current.is_some() => MediaPlayback::Paused { progress },
        _ => MediaPlayback::Stopped,
    };
    // While playing, resend the position every 5 s (MPRIS clients read it
    // as-is, and it keeps the Windows overlay's progress bar honest).
    let kind = match &playback {
        MediaPlayback::Playing { .. } => format!("play{}", (ps.position / 5.0) as i64),
        MediaPlayback::Paused { .. } => "pause".into(),
        MediaPlayback::Stopped => "stop".into(),
    };
    (meta, (format!("{kind}|{:?}", ps.current.as_ref().map(|c| c.uid)), playback))
}

/// After a seek the OS needs the new position even though the state didn't change.
pub fn position_changed(app: &AppHandle) {
    let _ = app.run_on_main_thread(|| LAST.with(|l| l.borrow_mut().1.clear()));
    if let Some(st) = app.try_state::<AppState>() {
        update(app, &st.playback_state());
    }
}
