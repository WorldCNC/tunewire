//! Tunewire accounts (Supabase Auth). Sign-in is Discord only: it opens the
//! browser (PKCE) and catches the redirect on a loopback port. There are no
//! guest accounts. The refresh token lives in the OS keychain; the page asks
//! for a fresh access token and talks to the database/realtime itself.
//!
//! Refreshing is single-flight: refresh tokens are one-time use, so two
//! concurrent refreshes would spend the same token twice. Only a rejected
//! grant signs the user out; network or server trouble keeps the session.
//!
//! Project URL and anon key are public by design (row level security does
//! the protecting). They're baked in at build time (TUNEWIRE_SUPABASE_URL,
//! TUNEWIRE_SUPABASE_ANON_KEY) or set in settings for development.

use std::io::Write;
use std::net::TcpListener;
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
    asset
        .trim_start_matches("a_")
        .chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| format!("https://cdn.discordapp.com/avatar-decoration-presets/{asset}.png?size=160&passthrough=true"))
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
static FALLBACK_FILE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

pub fn init(dir: &std::path::Path, settings_url: &str, settings_key: &str) {
    *FALLBACK_FILE.lock().unwrap() = Some(dir.join("cloud_session"));
    if !settings_url.is_empty() && !settings_key.is_empty() {
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

// Token storage: keychain, or a private file where there is none.
fn save_refresh(token: &str) {
    if crate::secrets::set(KEY_REFRESH, token).is_err() {
        if let Some(p) = FALLBACK_FILE.lock().unwrap().as_ref() {
            // Created private: never briefly readable by other users.
            let mut o = std::fs::OpenOptions::new();
            o.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            if let Ok(mut f) = o.open(p) {
                let _ = f.write_all(token.as_bytes());
            }
        }
    }
}

fn load_refresh() -> Option<String> {
    crate::secrets::get(KEY_REFRESH).or_else(|| FALLBACK_FILE.lock().unwrap().as_ref().and_then(|p| std::fs::read_to_string(p).ok()).filter(|s| !s.trim().is_empty()))
}

fn clear_refresh() {
    let _ = crate::secrets::set(KEY_REFRESH, "");
    if let Some(p) = FALLBACK_FILE.lock().unwrap().as_ref() {
        let _ = std::fs::remove_file(p);
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
    let mut req = crate::http::agent()
        .post(&format!("{}{path}", c.url))
        .header("apikey", &c.anon_key)
        .header("Content-Type", "application/json");
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
        save_refresh(&refresh);
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
    let url = format!(
        "{}/auth/v1/authorize?provider=discord&redirect_to={}&code_challenge={challenge}&code_challenge_method=s256",
        c.url,
        crate::search::percent_encode(&redirect)
    );
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
    let current = SESSION.lock().unwrap().as_ref().map(|i| i.refresh.clone()).filter(|r| !r.is_empty());
    let Some(refresh) = current.or_else(load_refresh) else { return Ok(None) };
    match post("/auth/v1/token?grant_type=refresh_token", serde_json::json!({ "refresh_token": refresh.trim() }), None) {
        // A guest session is refused by `adopt` (and cleared).
        Ok(v) => Ok(adopt(&v).ok()),
        Err(AuthFail::Rejected(_)) => {
            // The session really ended (revoked, expired, used elsewhere).
            clear_refresh();
            *SESSION.lock().unwrap() = None;
            Ok(None)
        }
        // Offline or server trouble: keep the refresh token for next time.
        Err(AuthFail::Other(e)) => Err(e),
    }
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
        let v = serde_json::json!({ "access_token": "t", "refresh_token": "", "user": { "id": "u", "is_anonymous": true } });
        assert!(adopt(&v).is_err());
    }

    /// Against the local stack: `supabase start` in the repo (see supabase/).
    #[test]
    #[ignore]
    fn live_refresh_is_single_flight() {
        let url = std::env::var("SB_URL").unwrap();
        let key = std::env::var("SB_ANON").unwrap();
        *OVERRIDE.lock().unwrap() = Some(CloudConfig { url, anon_key: key, share_url: String::new() });
        *FALLBACK_FILE.lock().unwrap() = Some(std::env::temp_dir().join("tw-cloud-test"));
        // Needs a Discord sign-in first (SB_REFRESH = a refresh token).
        let refresh = std::env::var("SB_REFRESH").unwrap();
        save_refresh(&refresh);
        let handles: Vec<_> = (0..4).map(|_| std::thread::spawn(session_blocking)).collect();
        let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap().unwrap().unwrap().user_id).collect();
        assert!(ids.windows(2).all(|w| w[0] == w[1]));
        sign_out_blocking();
    }
}
