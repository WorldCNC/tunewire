//! Tunewire accounts (Supabase Auth). Sign-in is Discord only: it opens the
//! browser (PKCE) and catches the redirect on a loopback port. There are no
//! guest accounts. The refresh token lives in the OS keychain; the page asks
//! for a fresh access token and talks to the database/realtime itself.
//!
//! Refreshing is single-flight: refresh tokens are one-time use, so two
//! concurrent refreshes would spend the same token twice. Only a rejected
//! grant signs the user out; network or server trouble keeps the session.
//!
//! Staying signed in across restarts and updates: the whole session (not
//! just the refresh token) is saved, so a restart within the hour reuses the
//! access token and spends nothing. Before an update restarts the app,
//! refreshing is paused and any refresh in flight finishes saving, so the
//! new version never starts with a token the server already replaced. If a
//! token is refused because the previous run (still closing) just replaced
//! it, the newer saved one is used instead of signing out.
//!
//! Project URL and anon key are public by design (row level security does
//! the protecting). They're baked in at build time (TUNEWIRE_SUPABASE_URL,
//! TUNEWIRE_SUPABASE_ANON_KEY) or set in settings for development.

use std::io::Write;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::store::now_ms;

/// Loopback ports tried in order. The auth settings allow-list
/// http://127.0.0.1:559*/callback, so any of them works.
const REDIRECT_PORTS: [u16; 5] = [5590, 5591, 5592, 5593, 5594];
const KEY_REFRESH: &str = "cloud_refresh_token";

#[derive(Debug, Clone, Serialize)]
pub struct CloudConfig {
    pub url: String,
    pub anon_key: String,
    /// Where the `share` function (link previews) is served; empty = links
    /// go straight to the GitHub Pages landing page.
    pub share_url: String,
}

fn share_url() -> String {
    option_env!("TUNEWIRE_SHARE_URL").unwrap_or("").trim_end_matches('/').to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSession {
    pub access_token: String,
    pub user_id: String,
    pub name: String,
    pub avatar: Option<String>,
    /// ms timestamp.
    pub expires_at: u64,
    /// Discord profile banner and accent color, read at sign-in only (the
    /// Discord token comes with the sign-in, not with refreshes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<u32>,
    /// Discord avatar decoration (the frame around the avatar), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoration: Option<String>,
}

/// The avatar decoration's image (animated PNG) from a Discord user object.
fn discord_decoration(me: &Value) -> Option<String> {
    let asset = me.pointer("/avatar_decoration_data/asset").and_then(Value::as_str)?;
    asset.trim_start_matches("a_").chars().all(|c| c.is_ascii_hexdigit()).then(|| format!("https://cdn.discordapp.com/avatar-decoration-presets/{asset}.png?size=160&passthrough=true"))
}

/// Banner image URL (Discord CDN) and accent color from a Discord user object.
fn discord_look(me: &Value) -> (Option<String>, Option<u32>) {
    let id = me.get("id").and_then(Value::as_str).filter(|i| i.chars().all(|c| c.is_ascii_digit()));
    let hash = me.get("banner").and_then(Value::as_str).filter(|h| h.trim_start_matches("a_").chars().all(|c| c.is_ascii_hexdigit()));
    let banner = match (id, hash) {
        (Some(id), Some(h)) => Some(format!("https://cdn.discordapp.com/banners/{id}/{h}.{}?size=1024", if h.starts_with("a_") { "gif" } else { "png" })),
        _ => None,
    };
    let accent = me.get("accent_color").and_then(Value::as_u64).filter(|c| *c <= 0xFF_FFFF).map(|c| c as u32);
    (banner, accent)
}

/// The signed-in Discord account's banner, accent color and avatar
/// decoration, via the sign-in's Discord token.
fn discord_profile(provider_token: &str) -> (Option<String>, Option<u32>, Option<String>) {
    let res = crate::http::agent().get("https://discord.com/api/v10/users/@me").header("Authorization", &format!("Bearer {provider_token}")).call();
    let me = match res {
        Ok(mut r) if r.status().as_u16() == 200 => r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()),
        _ => None,
    };
    match me {
        Some(v) => {
            let (b, c) = discord_look(&v);
            (b, c, discord_decoration(&v))
        }
        None => (None, None, None),
    }
}

struct Inner {
    session: CloudSession,
    refresh: String,
}

static SESSION: Mutex<Option<Inner>> = Mutex::new(None);
/// Held for the whole refresh request (see the module docs).
static REFRESHING: Mutex<()> = Mutex::new(());
static OVERRIDE: Mutex<Option<CloudConfig>> = Mutex::new(None);
/// Set while the app restarts for an update: no new refreshes.
static FROZEN: AtomicBool = AtomicBool::new(false);
static FALLBACK_FILE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

pub fn init(dir: &std::path::Path, settings_url: &str, settings_key: &str) {
    *FALLBACK_FILE.lock().unwrap() = Some(dir.join("cloud_session"));
    // Development only: in a release, settings (which the page can change)
    // must never be able to send the sign-in token to another server.
    if cfg!(debug_assertions) && !settings_url.is_empty() && !settings_key.is_empty() {
        *OVERRIDE.lock().unwrap() = Some(CloudConfig { url: settings_url.trim_end_matches('/').to_string(), anon_key: settings_key.to_string(), share_url: share_url() });
    }
}

pub fn config() -> Option<CloudConfig> {
    if let Some(c) = OVERRIDE.lock().unwrap().clone() {
        return Some(c);
    }
    match (option_env!("TUNEWIRE_SUPABASE_URL"), option_env!("TUNEWIRE_SUPABASE_ANON_KEY")) {
        (Some(u), Some(k)) if !u.is_empty() && !k.is_empty() => Some(CloudConfig { url: u.trim_end_matches('/').to_string(), anon_key: k.to_string(), share_url: share_url() }),
        _ => None,
    }
}

fn cfg() -> AppResult<CloudConfig> {
    config().ok_or_else(|| AppError::Unavailable("online features aren't set up in this build".into()))
}

/// What's kept between runs: the refresh token, plus the session it came
/// with (so a restart can reuse a still-valid access token).
struct Stored {
    refresh: String,
    session: Option<CloudSession>,
}

// Storage. The refresh token goes in the keychain, or a private file where
// the keychain doesn't work; that file only exists while it's the newest
// copy (a keychain write failed), is read first, and is removed once the
// keychain takes a write again. The session itself (a short-lived access
// token, too big for some keychains) goes in a private file, tagged with the
// refresh token it belongs to.
fn token_tag(refresh: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(refresh.trim().as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect()
}

fn session_file() -> Option<std::path::PathBuf> {
    FALLBACK_FILE.lock().unwrap().as_ref().map(|p| p.with_file_name("cloud_session.json"))
}

/// Write `data` readable by this user only.
fn write_private(path: &std::path::Path, data: &[u8]) {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    if let Ok(mut f) = o.open(path) {
        let _ = f.write_all(data).and_then(|_| f.sync_all());
    }
}

fn save_stored(st: &Stored) {
    let file = FALLBACK_FILE.lock().unwrap().clone();
    if crate::secrets::set(KEY_REFRESH, &st.refresh).is_ok() {
        if let Some(p) = &file {
            let _ = std::fs::remove_file(p);
        }
    } else if let Some(p) = &file {
        write_private(p, st.refresh.as_bytes());
    }
    if let (Some(p), Some(session)) = (session_file(), &st.session) {
        if let Ok(data) = serde_json::to_vec(&serde_json::json!({ "for": token_tag(&st.refresh), "session": session })) {
            write_private(&p, &data);
        }
    }
}

fn load_stored() -> Option<Stored> {
    let file = FALLBACK_FILE.lock().unwrap().as_ref().and_then(|p| std::fs::read_to_string(p).ok());
    let refresh = file.filter(|s| !s.trim().is_empty()).or_else(|| crate::secrets::get(KEY_REFRESH))?.trim().to_string();
    if refresh.is_empty() {
        return None;
    }
    // The saved session only counts if it came with this refresh token.
    let session = session_file()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(|v| v.get("for").and_then(Value::as_str) == Some(token_tag(&refresh).as_str()))
        .and_then(|v| serde_json::from_value::<CloudSession>(v.get("session")?.clone()).ok());
    Some(Stored { refresh, session })
}

fn clear_refresh() {
    let _ = crate::secrets::set(KEY_REFRESH, "");
    // Separate statements: a lock guard lives until the end of its statement,
    // and session_file() takes the same lock.
    let token_file = FALLBACK_FILE.lock().unwrap().clone();
    let files = [token_file, session_file()];
    for p in files.into_iter().flatten() {
        let _ = std::fs::remove_file(p);
    }
}

/// The update is about to restart the app (true), or that failed (false).
/// Waits for a refresh in flight, so its new token is saved before exiting.
pub fn freeze_for_restart(on: bool) {
    FROZEN.store(on, Ordering::SeqCst);
    if on {
        drop(REFRESHING.lock().unwrap_or_else(|e| e.into_inner()));
    }
}

/// How an auth request failed.
#[derive(Debug)]
enum AuthFail {
    /// The server rejected the grant (bad, used or revoked token): signed out.
    Rejected(String),
    Other(AppError),
}

impl From<AuthFail> for AppError {
    fn from(f: AuthFail) -> Self {
        match f {
            AuthFail::Rejected(m) => AppError::Unavailable(m),
            AuthFail::Other(e) => e,
        }
    }
}

/// Only these mean "this session is over"; anything else may be temporary.
fn is_rejected_grant(code: &str, msg: &str) -> bool {
    matches!(code, "invalid_grant" | "refresh_token_not_found" | "refresh_token_already_used" | "session_not_found" | "session_expired" | "user_not_found" | "user_banned")
        || msg.contains("Invalid Refresh Token")
}

/// Map an auth server reply (status + JSON body) to a result.
fn classify(status: u16, v: Value) -> Result<Value, AuthFail> {
    match status {
        200..=299 => Ok(v),
        400 | 401 | 403 => {
            let code = v.get("error_code").or_else(|| v.get("error")).and_then(Value::as_str).unwrap_or("");
            let msg = v.get("msg").or_else(|| v.get("error_description")).or_else(|| v.get("message")).and_then(Value::as_str).unwrap_or("sign-in failed").to_string();
            if is_rejected_grant(code, &msg) {
                Err(AuthFail::Rejected(msg))
            } else {
                Err(AuthFail::Other(AppError::Unavailable(format!("sign-in failed: {msg}"))))
            }
        }
        429 => Err(AuthFail::Other(AppError::Network("too many sign-in attempts, try again in a minute".into()))),
        c => Err(AuthFail::Other(AppError::Network(format!("Tunewire server returned HTTP {c}")))),
    }
}

fn post(path: &str, body: Value, bearer: Option<&str>) -> Result<Value, AuthFail> {
    let c = cfg().map_err(AuthFail::Other)?;
    let mut req = crate::http::agent().post(&format!("{}{path}", c.url)).header("apikey", &c.anon_key).header("Content-Type", "application/json");
    if let Some(b) = bearer {
        req = req.header("Authorization", &format!("Bearer {b}"));
    }
    // The shared agent returns HTTP errors as responses, so both the status
    // and the error body (`error_code`, `msg`) can be checked.
    let mut r = req.send(body.to_string()).map_err(|e| AuthFail::Other(AppError::Network(e.to_string())))?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().map_err(|e| AuthFail::Other(AppError::Network(e.to_string())))?;
    classify(status, if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::Null) })
}

fn adopt(v: &Value) -> AppResult<CloudSession> {
    let access = v.get("access_token").and_then(Value::as_str).ok_or_else(|| AppError::Unavailable("sign-in failed".into()))?.to_string();
    let refresh = v.get("refresh_token").and_then(Value::as_str).unwrap_or("").to_string();
    let expires_in = v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);
    let user = v.get("user").cloned().unwrap_or(Value::Null);
    if user.get("is_anonymous").and_then(Value::as_bool).unwrap_or(false) {
        // Guest accounts are gone: an old guest session just ends.
        clear_refresh();
        *SESSION.lock().unwrap() = None;
        return Err(AppError::Unavailable("guest accounts are no longer supported; sign in with Discord".into()));
    }
    let meta = user.get("user_metadata").cloned().unwrap_or(Value::Null);
    let s = |k: &str| meta.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_string);
    let (banner, accent_color, decoration) = v.get("provider_token").and_then(Value::as_str).filter(|t| !t.is_empty()).map(discord_profile).unwrap_or((None, None, None));
    let session = CloudSession {
        banner,
        accent_color,
        decoration,
        access_token: access,
        user_id: user.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
        name: s("full_name").or_else(|| s("name")).or_else(|| s("user_name")).unwrap_or_else(|| "Listener".into()),
        avatar: s("avatar_url"),
        expires_at: now_ms() + expires_in * 1000,
    };
    if !refresh.is_empty() {
        save_stored(&Stored { refresh: refresh.clone(), session: Some(session.clone()) });
    }
    *SESSION.lock().unwrap() = Some(Inner { session: session.clone(), refresh });
    Ok(session)
}

/// Sign in with Discord in the browser (the only sign-in method).
pub fn sign_in_blocking(provider: &str) -> AppResult<CloudSession> {
    let c = cfg()?;
    if provider != "discord" {
        return Err(AppError::Invalid("Tunewire accounts sign in with Discord".into()));
    }
    let (listener, port) = REDIRECT_PORTS
        .iter()
        .find_map(|&p| TcpListener::bind(("127.0.0.1", p)).ok().map(|l| (l, p)))
        .ok_or_else(|| AppError::Unavailable(format!("sign-in ports {}–{} are all busy; close the program using them and try again", REDIRECT_PORTS[0], REDIRECT_PORTS[REDIRECT_PORTS.len() - 1])))?;
    let (verifier, challenge) = crate::loopback::pkce();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let url = format!("{}/auth/v1/authorize?provider=discord&redirect_to={}&code_challenge={challenge}&code_challenge_method=s256", c.url, crate::search::percent_encode(&redirect));
    open::that_detached(&url).map_err(|e| AppError::Unavailable(format!("couldn't open the browser: {e}")))?;
    let code = crate::loopback::wait_for_code(listener, "/callback", None, "Tunewire", Instant::now() + Duration::from_secs(300))?;
    let v = post("/auth/v1/token?grant_type=pkce", serde_json::json!({ "auth_code": code, "code_verifier": verifier }), None)?;
    adopt(&v)
}

fn fresh_session() -> Option<CloudSession> {
    SESSION.lock().unwrap().as_ref().map(|i| i.session.clone()).filter(|s| s.expires_at > now_ms() + 120_000)
}

/// A valid session (refreshed when close to expiry), if signed in.
pub fn session_blocking() -> AppResult<Option<CloudSession>> {
    if config().is_none() {
        return Ok(None);
    }
    if let Some(s) = fresh_session() {
        return Ok(Some(s));
    }
    // One refresh at a time; whoever waited finds the new session ready.
    let _r = REFRESHING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = fresh_session() {
        return Ok(Some(s));
    }
    let current = SESSION.lock().unwrap().as_ref().filter(|i| !i.refresh.is_empty()).map(|i| Stored { refresh: i.refresh.clone(), session: Some(i.session.clone()) });
    let first = current.is_none();
    let Some(stored) = current.or_else(load_stored) else {
        return Ok(None);
    };
    let still_valid = |st: &Stored| st.session.clone().filter(|s| s.expires_at > now_ms() + 120_000);
    // Just started (or restarted for an update): the saved session is still
    // good, so nothing is spent.
    if first {
        if let Some(s) = still_valid(&stored) {
            *SESSION.lock().unwrap() = Some(Inner { session: s.clone(), refresh: stored.refresh });
            return Ok(Some(s));
        }
    }
    if FROZEN.load(Ordering::SeqCst) {
        // Restarting: keep the token for the next run.
        return Ok(stored.session.filter(|s| s.expires_at > now_ms()));
    }
    match refresh_with(&stored.refresh) {
        Ok(s) => Ok(s),
        Err(AuthFail::Rejected(e)) => {
            // Maybe the previous run (still closing) or a save we raced with
            // already replaced it: a newer saved token isn't "signed out".
            if let Some(newer) = load_stored().filter(|n| n.refresh != stored.refresh) {
                if let Some(s) = still_valid(&newer) {
                    *SESSION.lock().unwrap() = Some(Inner { session: s.clone(), refresh: newer.refresh });
                    return Ok(Some(s));
                }
                match refresh_with(&newer.refresh) {
                    Ok(s) => return Ok(s),
                    Err(AuthFail::Other(e)) => return Err(e),
                    Err(AuthFail::Rejected(_)) => {}
                }
            }
            // The session really ended (revoked, expired, used elsewhere).
            let _ = e;
            clear_refresh();
            *SESSION.lock().unwrap() = None;
            Ok(None)
        }
        // Offline or server trouble: keep the refresh token for next time,
        // and keep using the access token while it lasts.
        Err(AuthFail::Other(e)) => match stored.session.filter(|s| s.expires_at > now_ms() + 5_000) {
            Some(s) => {
                *SESSION.lock().unwrap() = Some(Inner { session: s.clone(), refresh: stored.refresh });
                Ok(Some(s))
            }
            None => Err(e),
        },
    }
}

/// Spend `refresh` for a new session (saved by `adopt`).
fn refresh_with(refresh: &str) -> Result<Option<CloudSession>, AuthFail> {
    let v = post("/auth/v1/token?grant_type=refresh_token", serde_json::json!({ "refresh_token": refresh.trim() }), None)?;
    // A guest session is refused by `adopt` (and cleared).
    Ok(adopt(&v).ok())
}

pub fn sign_out_blocking() {
    let token = SESSION.lock().unwrap().take().map(|i| i.session.access_token);
    if let Some(t) = token {
        let _ = post("/auth/v1/logout", serde_json::json!({}), Some(&t));
    }
    clear_refresh();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The session and its storage are process-wide: tests that touch them
    /// take turns (cargo runs tests in parallel).
    static GLOBALS: Mutex<()> = Mutex::new(());
    fn globals() -> std::sync::MutexGuard<'static, ()> {
        GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn only_rejected_grants_sign_out() {
        let rej = classify(400, serde_json::json!({ "error_code": "refresh_token_already_used", "msg": "Invalid Refresh Token: Already Used" }));
        assert!(matches!(rej, Err(AuthFail::Rejected(_))));
        let rej = classify(400, serde_json::json!({ "error": "invalid_grant", "error_description": "Invalid Refresh Token: Refresh Token Not Found" }));
        assert!(matches!(rej, Err(AuthFail::Rejected(_))));
        // Temporary trouble keeps the session.
        assert!(matches!(classify(502, Value::Null), Err(AuthFail::Other(AppError::Network(_)))));
        assert!(matches!(classify(429, Value::Null), Err(AuthFail::Other(AppError::Network(_)))));
        assert!(matches!(classify(400, serde_json::json!({ "error_code": "validation_failed", "msg": "bad" })), Err(AuthFail::Other(_))));
        assert!(classify(200, serde_json::json!({ "ok": true })).is_ok());
    }

    #[test]
    fn discord_banners_become_cdn_urls() {
        let (b, c) = discord_look(&serde_json::json!({ "id": "80351110224678912", "banner": "a_06c16474723fe537c283b8efa61a30c8", "accent_color": 5793266 }));
        assert_eq!(b.as_deref(), Some("https://cdn.discordapp.com/banners/80351110224678912/a_06c16474723fe537c283b8efa61a30c8.gif?size=1024"));
        assert_eq!(c, Some(5793266));
        let (b, _) = discord_look(&serde_json::json!({ "id": "1", "banner": "06c16474723fe537c283b8efa61a30c8" }));
        assert!(b.unwrap().ends_with(".png?size=1024"));
        // No banner, or something that isn't a banner hash: none.
        assert_eq!(discord_look(&serde_json::json!({ "id": "1", "banner": null })).0, None);
        assert_eq!(discord_look(&serde_json::json!({ "id": "1", "banner": "../../x" })).0, None);
        // Avatar decorations.
        assert_eq!(
            discord_decoration(&serde_json::json!({ "avatar_decoration_data": { "asset": "a_fed43ab12698df65902ba06727e20c0e", "sku_id": "1" } })).as_deref(),
            Some("https://cdn.discordapp.com/avatar-decoration-presets/a_fed43ab12698df65902ba06727e20c0e.png?size=160&passthrough=true")
        );
        assert_eq!(discord_decoration(&serde_json::json!({ "avatar_decoration_data": null })), None);
        assert_eq!(discord_decoration(&serde_json::json!({ "avatar_decoration_data": { "asset": "x/../y" } })), None);
    }

    #[test]
    fn guest_sessions_are_refused() {
        let _g = globals();
        let v = serde_json::json!({ "access_token": "t", "refresh_token": "", "user": { "id": "u", "is_anonymous": true } });
        assert!(adopt(&v).is_err());
    }

    fn fake_session(expires_in_ms: u64) -> CloudSession {
        CloudSession { access_token: "access".into(), user_id: "u1".into(), name: "Me".into(), avatar: None, expires_at: now_ms() + expires_in_ms, banner: None, accent_color: None, decoration: None }
    }

    /// Staying signed in across restarts, without a server.
    #[test]
    fn restarts_keep_the_session() {
        let _g = globals();
        use crate::secrets::fake::BROKEN;
        let dir = std::env::temp_dir().join(format!("tw-cloud-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        *FALLBACK_FILE.lock().unwrap() = Some(dir.join("cloud_session"));
        // Nothing listens here: any refresh attempt fails with a network error.
        *OVERRIDE.lock().unwrap() = Some(CloudConfig { url: "http://127.0.0.1:9".into(), anon_key: "k".into(), share_url: String::new() });
        let restart = || *SESSION.lock().unwrap() = None;

        // Saved with its session; read back as long as it's the same token.
        save_stored(&Stored { refresh: "r1".into(), session: Some(fake_session(3_600_000)) });
        assert_eq!(load_stored().unwrap().session.unwrap().user_id, "u1");
        crate::secrets::set(KEY_REFRESH, "r-other").unwrap();
        assert!(load_stored().unwrap().session.is_none(), "a session never pairs with another token");

        // Restart within the hour: the saved session, nothing spent.
        save_stored(&Stored { refresh: "r1".into(), session: Some(fake_session(3_600_000)) });
        restart();
        assert_eq!(session_blocking().unwrap().unwrap().access_token, "access");
        assert_eq!(load_stored().unwrap().refresh, "r1");

        // Offline at start with a token near expiry: still signed in.
        save_stored(&Stored { refresh: "r1".into(), session: Some(fake_session(60_000)) });
        restart();
        assert!(session_blocking().unwrap().is_some(), "offline is not signed out");
        // Expired and offline: an error (try again later), and the token is kept.
        save_stored(&Stored { refresh: "r1".into(), session: Some(fake_session(0)) });
        restart();
        assert!(session_blocking().is_err());
        assert_eq!(load_stored().unwrap().refresh, "r1");

        // Restarting for an update: no refresh is started.
        save_stored(&Stored { refresh: "r1".into(), session: Some(fake_session(60_000)) });
        restart();
        freeze_for_restart(true);
        assert!(session_blocking().unwrap().is_some());
        freeze_for_restart(false);

        // Keychain locked: the newest token goes to the file and wins over
        // the older one still in the keychain…
        BROKEN.store(true, Ordering::SeqCst);
        save_stored(&Stored { refresh: "r2".into(), session: None });
        BROKEN.store(false, Ordering::SeqCst);
        assert_eq!(crate::secrets::get(KEY_REFRESH).as_deref(), Some("r1"));
        assert_eq!(load_stored().unwrap().refresh, "r2", "the newest copy wins");
        // …until the keychain works again.
        save_stored(&Stored { refresh: "r3".into(), session: None });
        assert!(!dir.join("cloud_session").exists());
        assert_eq!(load_stored().unwrap().refresh, "r3");

        // Older versions kept only the bare token.
        crate::secrets::set(KEY_REFRESH, "legacy").unwrap();
        assert_eq!(load_stored().unwrap().refresh, "legacy");

        clear_refresh();
        assert!(load_stored().is_none());
        *OVERRIDE.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The previous run replaced the token while this one started with the
    /// old one: the newer saved token is used, not "signed out".
    /// Local stack: SB_URL, SB_ANON, SB_EMAIL/SB_PASSWORD of a user.
    #[test]
    #[ignore]
    fn live_replaced_token_is_not_a_sign_out() {
        let _g = globals();
        *OVERRIDE.lock().unwrap() = Some(CloudConfig { url: std::env::var("SB_URL").unwrap(), anon_key: std::env::var("SB_ANON").unwrap(), share_url: String::new() });
        *FALLBACK_FILE.lock().unwrap() = Some(std::env::temp_dir().join(format!("tw-cloud-live-{}", now_ms())));
        let grant = post("/auth/v1/token?grant_type=password", serde_json::json!({ "email": std::env::var("SB_EMAIL").unwrap(), "password": std::env::var("SB_PASSWORD").unwrap() }), None)
            .unwrap_or_else(|_| panic!("password sign-in"));
        let t1 = grant["refresh_token"].as_str().unwrap().to_string();
        // "The other run" spends t1 and saves t2.
        let r = post("/auth/v1/token?grant_type=refresh_token", serde_json::json!({ "refresh_token": t1 }), None).unwrap_or_else(|_| panic!("refresh"));
        let t2 = r["refresh_token"].as_str().unwrap().to_string();
        // Past the server's reuse window: t1 alone would now be refused.
        std::thread::sleep(Duration::from_secs(12));
        save_stored(&Stored { refresh: t2.clone(), session: None });
        // This run still holds t1 (read before the other run saved).
        *SESSION.lock().unwrap() = Some(Inner { session: fake_session(0), refresh: t1 });
        let s = session_blocking().expect("no error").expect("still signed in");
        assert!(!s.user_id.is_empty());
        // What's saved still works (whether the server answered t1 with t2's
        // session or we fell back to t2).
        let saved = load_stored().unwrap().refresh;
        assert!(refresh_with(&saved).is_ok(), "the saved token is usable");
        sign_out_blocking();
    }

    /// Against the local stack: `supabase start` in the repo (see supabase/).
    #[test]
    #[ignore]
    fn live_refresh_is_single_flight() {
        let _g = globals();
        let url = std::env::var("SB_URL").unwrap();
        let key = std::env::var("SB_ANON").unwrap();
        *OVERRIDE.lock().unwrap() = Some(CloudConfig { url, anon_key: key, share_url: String::new() });
        *FALLBACK_FILE.lock().unwrap() = Some(std::env::temp_dir().join("tw-cloud-test"));
        // Needs a Discord sign-in first (SB_REFRESH = a refresh token).
        let refresh = std::env::var("SB_REFRESH").unwrap();
        save_stored(&Stored { refresh, session: None });
        let handles: Vec<_> = (0..4).map(|_| std::thread::spawn(session_blocking)).collect();
        let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap().unwrap().unwrap().user_id).collect();
        assert!(ids.windows(2).all(|w| w[0] == w[1]));
        sign_out_blocking();
    }
}
