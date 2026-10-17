//! A long-lived yt-dlp process for stream extraction.
//!
//! Starting yt-dlp costs ~0.3 s of Python startup and imports per song, and
//! every run opens fresh connections to YouTube. When the configured yt-dlp is
//! a Python script or zipapp (Linux/macOS installs, the bundled Linux copy),
//! this module runs one Python process that imports yt_dlp once and serves
//! extraction requests over stdin/stdout (JSON lines), which makes resolving a
//! song ~0.5 s faster. Anything else (e.g. the Windows .exe) keeps using one
//! process per call; so does every error path here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

use crate::error::{classify_ytdlp_stderr, AppError, AppResult};

const SCRIPT: &str = r#"
import sys, json, threading
if len(sys.argv) > 1 and sys.argv[1]:
    sys.path.insert(0, sys.argv[1])
import yt_dlp
lock = threading.Lock()
def out(o):
    with lock:
        sys.stdout.write(json.dumps(o) + "\n")
        sys.stdout.flush()
class Log:
    def __init__(self): self.errors = []
    def debug(self, m): pass
    def info(self, m): pass
    def warning(self, m): pass
    def error(self, m): self.errors.append(m)
# Reuse YoutubeDL instances (per option set): their HTTP connections stay
# open, so a song doesn't pay for fresh TLS handshakes every time.
pools = {}
plock = threading.Lock()
def take(key, opts):
    with plock:
        free = pools.setdefault(key, [])
        if free:
            return free.pop()
    return yt_dlp.YoutubeDL(opts)
def give(key, y):
    with plock:
        free = pools.setdefault(key, [])
        if len(free) < 4:
            free.append(y)
def handle(req):
    log = Log()
    opts = dict(req.get("opts") or {})
    key = json.dumps(opts, sort_keys=True)
    opts.update(quiet=True, no_warnings=True, noprogress=True, logger=log)
    y = take(key, opts)
    y.params["logger"] = log
    try:
        info = y.extract_info(req["url"], download=False)
        out({"id": req["id"], "ok": y.sanitize_info(info)})
    except BaseException as e:
        out({"id": req["id"], "err": "\n".join(log.errors) or ("ERROR: " + str(e))})
    finally:
        give(key, y)
out({"id": 0, "ready": yt_dlp.version.__version__})
for line in sys.stdin:
    try:
        req = json.loads(line)
    except ValueError:
        continue
    threading.Thread(target=handle, args=(req,), daemon=True).start()
"#;

type Pending = HashMap<u64, oneshot::Sender<Result<Value, String>>>;

struct Worker {
    stdin: ChildStdin,
    _child: Child,
}

static WORKER: tokio::sync::Mutex<Option<Worker>> = tokio::sync::Mutex::const_new(None);
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
/// Set once starting the worker failed for this binary: don't keep retrying.
static DISABLED: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Stop the worker (yt-dlp was switched or updated).
pub async fn reset() {
    if let Some(mut w) = WORKER.lock().await.take() {
        let _ = w._child.start_kill();
    }
    DISABLED.store(false, Ordering::SeqCst);
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|f| f.is_file()))
}

/// (python interpreter, extra sys.path entry) for a yt-dlp script or zipapp.
fn python_for(binary: &str) -> Option<(Vec<String>, String)> {
    let path = find_on_path(binary)?;
    let mut head = vec![0u8; 4096];
    let n = {
        use std::io::Read;
        std::fs::File::open(&path).ok()?.read(&mut head).ok()?
    };
    let head = &head[..n];
    if !head.starts_with(b"#!") {
        return None; // native executable (PyInstaller build)
    }
    let line = String::from_utf8_lossy(&head[2..head.iter().position(|&b| b == b'\n')?]).trim().to_string();
    let mut parts: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    if parts.first().map(|p| p.ends_with("/env")).unwrap_or(false) {
        parts.remove(0);
        parts.retain(|p| !p.starts_with('-'));
    }
    if !parts.first().map(|p| p.contains("python")).unwrap_or(false) {
        return None;
    }
    let interp = vec![parts[0].clone()];
    // A zipapp carries the yt_dlp package inside the file itself.
    let is_zip = head.windows(4).any(|w| w == b"PK\x03\x04") || std::fs::read(&path).map(|b| b.windows(4).any(|w| w == b"PK\x03\x04")).unwrap_or(false);
    Some((interp, if is_zip { path.to_string_lossy().into_owned() } else { String::new() }))
}

async fn start(binary: &str) -> Option<Worker> {
    let (interp, syspath) = python_for(binary)?;
    let mut cmd = Command::new(&interp[0]);
    cmd.args(["-c", SCRIPT, &syspath]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let stdin = child.stdin.take()?;
    let mut lines = BufReader::new(child.stdout.take()?).lines();
    // The worker announces itself once yt_dlp is imported.
    let ready = tokio::time::timeout(Duration::from_secs(20), lines.next_line()).await.ok()?.ok()??;
    if !ready.contains("\"ready\"") {
        return None;
    }
    *PENDING.lock().unwrap() = Some(HashMap::new());
    tauri::async_runtime::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let Some(id) = v.get("id").and_then(Value::as_u64) else { continue };
            let tx = PENDING.lock().unwrap().as_mut().and_then(|p| p.remove(&id));
            if let Some(tx) = tx {
                let r = match (v.get("ok"), v.get("err").and_then(Value::as_str)) {
                    (Some(ok), _) => Ok(ok.clone()),
                    (_, Some(err)) => Err(err.to_string()),
                    _ => Err("malformed worker reply".into()),
                };
                let _ = tx.send(r);
            }
        }
        // Worker exited: fail whatever is still waiting (callers fall back).
        if let Some(p) = PENDING.lock().unwrap().take() {
            for (_, tx) in p {
                let _ = tx.send(Err("__worker_gone__".into()));
            }
        }
        if let Ok(mut w) = WORKER.try_lock() {
            *w = None;
        }
    });
    Some(Worker { stdin, _child: child })
}

/// Start the worker ahead of the first song (app launch), if it applies here.
pub async fn warm(binary: String) {
    if DISABLED.load(Ordering::SeqCst) {
        return;
    }
    let mut guard = WORKER.lock().await;
    if guard.is_none() {
        match start(&binary).await {
            Some(w) => *guard = Some(w),
            None => DISABLED.store(true, Ordering::SeqCst),
        }
    }
}

/// `extract_info(url)` with yt-dlp Python options in the long-lived worker.
/// `Ok(None)` = worker not available here; use the command line instead.
pub async fn extract(binary: &str, url: &str, opts: Value) -> AppResult<Option<Value>> {
    if DISABLED.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel();
    {
        let mut guard = WORKER.lock().await;
        if guard.is_none() {
            match start(binary).await {
                Some(w) => *guard = Some(w),
                None => {
                    DISABLED.store(true, Ordering::SeqCst);
                    return Ok(None);
                }
            }
        }
        let Some(p) = PENDING.lock().unwrap().as_mut().map(|p| p.insert(id, tx)) else { return Ok(None) };
        let _ = p;
        let line = serde_json::json!({ "id": id, "url": url, "opts": opts }).to_string() + "\n";
        let w = guard.as_mut().unwrap();
        if w.stdin.write_all(line.as_bytes()).await.is_err() || w.stdin.flush().await.is_err() {
            *guard = None;
            return Ok(None);
        }
    }
    match tokio::time::timeout(Duration::from_secs(60), rx).await {
        Err(_) => {
            if let Some(p) = PENDING.lock().unwrap().as_mut() {
                p.remove(&id);
            }
            Err(AppError::Timeout("yt-dlp took too long to respond".into()))
        }
        Ok(Err(_)) => Ok(None),
        Ok(Ok(Err(e))) if e == "__worker_gone__" => Ok(None),
        Ok(Ok(Err(e))) => Err(classify_ytdlp_stderr(&e)),
        Ok(Ok(Ok(v))) => Ok(Some(v)),
    }
}
