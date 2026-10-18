//! Discord Rich Presence ("Listening to …") over Discord's local IPC socket.
//! Runs on its own thread; the app feeds it playback snapshots and it decides
//! when to push an update (Discord allows ~5 updates / 20 s).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, ActivityType, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

#[derive(Clone, PartialEq)]
pub struct Presence {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub platform: String,
    pub url: String,
    pub cover: Option<String>,
    pub playing: bool,
    pub position: f64,
    pub duration: Option<f64>,
}

enum Msg {
    Config { enabled: bool, client_id: String, buttons: bool },
    Update(Option<Presence>),
}

/// Human-readable connection state, shown in Settings.
pub type Status = Arc<Mutex<String>>;

#[derive(Clone)]
pub struct DiscordHandle(Sender<Msg>, Status);

impl DiscordHandle {
    pub fn spawn() -> DiscordHandle {
        let (tx, rx) = mpsc::channel();
        let status: Status = Arc::new(Mutex::new("off".into()));
        let st = status.clone();
        std::thread::Builder::new().name("discord-rpc".into()).spawn(move || run(rx, st)).expect("spawn discord thread");
        DiscordHandle(tx, status)
    }
    pub fn status(&self) -> String {
        self.1.lock().unwrap().clone()
    }
    pub fn configure(&self, enabled: bool, client_id: &str, buttons: bool) {
        let _ = self.0.send(Msg::Config { enabled, client_id: client_id.trim().to_string(), buttons });
    }
    pub fn update(&self, p: Option<Presence>) {
        let _ = self.0.send(Msg::Update(p));
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn set(status: &Status, s: impl Into<String>) {
    *status.lock().unwrap() = s.into();
}

/// Read Discord's reply to a command; returns its error message if it rejected it.
fn reply_error(cl: &mut DiscordIpcClient) -> Option<String> {
    match cl.recv() {
        Ok((_, v)) if v.get("evt").and_then(|e| e.as_str()) == Some("ERROR") => Some(v.pointer("/data/message").and_then(|m| m.as_str()).unwrap_or("rejected").to_string()),
        _ => None,
    }
}

fn run(rx: Receiver<Msg>, status: Status) {
    let (mut enabled, mut client_id, mut buttons) = (false, String::new(), true);
    let mut client: Option<DiscordIpcClient> = None;
    let mut want: Option<Presence> = None;
    let mut sent: Option<Option<Presence>> = None; // what Discord currently shows
    let mut last_push = Instant::now() - Duration::from_secs(60);
    let mut next_connect = Instant::now();

    loop {
        // Disabled: sleep until reconfigured instead of polling.
        let msg = if enabled { rx.recv_timeout(Duration::from_millis(500)) } else { rx.recv().map_err(|_| RecvTimeoutError::Disconnected) };
        match msg {
            Ok(Msg::Config { enabled: e, client_id: c, buttons: b }) => {
                if c != client_id || !e {
                    if let Some(mut cl) = client.take() {
                        let _ = cl.clear_activity();
                        let _ = cl.close();
                    }
                }
                (enabled, client_id, buttons) = (e, c, b);
                next_connect = Instant::now();
                sent = None;
            }
            Ok(Msg::Update(p)) => want = p,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if !enabled {
            set(&status, "off");
            continue;
        }
        if client_id.is_empty() {
            set(&status, "error: no application ID set");
            continue;
        }
        if client.is_none() {
            if Instant::now() < next_connect {
                continue;
            }
            let mut cl = DiscordIpcClient::new(&client_id);
            match cl.connect() {
                Ok(()) => {
                    client = Some(cl);
                    sent = None;
                    set(&status, "connected");
                }
                Err(e) => {
                    // No socket (Discord closed, or Vesktop with arRPC off): retry later.
                    let msg = e.to_string();
                    set(&status, if msg.to_lowercase().contains("not found") || msg.contains("IPC") { "waiting: no Discord IPC socket found".to_string() } else { format!("waiting: {msg}") });
                    next_connect = Instant::now() + Duration::from_secs(10);
                    continue;
                }
            }
        }
        // Decide whether Discord's view is stale.
        let stale = match (&sent, &want) {
            (None, _) => true,
            (Some(None), None) => false,
            (Some(Some(a)), Some(b)) => {
                a.key != b.key || a.playing != b.playing || {
                    // Position drift (seek) vs. what the timestamps imply.
                    let elapsed = last_push.elapsed().as_secs_f64();
                    let expected = if a.playing { a.position + elapsed } else { a.position };
                    (b.position - expected).abs() > 3.0
                }
            }
            _ => true,
        };
        if !stale || last_push.elapsed() < Duration::from_secs(4) {
            continue;
        }
        let cl = client.as_mut().unwrap();
        let res = match &want {
            None => cl.clear_activity(),
            Some(p) => {
                let state = if p.playing { format!("by {}", p.artist) } else { format!("Paused · {}", p.artist) };
                let caption = format!("{} · {}", p.title, p.platform);
                let mut act = Activity::new().activity_type(ActivityType::Listening).details(&p.title).state(&state);
                // Only send fields every client renders: Vesktop's web client
                // (arRPC) shows "null" for status_display_type and for captions
                // without an image.
                if let Some(c) = p.cover.as_deref().filter(|c| c.starts_with("https://")) {
                    act = act.assets(Assets::new().large_image(c).large_text(&caption));
                }
                if p.playing {
                    let start = now() - p.position as i64;
                    let mut ts = Timestamps::new().start(start);
                    if let Some(d) = p.duration {
                        ts = ts.end(start + d as i64);
                    }
                    act = act.timestamps(ts);
                }
                let label = format!("Open on {}", p.platform);
                if buttons && p.url.starts_with("https://") {
                    act = act.buttons(vec![Button::new(&label, &p.url)]);
                }
                cl.set_activity(act)
            }
        };
        last_push = Instant::now();
        match res {
            Ok(()) => {
                match reply_error(cl) {
                    Some(err) => set(&status, format!("error: Discord rejected the update ({err})")),
                    None => set(&status, "connected"),
                }
                sent = Some(want.clone());
            }
            Err(_) => {
                client = None; // Discord closed; reconnect later
                set(&status, "waiting: connection lost");
                next_connect = Instant::now() + Duration::from_secs(5);
            }
        }
    }
}
