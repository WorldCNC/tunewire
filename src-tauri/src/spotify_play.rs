//! Real Spotify playback through librespot (the open-source Spotify client
//! behind spotifyd/ncspot). Needs a Spotify Premium account: the user logs in
//! once in the browser (OAuth), librespot stores a reusable login blob, and
//! tracks are fetched from Spotify's CDN, decrypted, and fed to our own engine
//! like any other stream (Ogg Vorbis, up to 320 kbps).
//!
//! When not logged in (or the account can't stream) Spotify tracks keep
//! playing through a matched upload, as before.

use std::io::{Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::{Session, SessionConfig, SpotifyId, SpotifyUri};
use librespot_metadata::audio::{AudioFileFormat, AudioItem};
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::stream::{self, OpenedStream};

/// Spotify's Ogg files start with a 167-byte custom header page.
const OGG_HEADER_END: u64 = 0xa7;
pub(crate) const REDIRECT: &str = "http://127.0.0.1:5588/login";

static DIR: OnceLock<PathBuf> = OnceLock::new();
static SESSION: Mutex<Option<Session>> = Mutex::const_new(None);

/// Where librespot keeps its login blob (`credentials.json`).
pub fn init(data_dir: PathBuf) {
    let _ = DIR.set(data_dir.join("spotify"));
}

fn cache() -> AppResult<Cache> {
    let dir = DIR.get().ok_or_else(|| AppError::Extraction("spotify not initialised".into()))?;
    Cache::new(Some(dir), None, None, None).map_err(|e| AppError::Extraction(e.to_string()))
}

fn creds_path() -> Option<PathBuf> {
    DIR.get().map(|d| d.join("credentials.json"))
}

fn restrict_permissions() {
    #[cfg(unix)]
    if let Some(p) = creds_path() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
    }
}

pub fn has_login() -> bool {
    creds_path().map(|p| p.exists()).unwrap_or(false)
}

async fn connect(creds: Credentials) -> AppResult<Session> {
    let session = Session::new(SessionConfig::default(), Some(cache()?));
    session.connect(creds, true).await.map_err(|e| {
        let m = e.to_string();
        let l = m.to_lowercase();
        if l.contains("premium") {
            AppError::Unavailable("Spotify playback needs a Premium account".into())
        } else if l.contains("bad credentials") || l.contains("login failed") {
            AppError::Unavailable("Spotify login expired; log in again in Settings → Spotify".into())
        } else {
            AppError::Network(format!("Spotify connection failed: {m}"))
        }
    })?;
    restrict_permissions();
    // Spotify only serves audio to Premium accounts. The account type arrives
    // in a packet right after login.
    for _ in 0..50 {
        if let Some(kind) = session.get_user_attribute("type") {
            if kind != "premium" {
                session.shutdown();
                if let Some(p) = creds_path() {
                    let _ = std::fs::remove_file(p);
                }
                return Err(AppError::Unavailable(format!("playing Spotify audio needs Spotify Premium (this is a {kind} account)")));
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(session)
}

/// Spotify's sign-in in the browser (PKCE, Spotify's own client id, no
/// secret): returns an access token with `scopes`. Our own loopback wait
/// (see loopback.rs) replaces librespot's, which could hang forever.
pub async fn oauth(scopes: &'static [&'static str]) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || oauth_blocking(scopes)).await.map_err(|e| AppError::Network(e.to_string()))?
}

fn oauth_blocking(scopes: &[&str]) -> AppResult<String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 5588))
        .map_err(|e| AppError::Unavailable(format!("Spotify sign-in needs port 5588, which is busy ({e}). Close whatever uses it (or an older Tunewire window) and try again")))?;
    let client_id = SessionConfig::default().client_id;
    let (verifier, challenge) = crate::loopback::pkce();
    let state = crate::loopback::random_b64(16);
    let enc = crate::search::percent_encode;
    let url = format!(
        "https://accounts.spotify.com/authorize?response_type=code&client_id={client_id}&redirect_uri={}&scope={}&code_challenge_method=S256&code_challenge={challenge}&state={state}",
        enc(REDIRECT),
        enc(&scopes.join(" "))
    );
    open::that_detached(&url).map_err(|e| AppError::Unavailable(format!("couldn't open the browser: {e}")))?;
    let code = crate::loopback::wait_for_code(listener, "/login", Some(&state), "Spotify", std::time::Instant::now() + Duration::from_secs(300))?;
    let form = [("grant_type", "authorization_code"), ("code", code.as_str()), ("redirect_uri", REDIRECT), ("client_id", client_id.as_str()), ("code_verifier", verifier.as_str())]
        .iter()
        .map(|(k, v)| format!("{k}={}", enc(v)))
        .collect::<Vec<_>>()
        .join("&");
    let mut r = crate::http::agent()
        .post("https://accounts.spotify.com/api/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form)
        .map_err(|e| AppError::Network(e.to_string()))?;
    let status = r.status().as_u16();
    let v: serde_json::Value = serde_json::from_str(&r.body_mut().read_to_string().unwrap_or_default()).unwrap_or_default();
    match v.get("access_token").and_then(|t| t.as_str()) {
        Some(t) if status == 200 => Ok(t.to_string()),
        _ => Err(AppError::Unavailable(format!(
            "Spotify sign-in failed: {}",
            v.get("error_description").or_else(|| v.get("error")).and_then(|e| e.as_str()).unwrap_or("no token")
        ))),
    }
}

/// Open the browser for Spotify's login page and connect. Returns the username.
pub async fn login() -> AppResult<String> {
    let token = oauth(&["streaming"]).await?;
    let session = connect(Credentials::with_access_token(token)).await?;
    let user = session.username();
    *SESSION.lock().await = Some(session);
    Ok(user)
}

pub async fn logout() {
    if let Some(s) = SESSION.lock().await.take() {
        s.shutdown();
    }
    if let Some(p) = creds_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// Logged-in username (connects with the stored login if needed).
pub async fn username() -> Option<String> {
    session().await.ok().map(|s| s.username())
}

async fn session() -> AppResult<Session> {
    let mut guard = SESSION.lock().await;
    if let Some(s) = guard.as_ref().filter(|s| !s.is_invalid()) {
        return Ok(s.clone());
    }
    let creds = cache()?.credentials().ok_or_else(|| AppError::Unavailable("not logged in to Spotify".into()))?;
    let s = connect(creds).await?;
    *guard = Some(s.clone());
    Ok(s)
}

/// Open a Spotify track (base62 id) as a stream for our decoder.
pub async fn open(track_id: &str) -> AppResult<OpenedStream> {
    let session = session().await?;
    let id = SpotifyId::from_base62(track_id).map_err(|_| AppError::Invalid("malformed Spotify id".into()))?;
    let uri = SpotifyUri::Track { id };
    let mut item = AudioItem::get_file(&session, uri).await.map_err(|e| AppError::Unavailable(format!("Spotify: {e}")))?;
    if item.files.is_empty() {
        // Region-relinked: the same recording under another id.
        let alts = item.alternatives.take().map(|t| t.0).unwrap_or_default();
        let mut found = None;
        for alt in alts {
            if let Ok(a) = AudioItem::get_file(&session, alt).await {
                if a.availability.is_ok() && !a.files.is_empty() {
                    found = Some(a);
                    break;
                }
            }
        }
        item = found.ok_or_else(|| AppError::Unavailable("this track isn't available on Spotify in your region".into()))?;
    }
    if let Err(e) = &item.availability {
        return Err(AppError::Unavailable(format!("Spotify: {e:?}")));
    }
    let track_id = match &item.track_id {
        SpotifyUri::Track { id } => *id,
        _ => id,
    };
    let (format, file_id, bytes_per_second) = [
        (AudioFileFormat::OGG_VORBIS_320, 40 * 1024),
        (AudioFileFormat::OGG_VORBIS_160, 20 * 1024),
        (AudioFileFormat::OGG_VORBIS_96, 12 * 1024),
        (AudioFileFormat::MP3_320, 40 * 1024),
        (AudioFileFormat::MP3_256, 32 * 1024),
        (AudioFileFormat::MP3_160, 20 * 1024),
    ]
    .into_iter()
    .find_map(|(f, bps)| item.files.get(&f).map(|id| (f, *id, bps)))
    .ok_or_else(|| AppError::Unavailable("no playable Spotify audio format".into()))?;

    let file = AudioFile::open(&session, file_id, bytes_per_second).await.map_err(|e| AppError::Network(format!("Spotify audio: {e}")))?;
    let len = file.get_stream_loader_controller().map_err(|e| AppError::Network(e.to_string()))?.len() as u64;
    let key = session.audio_key().request(track_id, file_id).await.map_err(|e| AppError::Unavailable(format!("Spotify refused the audio key: {e}")))?;
    let mut decrypted = AudioDecrypt::new(Some(key), file);
    let is_ogg = matches!(format, AudioFileFormat::OGG_VORBIS_320 | AudioFileFormat::OGG_VORBIS_160 | AudioFileFormat::OGG_VORBIS_96);
    let base = if is_ogg { OGG_HEADER_END } else { 0 };
    let hint = Some(if is_ogg { "ogg" } else { "mp3" }.to_string());
    tauri::async_runtime::spawn_blocking(move || {
        decrypted.seek(SeekFrom::Start(base))?;
        stream::open_source(decrypted, base, len.saturating_sub(base), hint)
    })
    .await
    .map_err(|e| AppError::Extraction(e.to_string()))?
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn live_connect_rejects_bad_token() {
        let dir = std::env::temp_dir().join(format!("tunewire-sp-{}", std::process::id()));
        super::init(dir.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(super::connect(librespot_core::authentication::Credentials::with_access_token("bogus")));
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());
        assert!(!super::has_login());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Needs a login made in the app (Settings → Spotify).
    #[test]
    #[ignore]
    fn live_play_with_app_login() {
        let data = dirs_data().join("app.tunewire.player");
        super::init(data);
        assert!(super::has_login(), "log in from the app first");
        crate::stream::set_cache_dir(std::env::temp_dir().join("tunewire-sp-stream"));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let t0 = std::time::Instant::now();
        let opened = rt.block_on(super::open("4uLU6hMCjMI75M1A2tKUQC")).unwrap();
        let mut dec = crate::decoder::AudioDecoder::new(opened.reader, opened.hint.as_deref()).unwrap();
        println!("opened in {:?}: {} Hz {} ch, {:?}", t0.elapsed(), dec.sample_rate(), dec.channels(), dec.total_duration());
        let n = dec.sample_rate() as usize * dec.channels() as usize * 3;
        let loud = dec.by_ref().take(n).filter(|s| s.abs() > 0.05).count();
        dec.try_seek(std::time::Duration::from_secs(120)).unwrap();
        let after = dec.by_ref().take(n).filter(|s| s.abs() > 0.05).count();
        println!("loud samples: {loud} at start, {after} after seeking to 2:00");
        assert!(loud > 1000 && after > 1000);
    }

    fn dirs_data() -> std::path::PathBuf {
        std::env::var_os("XDG_DATA_HOME").map(Into::into).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share"))
    }
}
