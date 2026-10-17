//! Tunewire accounts (Supabase Auth). Sign-in opens the browser (Discord or
//! Google, PKCE) and catches the redirect on a loopback port; guests get an
//! anonymous account. The refresh token lives in the OS keychain; the page
//! asks for a fresh access token and talks to the database/realtime itself.
//!
//! Project URL and anon key are public by design (row level security does
//! the protecting). They're baked in at build time (TUNEWIRE_SUPABASE_URL,
//! TUNEWIRE_SUPABASE_ANON_KEY) or set in settings for development.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;

use crate::error::{AppError, AppResult};
use crate::store::now_ms;

const REDIRECT_PORT: u16 = 5590;
const KEY_REFRESH: &str = "cloud_refresh_token";

#[derive(Debug, Clone, Serialize)]
pub struct CloudConfig {
    pub url: String,
    pub anon_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSession {
    pub access_token: String,
    pub user_id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub anonymous: bool,
    /// ms timestamp.
    pub expires_at: u64,
}

struct Inner {
    session: CloudSession,
    refresh: String,
}

static SESSION: Mutex<Option<Inner>> = Mutex::new(None);
static OVERRIDE: Mutex<Option<CloudConfig>> = Mutex::new(None);
static FALLBACK_FILE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

pub fn init(dir: &std::path::Path, settings_url: &str, settings_key: &str) {
    *FALLBACK_FILE.lock().unwrap() = Some(dir.join("cloud_session"));
    if !settings_url.is_empty() && !settings_key.is_empty() {
        *OVERRIDE.lock().unwrap() = Some(CloudConfig { url: settings_url.trim_end_matches('/').to_string(), anon_key: settings_key.to_string() });
    }
}

pub fn config() -> Option<CloudConfig> {
    if let Some(c) = OVERRIDE.lock().unwrap().clone() {
        return Some(c);
    }
    match (option_env!("TUNEWIRE_SUPABASE_URL"), option_env!("TUNEWIRE_SUPABASE_ANON_KEY")) {
        (Some(u), Some(k)) if !u.is_empty() && !k.is_empty() => Some(CloudConfig { url: u.trim_end_matches('/').to_string(), anon_key: k.to_string() }),
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
            let _ = std::fs::write(p, token);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
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

fn post(path: &str, body: Value, bearer: Option<&str>) -> AppResult<Value> {
    let c = cfg()?;
    let mut req = crate::http::agent()
        .post(&format!("{}{path}", c.url))
        .header("apikey", &c.anon_key)
        .header("Content-Type", "application/json");
    if let Some(b) = bearer {
        req = req.header("Authorization", &format!("Bearer {b}"));
    }
    match req.send(body.to_string()) {
        Ok(mut r) => {
            let text = r.body_mut().read_to_string().map_err(|e| AppError::Network(e.to_string()))?;
            Ok(if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text)? })
        }
        Err(ureq::Error::StatusCode(c)) => Err(if c == 400 || c == 401 || c == 403 { AppError::Unavailable(format!("sign-in failed (HTTP {c})")) } else { AppError::Network(format!("Tunewire server returned HTTP {c}")) }),
        Err(e) => Err(AppError::Network(e.to_string())),
    }
}

fn adopt(v: &Value) -> AppResult<CloudSession> {
    let access = v.get("access_token").and_then(Value::as_str).ok_or_else(|| AppError::Unavailable("sign-in failed".into()))?.to_string();
    let refresh = v.get("refresh_token").and_then(Value::as_str).unwrap_or("").to_string();
    let expires_in = v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);
    let user = v.get("user").cloned().unwrap_or(Value::Null);
    let meta = user.get("user_metadata").cloned().unwrap_or(Value::Null);
    let s = |k: &str| meta.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_string);
    let session = CloudSession {
        access_token: access,
        user_id: user.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
        name: s("full_name").or_else(|| s("name")).or_else(|| s("user_name")).unwrap_or_else(|| "Guest".into()),
        avatar: s("avatar_url"),
        anonymous: user.get("is_anonymous").and_then(Value::as_bool).unwrap_or(false),
        expires_at: now_ms() + expires_in * 1000,
    };
    if !refresh.is_empty() {
        save_refresh(&refresh);
    }
    *SESSION.lock().unwrap() = Some(Inner { session: session.clone(), refresh });
    Ok(session)
}

fn random_b64(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("system randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Wait for the browser to come back to http://127.0.0.1:5590/callback?code=…
fn catch_code(listener: TcpListener, deadline: Instant) -> AppResult<String> {
    listener.set_nonblocking(true)?;
    loop {
        if Instant::now() > deadline {
            return Err(AppError::Timeout("sign-in timed out".into()));
        }
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false)?;
                sock.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut buf = [0u8; 8192];
                let n = sock.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let line = req.lines().next().unwrap_or("");
                let path = line.split_whitespace().nth(1).unwrap_or("");
                if !path.starts_with("/callback") {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    continue;
                }
                let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
                let param = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(|v| v.replace('+', " "));
                let ok = param("code").is_some();
                let page = format!(
                    "<!doctype html><meta charset=utf-8><title>Tunewire</title><body style=\"font:16px system-ui;background:#111;color:#eee;display:grid;place-items:center;height:90vh\"><p>{}</p>",
                    if ok { "You're signed in to Tunewire. You can close this tab." } else { "Sign-in was cancelled. You can close this tab." }
                );
                let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes());
                return match param("code") {
                    Some(c) => Ok(c),
                    None => Err(AppError::Cancelled),
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.into()),
        }
    }
}

/// Sign in with "discord" or "google" in the browser.
pub fn sign_in_blocking(provider: &str) -> AppResult<CloudSession> {
    let c = cfg()?;
    if !matches!(provider, "discord" | "google" | "github") {
        return Err(AppError::Invalid("unknown sign-in provider".into()));
    }
    let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT)).map_err(|e| AppError::Unavailable(format!("sign-in port {REDIRECT_PORT} is busy: {e}")))?;
    let verifier = random_b64(48);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    let redirect = format!("http://127.0.0.1:{REDIRECT_PORT}/callback");
    let url = format!(
        "{}/auth/v1/authorize?provider={provider}&redirect_to={}&code_challenge={challenge}&code_challenge_method=s256",
        c.url,
        crate::search::percent_encode(&redirect)
    );
    open::that_detached(&url).map_err(|e| AppError::Unavailable(format!("couldn't open the browser: {e}")))?;
    let code = catch_code(listener, Instant::now() + Duration::from_secs(300))?;
    let v = post("/auth/v1/token?grant_type=pkce", serde_json::json!({ "auth_code": code, "code_verifier": verifier }), None)?;
    adopt(&v)
}

/// A guest account (no sign-in): enough to join Listen Together or open shares.
pub fn sign_in_guest_blocking() -> AppResult<CloudSession> {
    let v = post("/auth/v1/signup", serde_json::json!({}), None)?;
    adopt(&v)
}

/// A valid session (refreshed when close to expiry), if signed in.
pub fn session_blocking() -> AppResult<Option<CloudSession>> {
    if config().is_none() {
        return Ok(None);
    }
    let current = SESSION.lock().unwrap().as_ref().map(|i| (i.session.clone(), i.refresh.clone()));
    let (refresh, have) = match current {
        Some((s, _)) if s.expires_at > now_ms() + 120_000 => return Ok(Some(s)),
        Some((_, r)) if !r.is_empty() => (r, true),
        _ => match load_refresh() {
            Some(r) => (r, false),
            None => return Ok(None),
        },
    };
    match post("/auth/v1/token?grant_type=refresh_token", serde_json::json!({ "refresh_token": refresh.trim() }), None) {
        Ok(v) => Ok(Some(adopt(&v)?)),
        Err(AppError::Unavailable(_)) => {
            // Refresh token revoked/expired: signed out.
            clear_refresh();
            *SESSION.lock().unwrap() = None;
            Ok(None)
        }
        Err(e) if have => Err(e),
        Err(e) => Err(e),
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

    /// Against the local stack: `supabase start` in the repo (see supabase/).
    #[test]
    #[ignore]
    fn live_guest_session_and_refresh() {
        let url = std::env::var("SB_URL").unwrap();
        let key = std::env::var("SB_ANON").unwrap();
        *OVERRIDE.lock().unwrap() = Some(CloudConfig { url, anon_key: key });
        *FALLBACK_FILE.lock().unwrap() = Some(std::env::temp_dir().join("tw-cloud-test"));
        let s = sign_in_guest_blocking().unwrap();
        assert!(s.anonymous && !s.user_id.is_empty());
        // Force a refresh.
        SESSION.lock().unwrap().as_mut().unwrap().session.expires_at = 0;
        let r = session_blocking().unwrap().unwrap();
        assert_eq!(r.user_id, s.user_id);
        assert_ne!(r.access_token, s.access_token);
        sign_out_blocking();
        assert!(session_blocking().unwrap().is_none());
    }
}
