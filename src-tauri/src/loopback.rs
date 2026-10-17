//! Browser sign-ins that come back to a loopback address
//! (http://127.0.0.1:<port>/<path>?code=…): Discord (Tunewire accounts) and
//! Spotify (library import, Premium playback).
//!
//! Browsers often open a "warm-up" connection to the redirect address that
//! never sends a request, and may ask for /favicon.ico. Each connection is
//! read with its own short timeout and anything that isn't the callback is
//! answered and skipped; the whole wait has a deadline. (librespot's own
//! helper took the first connection and blocked on it with no time limit,
//! which left the Spotify import "waiting for the browser" forever.)

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use base64::Engine;
use sha2::Digest;

use crate::error::{AppError, AppResult};

/// Bumped by `cancel()`: any sign-in waiting since before then stops.
static CANCEL_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Stop waiting for any browser sign-in (the user closed the dialog).
pub fn cancel() {
    CANCEL_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

pub fn random_b64(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("system randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// A PKCE verifier and its S256 challenge.
pub fn pkce() -> (String, String) {
    let verifier = random_b64(48);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn param(query: &str, k: &str) -> Option<String> {
    query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(|v| percent_decode(&v.replace('+', " ")))
}

/// The callback query → the auth code, or why there is none.
pub fn parse_callback(query: &str) -> Result<String, Option<String>> {
    if let Some(c) = param(query, "code").filter(|c| !c.is_empty()) {
        return Ok(c);
    }
    // The provider explains failures in error_description.
    match param(query, "error_description").or_else(|| param(query, "error")) {
        Some(r) if r != "access_denied" => Err(Some(r)),
        _ => Err(None),
    }
}

/// Wait for the browser to come back to `path` with a code. `state`, when
/// given, must match (a callback from another sign-in is refused).
pub fn wait_for_code(listener: TcpListener, path: &str, state: Option<&str>, product: &str, deadline: Instant) -> AppResult<String> {
    listener.set_nonblocking(true)?;
    let gen = CANCEL_GEN.load(std::sync::atomic::Ordering::SeqCst);
    loop {
        if CANCEL_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen {
            return Err(AppError::Cancelled);
        }
        if Instant::now() > deadline {
            return Err(AppError::Timeout(format!("{product} sign-in timed out")));
        }
        match listener.accept() {
            Ok((mut sock, _)) => {
                let _ = sock.set_nonblocking(false);
                let _ = sock.set_read_timeout(Some(Duration::from_secs(3)));
                let mut buf = [0u8; 8192];
                // A warm-up connection sends nothing: skip it.
                let n = sock.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    continue;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let target = req.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("");
                if !target.starts_with(path) {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
                let mut result = parse_callback(query);
                if let (Ok(_), Some(want)) = (&result, state) {
                    if param(query, "state").as_deref() != Some(want) {
                        result = Err(Some("the sign-in didn't match; please try again".into()));
                    }
                }
                let text = match &result {
                    Ok(_) => format!("You're signed in to {product} in Tunewire. You can close this tab."),
                    Err(Some(r)) => format!("Sign-in failed: {}. You can close this tab.", html_escape(r)),
                    Err(None) => "Sign-in was cancelled. You can close this tab.".to_string(),
                };
                let page = format!("<!doctype html><meta charset=utf-8><title>Tunewire</title><body style=\"font:16px system-ui;background:#111;color:#eee;display:grid;place-items:center;height:90vh\"><p>{text}</p>");
                let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes());
                return match result {
                    Ok(c) => Ok(c),
                    Err(Some(r)) => Err(AppError::Unavailable(format!("{product} sign-in failed: {r}"))),
                    Err(None) => Err(AppError::Cancelled),
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_errors_are_explained() {
        assert_eq!(parse_callback("code=abc-123&state=x"), Ok("abc-123".into()));
        assert_eq!(parse_callback("error=server_error&error_description=Unable+to+exchange%20code"), Err(Some("Unable to exchange code".into())));
        assert_eq!(parse_callback("error=access_denied"), Err(None));
        assert_eq!(parse_callback(""), Err(None));
        assert_eq!(percent_decode("a%2Fb%zz%"), "a/b%zz%");
    }

    /// The bug behind "waiting for the browser" forever: an empty warm-up
    /// connection and a favicon request come first; the real callback after.
    #[test]
    fn skips_warmup_connections_and_other_paths() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            use std::net::TcpStream;
            let _warmup = TcpStream::connect(("127.0.0.1", port)).unwrap(); // never sends anything
            std::thread::sleep(Duration::from_millis(200));
            let mut fav = TcpStream::connect(("127.0.0.1", port)).unwrap();
            fav.write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut real = TcpStream::connect(("127.0.0.1", port)).unwrap();
            real.write_all(b"GET /login?code=good&state=s1 HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut page = String::new();
            let _ = real.read_to_string(&mut page);
            page
        });
        let code = wait_for_code(listener, "/login", Some("s1"), "Spotify", Instant::now() + Duration::from_secs(20)).unwrap();
        assert_eq!(code, "good");
        assert!(browser.join().unwrap().contains("signed in"));
    }

    #[test]
    fn a_callback_from_another_sign_in_is_refused() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"GET /login?code=x&state=other HTTP/1.1\r\n\r\n").unwrap();
            let mut p = String::new();
            let _ = s.read_to_string(&mut p);
        });
        assert!(wait_for_code(listener, "/login", Some("mine"), "Spotify", Instant::now() + Duration::from_secs(10)).is_err());
    }

    #[test]
    fn gives_up_at_the_deadline() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let t0 = Instant::now();
        assert!(matches!(wait_for_code(listener, "/login", None, "Spotify", Instant::now() + Duration::from_millis(300)), Err(AppError::Timeout(_))));
        assert!(t0.elapsed() < Duration::from_secs(3));
    }
}
