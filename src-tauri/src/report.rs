//! Automatic error reports to the developer's Discord webhook.
//!
//! The webhook URL is baked in at build time from TUNEWIRE_REPORT_WEBHOOK
//! (a CI secret), so it never appears in the source. Reports are short and
//! scrubbed (home folder, stream URLs), de-duplicated, rate-limited, and can
//! be turned off in Settings → Privacy.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WEBHOOK: Option<&str> = option_env!("TUNEWIRE_REPORT_WEBHOOK");
const MAX_PER_SESSION: usize = 15;

static ENABLED: AtomicBool = AtomicBool::new(true);
static SENT: AtomicUsize = AtomicUsize::new(0);
static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static LAST: Mutex<Option<Instant>> = Mutex::new(None);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn available() -> bool {
    WEBHOOK.is_some_and(|w| w.starts_with("https://discord.com/api/webhooks/") || w.starts_with("https://discordapp.com/api/webhooks/"))
}

/// Remove personal bits: home folder paths, user names in paths, long URLs.
fn scrub(s: &str) -> String {
    let mut out = s.to_string();
    for var in ["HOME", "USERPROFILE"] {
        if let Ok(home) = std::env::var(var) {
            if home.len() > 3 {
                out = out.replace(&home, "~");
                out = out.replace(&home.replace('\\', "/"), "~");
            }
        }
    }
    if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
        if user.len() > 2 {
            out = out.replace(&format!("/{user}/"), "/<user>/").replace(&format!("\\{user}\\"), "\\<user>\\");
        }
    }
    // Stream/CDN URLs carry tokens and IPs: keep only the host.
    let mut cleaned = String::with_capacity(out.len());
    for word in out.split_inclusive(char::is_whitespace) {
        let w = word.trim_end();
        if (w.starts_with("http://") || w.starts_with("https://")) && (w.contains("googlevideo") || w.contains('?') || w.len() > 90) {
            let host = w.split('/').nth(2).unwrap_or("");
            cleaned.push_str(&format!("https://{host}/…"));
            cleaned.push_str(&word[w.len()..]);
        } else {
            cleaned.push_str(word);
        }
    }
    cleaned
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn payload(kind: &str, message: &str, details: &str) -> serde_json::Value {
    let color = match kind {
        "crash" => 0xE5_3935,
        "frontend" => 0xFB_8C00,
        _ => 0xFD_D835,
    };
    serde_json::json!({
        "username": "Tunewire error reports",
        "embeds": [{
            "title": clip(&format!("{kind}: {}", scrub(message).lines().next().unwrap_or("")), 240),
            "description": clip(&format!("```\n{}\n```", scrub(&format!("{message}\n{details}")).trim()), 3900),
            "color": color,
            "fields": [
                { "name": "Version", "value": env!("CARGO_PKG_VERSION"), "inline": true },
                { "name": "OS", "value": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH), "inline": true },
            ],
        }]
    })
}

fn should_send(kind: &str, message: &str) -> bool {
    if !available() || !ENABLED.load(Ordering::Relaxed) || SENT.load(Ordering::Relaxed) >= MAX_PER_SESSION {
        return false;
    }
    let sig = format!("{kind}|{}", message.lines().next().unwrap_or("").chars().take(160).collect::<String>());
    let mut seen = SEEN.lock().unwrap();
    if !seen.get_or_insert_with(HashSet::new).insert(sig) {
        return false; // already reported this session
    }
    let mut last = LAST.lock().unwrap();
    if last.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) && kind != "crash" {
        return false;
    }
    *last = Some(Instant::now());
    SENT.fetch_add(1, Ordering::Relaxed);
    true
}

fn send_blocking(body: serde_json::Value) {
    let Some(url) = WEBHOOK else { return };
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(4))).build().into();
    let _ = agent.post(url).header("Content-Type", "application/json").send(body.to_string());
}

/// Report in the background.
pub fn report(kind: &str, message: &str, details: &str) {
    if !should_send(kind, message) {
        return;
    }
    let body = payload(kind, message, details);
    std::thread::spawn(move || send_blocking(body));
}

/// Errors worth a report: not the user's network, not songs that simply
/// aren't available, not cancellations.
pub fn report_app_error(context: &str, e: &crate::error::AppError) {
    use crate::error::AppError::*;
    match e {
        Network(_) | Timeout(_) | Cancelled | Unavailable(_) | RegionLocked | Drm(_) | EmptyResults | NotFound(_) | Invalid(_) | YtDlpMissing => {}
        _ => report("error", &format!("{context}: {e}"), ""),
    }
}

/// Crashes: send synchronously (the process aborts right after).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        if should_send("crash", &format!("{msg} at {at}")) {
            send_blocking(payload("crash", &format!("{msg}"), &format!("at {at} (thread {thread})")));
        }
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_paths_and_stream_urls() {
        std::env::set_var("HOME", "/home/alice");
        let s = scrub("failed opening /home/alice/Music/a.mp3 from https://rr3---sn-x.googlevideo.com/videoplayback?expire=1&ip=1.2.3.4 ok");
        assert!(!s.contains("alice"), "{s}");
        assert!(s.contains("~/Music/a.mp3"));
        assert!(s.contains("https://rr3---sn-x.googlevideo.com/…"));
        assert!(!s.contains("ip=1.2.3.4"));
    }

    #[test]
    fn dedupes() {
        // No webhook in tests: never sends, never panics.
        assert!(!should_send("error", "x"));
        let p = payload("error", "boom", "details");
        assert_eq!(p["embeds"][0]["title"], "error: boom");
    }
}
