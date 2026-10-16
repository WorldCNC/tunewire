//! Network/file reader for the decoder.
//!
//! Streams are written to a temporary file (not RAM) by a background thread:
//! ranged chunks for progressive HTTP, segment by segment for HLS. The reader
//! serves whatever ranges are already on disk; if the decoder seeks into a
//! part that hasn't arrived yet, the downloader jumps there with a Range
//! request instead of making the user wait for everything before it.
//! Local files (downloads, local library) use the same reader with no network.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::ytdlp::{validate_url, StreamInfo};

/// First request size; later sequential requests grow up to `MAX_CHUNK`
/// (fewer round trips), and shrink back after a random-access jump.
const CHUNK: u64 = 512 * 1024;
const MAX_CHUNK: u64 = 4 * 1024 * 1024;
const MAX_TRIES: u32 = 4;
const MAX_STREAM_BYTES: u64 = 1024 * 1024 * 1024;

static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where partial streams are kept. Cleared on startup.
pub fn set_cache_dir(dir: PathBuf) {
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let _ = CACHE_DIR.set(dir);
}

fn cache_dir() -> PathBuf {
    CACHE_DIR.get().cloned().unwrap_or_else(|| {
        let d = std::env::temp_dir().join("tunewire-stream-cache");
        let _ = std::fs::create_dir_all(&d);
        d
    })
}

use crate::http::agent;

// ---------------------------------------------------------------------------
// Shared buffer (file + downloaded ranges)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct State {
    /// Sorted, non-overlapping [start, end) byte ranges present in the file.
    ranges: Vec<(u64, u64)>,
    total: Option<u64>,
    done: bool,
    error: Option<String>,
    /// Offset a blocked reader is waiting for (random-access request).
    want: Option<u64>,
}

impl State {
    fn add(&mut self, start: u64, end: u64) {
        if end <= start {
            return;
        }
        self.ranges.push((start, end));
        self.ranges.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(self.ranges.len());
        for (s, e) in self.ranges.drain(..) {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        self.ranges = merged;
    }

    /// End of the contiguous downloaded range containing `pos`.
    fn available_until(&self, pos: u64) -> Option<u64> {
        self.ranges.iter().find(|(s, e)| *s <= pos && pos < *e).map(|r| r.1)
    }

    fn downloaded(&self) -> u64 {
        self.ranges.iter().map(|(s, e)| e - s).sum()
    }

    /// Sequential length (HLS appends from 0).
    fn seq_len(&self) -> u64 {
        self.ranges.first().filter(|r| r.0 == 0).map(|r| r.1).unwrap_or(0)
    }

    /// First missing byte at or after `from` (wrapping to the start).
    fn next_gap(&self, from: u64, total: u64) -> Option<u64> {
        let gap_after = |p: u64| -> Option<u64> {
            let mut p = p;
            for (s, e) in &self.ranges {
                if p < *s {
                    return Some(p);
                }
                if p < *e {
                    p = *e;
                }
            }
            (p < total).then_some(p)
        };
        gap_after(from).or_else(|| gap_after(0))
    }

    fn complete(&self, total: u64) -> bool {
        self.ranges.len() == 1 && self.ranges[0] == (0, total)
    }
}

pub struct Shared {
    state: Mutex<State>,
    cond: Condvar,
    cancel: AtomicBool,
    file: Mutex<File>,
    path: PathBuf,
    temporary: bool,
}

impl Drop for Shared {
    fn drop(&mut self) {
        if self.temporary {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Shared {
    fn temp() -> AppResult<Arc<Shared>> {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = cache_dir().join(format!("{}-{}.part", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path)?;
        Ok(Arc::new(Shared {
            state: Mutex::new(State::default()),
            cond: Condvar::new(),
            cancel: AtomicBool::new(false),
            file: Mutex::new(file),
            path,
            temporary: true,
        }))
    }

    /// A finished local file.
    pub fn local(path: &Path) -> AppResult<Arc<Shared>> {
        let file = File::open(path).map_err(|e| AppError::NotFound(format!("{}: {e}", path.display())))?;
        let len = file.metadata()?.len();
        let mut st = State { total: Some(len), done: true, ..Default::default() };
        st.add(0, len);
        Ok(Arc::new(Shared {
            state: Mutex::new(st),
            cond: Condvar::new(),
            cancel: AtomicBool::new(false),
            file: Mutex::new(file),
            path: path.to_path_buf(),
            temporary: false,
        }))
    }

    fn write_at(&self, offset: u64, data: &[u8]) -> AppResult<()> {
        if offset + data.len() as u64 > MAX_STREAM_BYTES {
            return Err(AppError::Extraction("stream too large".into()));
        }
        {
            let mut f = self.file.lock().unwrap();
            f.seek(SeekFrom::Start(offset))?;
            f.write_all(data)?;
        }
        let mut st = self.state.lock().unwrap();
        st.add(offset, offset + data.len() as u64);
        drop(st);
        self.cond.notify_all();
        Ok(())
    }

    fn finish(&self, error: Option<String>) {
        let mut st = self.state.lock().unwrap();
        st.done = true;
        if error.is_some() {
            st.error = error;
        } else if let Some(t) = st.total {
            if !st.complete(t) {
                st.error = Some("stream ended early".into());
            }
        }
        drop(st);
        self.cond.notify_all();
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Fraction downloaded (0..1) when the total size is known.
    pub fn buffered(&self) -> Option<f64> {
        let st = self.state.lock().unwrap();
        if st.done && st.error.is_none() {
            return Some(1.0);
        }
        st.total.filter(|t| *t > 0).map(|t| st.downloaded() as f64 / t as f64)
    }

    pub fn error(&self) -> Option<String> {
        self.state.lock().unwrap().error.clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cond.notify_all();
    }

    /// Block until the whole stream is on disk, then return the temp file path.
    pub fn wait_complete(&self, timeout: Duration) -> AppResult<PathBuf> {
        let deadline = std::time::Instant::now() + timeout;
        let mut st = self.state.lock().unwrap();
        while !st.done {
            if self.cancelled() {
                return Err(AppError::Cancelled);
            }
            if std::time::Instant::now() > deadline {
                return Err(AppError::Timeout("download took too long".into()));
            }
            st = self.cond.wait_timeout(st, Duration::from_millis(250)).unwrap().0;
        }
        match &st.error {
            Some(e) => Err(AppError::Network(e.clone())),
            None => Ok(self.path.clone()),
        }
    }

    fn take_want(&self) -> Option<u64> {
        self.state.lock().unwrap().want.take()
    }
}

/// `Read + Seek` view over the shared file.
///
/// For HLS seeking the view can splice the init segment onto a later media
/// segment: virtual offsets `>= head` map to physical `offset + jump`.
/// Dropping a reader does not stop the download (a seek may replace the
/// reader); the owner calls `Shared::cancel`.
pub struct StreamReader {
    shared: Arc<Shared>,
    pos: u64,
    head: u64,
    jump: u64,
    random_access: bool,
    seekable: bool,
    byte_len: Option<u64>,
}

impl StreamReader {
    fn new(shared: Arc<Shared>, random_access: bool) -> StreamReader {
        let byte_len = shared.state.lock().unwrap().total;
        StreamReader { shared, pos: 0, head: 0, jump: 0, random_access, seekable: byte_len.is_some(), byte_len }
    }

    fn phys(&self, v: u64) -> u64 {
        if v < self.head {
            v
        } else {
            v + self.jump
        }
    }
}

impl Read for StreamReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let phys = self.phys(self.pos);
        let mut st = self.shared.state.lock().unwrap();
        let until = loop {
            if let Some(until) = st.available_until(phys) {
                break until;
            }
            if let Some(t) = st.total {
                if phys >= t {
                    return Ok(0);
                }
            }
            if st.done {
                return match &st.error {
                    Some(e) if st.ranges.is_empty() => Err(io::Error::other(e.clone())),
                    _ => Ok(0),
                };
            }
            if self.shared.cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            if self.random_access && st.want != Some(phys) {
                st.want = Some(phys);
                self.shared.cond.notify_all();
            }
            st = self.shared.cond.wait_timeout(st, Duration::from_millis(200)).unwrap().0;
        };
        drop(st);
        let mut n = (out.len() as u64).min(until - phys);
        if self.pos < self.head {
            n = n.min(self.head - self.pos);
        }
        let mut f = self.shared.file.lock().unwrap();
        f.seek(SeekFrom::Start(phys))?;
        let got = f.read(&mut out[..n as usize])?;
        self.pos += got as u64;
        Ok(got)
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => {
                // Unknown length (HLS): wait until the download completes.
                let mut st = self.shared.state.lock().unwrap();
                loop {
                    if let Some(t) = st.total {
                        break (t - self.jump) as i64 + d;
                    }
                    if st.done {
                        break (st.seq_len() - self.jump) as i64 + d;
                    }
                    if self.shared.cancelled() {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
                    }
                    st = self.shared.cond.wait_timeout(st, Duration::from_millis(200)).unwrap().0;
                }
            }
        };
        if new < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

impl symphonia::core::io::MediaSource for StreamReader {
    fn is_seekable(&self) -> bool {
        self.seekable
    }
    fn byte_len(&self) -> Option<u64> {
        self.byte_len
    }
}

pub struct OpenedStream {
    pub reader: StreamReader,
    pub shared: Arc<Shared>,
    pub hint: Option<String>,
    pub hls: Option<HlsIndex>,
}

/// Where each HLS media segment starts, in time and in the byte buffer.
#[derive(Clone)]
pub struct HlsIndex {
    init_len: u64,
    starts: Vec<f64>,
    offsets: Arc<Mutex<Vec<Option<u64>>>>,
    pub total_duration: f64,
}

impl HlsIndex {
    /// A reader that presents `init + segments[k..]` where segment k contains
    /// time `t`. Blocks until that segment has started downloading.
    pub fn reader_at(&self, shared: &Arc<Shared>, t: f64) -> AppResult<(StreamReader, f64)> {
        let k = self.starts.iter().rposition(|s| *s <= t).unwrap_or(0);
        let mut st = shared.state.lock().unwrap();
        let offset = loop {
            if let Some(o) = self.offsets.lock().unwrap()[k] {
                break o;
            }
            if st.done || shared.cancelled() {
                return Err(AppError::Extraction("segment not available".into()));
            }
            st = shared.cond.wait_timeout(st, Duration::from_millis(200)).unwrap().0;
        };
        drop(st);
        let mut reader = StreamReader::new(shared.clone(), false);
        reader.head = self.init_len;
        reader.jump = offset - self.init_len;
        reader.seekable = false;
        reader.byte_len = None;
        Ok((reader, self.starts[k]))
    }
}

/// Open a local audio file (downloads, local library).
pub fn open_local(path: &Path) -> AppResult<OpenedStream> {
    let shared = Shared::local(path)?;
    let hint = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    Ok(OpenedStream { reader: StreamReader::new(shared.clone(), false), shared, hint, hls: None })
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn request(url: &str, headers: &HashMap<String, String>, range: Option<(u64, u64)>) -> AppResult<ureq::http::Response<ureq::Body>> {
    let mut req = agent().get(url);
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("accept-encoding") {
            continue;
        }
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some((a, b)) = range {
        req = req.header("Range", format!("bytes={a}-{b}"));
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Timeout(_) => AppError::Timeout("stream server did not respond".into()),
        e => AppError::Network(e.to_string()),
    })?;
    let status = resp.status().as_u16();
    match status {
        200..=299 => Ok(resp),
        403 => Err(AppError::Unavailable("stream URL rejected (403) — it may have expired or be region-locked".into())),
        404 | 410 => Err(AppError::Unavailable(format!("stream not found ({status})"))),
        416 => Err(AppError::Extraction("range not satisfiable".into())),
        s if s >= 500 || s == 429 => Err(AppError::Network(format!("server error {s}"))),
        s => Err(AppError::Extraction(format!("unexpected HTTP status {s}"))),
    }
}

/// Retry transient failures with exponential backoff (300ms, 600ms, 1.2s…).
fn with_retry<T>(shared: Option<&Shared>, mut f: impl FnMut() -> AppResult<T>) -> AppResult<T> {
    let mut delay = Duration::from_millis(300);
    for attempt in 1..=MAX_TRIES {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if e.is_transient() && attempt < MAX_TRIES => {
                if shared.map(|s| s.cancelled()).unwrap_or(false) {
                    return Err(AppError::Cancelled);
                }
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// Stream a response body into the file at `offset`. Stops early (Ok) when a
/// reader asks for a different position, so the downloader can jump there.
fn write_body(resp: ureq::http::Response<ureq::Body>, shared: &Shared, offset: u64, interruptible: bool) -> AppResult<u64> {
    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    loop {
        if shared.cancelled() {
            return Err(AppError::Cancelled);
        }
        if interruptible {
            let st = shared.state.lock().unwrap();
            if let Some(w) = st.want {
                // Only abandon this chunk if the wanted byte isn't coming soon.
                let here = offset + written;
                if w < here || w > here + CHUNK {
                    return Ok(written);
                }
            }
        }
        let n = reader.read(&mut buf).map_err(|e| AppError::Network(e.to_string()))?;
        if n == 0 {
            return Ok(written);
        }
        shared.write_at(offset + written, &buf[..n])?;
        written += n as u64;
    }
}

fn check_content_type(resp: &ureq::http::Response<ureq::Body>) -> AppResult<()> {
    if let Some(ct) = resp.headers().get("content-type").and_then(|v| v.to_str().ok()) {
        let ct = ct.to_lowercase();
        if ct.starts_with("text/html") || ct.starts_with("application/json") {
            return Err(AppError::Extraction(format!("server returned {ct} instead of audio")));
        }
    }
    Ok(())
}

fn total_from_headers(resp: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    let h = resp.headers();
    if resp.status().as_u16() == 206 {
        h.get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.trim().parse().ok())
    } else {
        h.get("content-length").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok())
    }
}

/// Open a stream described by yt-dlp. Blocks until the first chunk/segment has
/// arrived (that doubles as URL validation), then continues in the background.
pub fn open(info: &StreamInfo) -> AppResult<OpenedStream> {
    validate_url(&info.url)?;
    let is_hls = info.protocol.as_deref().map(|p| p.contains("m3u8")).unwrap_or(false) || info.url.contains(".m3u8");
    if is_hls {
        open_hls(info)
    } else {
        open_progressive(&info.url, &info.headers, info.filesize, info.ext.clone())
    }
}

pub fn open_progressive(url: &str, headers: &HashMap<String, String>, filesize: Option<u64>, hint: Option<String>) -> AppResult<OpenedStream> {
    let shared = Shared::temp()?;
    let headers = headers.clone();
    let url = url.to_string();

    let first = with_retry(None, || request(&url, &headers, Some((0, CHUNK - 1))))?;
    check_content_type(&first)?;
    let ranged = first.status().as_u16() == 206;
    let total = total_from_headers(&first).or(filesize);
    shared.state.lock().unwrap().total = total;

    if !ranged || total.is_none() {
        // No random access: stream the whole body sequentially in the background.
        let s2 = shared.clone();
        std::thread::spawn(move || {
            let r = write_body(first, &s2, 0, false);
            if total.is_none() {
                let len = s2.state.lock().unwrap().seq_len();
                s2.state.lock().unwrap().total = Some(len);
            }
            s2.finish(r.err().filter(|e| !matches!(e, AppError::Cancelled)).map(|e| e.to_string()));
        });
        // Give the decoder something to probe.
        wait_for(&shared, 64 * 1024);
        let reader = StreamReader::new(shared.clone(), false);
        return Ok(OpenedStream { reader, shared, hint, hls: None });
    }

    let total = total.unwrap();
    write_body(first, &shared, 0, false)?;
    let s2 = shared.clone();
    std::thread::spawn(move || {
        let result = (|| -> AppResult<()> {
            let mut cursor = 0u64;
            let mut chunk = CHUNK;
            loop {
                if s2.cancelled() {
                    return Err(AppError::Cancelled);
                }
                let start = {
                    let want = s2.take_want();
                    let st = s2.state.lock().unwrap();
                    if st.complete(total) {
                        return Ok(());
                    }
                    match want.filter(|w| st.available_until(*w).is_none()) {
                        Some(w) => {
                            chunk = CHUNK; // the user is seeking: answer fast
                            w
                        }
                        None => match st.next_gap(cursor, total) {
                            Some(g) => g,
                            None => return Ok(()),
                        },
                    }
                };
                // Don't re-download bytes we already have.
                let end = {
                    let st = s2.state.lock().unwrap();
                    let next_have = st.ranges.iter().map(|r| r.0).filter(|s| *s > start).min().unwrap_or(total);
                    (start + chunk).min(next_have).min(total) - 1
                };
                let got = with_retry(Some(&s2), || {
                    let resp = request(&url, &headers, Some((start, end)))?;
                    write_body(resp, &s2, start, true)
                });
                match got {
                    Ok(n) => {
                        cursor = start + n;
                        chunk = (chunk * 2).min(MAX_CHUNK);
                    }
                    Err(AppError::Extraction(m)) if m.contains("range not satisfiable") => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
        })();
        s2.finish(result.err().filter(|e| !matches!(e, AppError::Cancelled)).map(|e| e.to_string()));
    });

    let reader = StreamReader::new(shared.clone(), true);
    Ok(OpenedStream { reader, shared, hint, hls: None })
}

fn wait_for(shared: &Shared, bytes: u64) {
    let mut st = shared.state.lock().unwrap();
    while st.seq_len() < bytes && !st.done && !shared.cancelled() {
        st = shared.cond.wait_timeout(st, Duration::from_millis(100)).unwrap().0;
    }
}

fn resolve(base: &str, uri: &str) -> String {
    let uri = uri.trim();
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    let no_query = base.split('?').next().unwrap_or(base);
    if let Some(stripped) = uri.strip_prefix('/') {
        let scheme_end = no_query.find("://").map(|i| i + 3).unwrap_or(0);
        let host_end = no_query[scheme_end..].find('/').map(|i| i + scheme_end).unwrap_or(no_query.len());
        return format!("{}/{}", &no_query[..host_end], stripped);
    }
    let dir = &no_query[..no_query.rfind('/').map(|i| i + 1).unwrap_or(0)];
    format!("{dir}{uri}")
}

fn attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("{key}=");
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    if let Some(r) = rest.strip_prefix('"') {
        r.find('"').map(|e| &r[..e])
    } else {
        Some(rest.split(',').next().unwrap_or(rest))
    }
}

fn fetch_text(url: &str, headers: &HashMap<String, String>) -> AppResult<String> {
    with_retry(None, || {
        let mut resp = request(url, headers, None)?;
        resp.body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| AppError::Network(e.to_string()))
    })
}

/// Fetch one HLS segment and append it; a retry never leaves a partial copy.
fn fetch_segment(s: &Shared, u: &str, headers: &HashMap<String, String>) -> AppResult<u64> {
    let mark = s.state.lock().unwrap().seq_len();
    with_retry(Some(s), || {
        let resp = request(u, headers, None)?;
        check_content_type(&resp)?;
        // Restart at `mark` on retry; later bytes are simply overwritten.
        let mut st = s.state.lock().unwrap();
        st.ranges.retain(|r| r.0 < mark);
        if let Some(r) = st.ranges.last_mut() {
            r.1 = r.1.min(mark);
        }
        drop(st);
        write_body(resp, s, mark, false)
    })
}

fn open_hls(info: &StreamInfo) -> AppResult<OpenedStream> {
    let headers = info.headers.clone();
    let mut url = info.url.clone();
    let mut text = fetch_text(&url, &headers)?;
    if !text.starts_with("#EXTM3U") {
        return Err(AppError::Extraction("invalid HLS playlist".into()));
    }
    // Master playlist → pick the highest-bandwidth variant.
    if text.contains("#EXT-X-STREAM-INF") {
        let mut best: Option<(u64, String)> = None;
        let mut lines = text.lines().peekable();
        while let Some(l) = lines.next() {
            if l.starts_with("#EXT-X-STREAM-INF") {
                let bw = attr(l, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0);
                if let Some(next) = lines.peek() {
                    if !next.starts_with('#') && best.as_ref().map(|(b, _)| bw > *b).unwrap_or(true) {
                        best = Some((bw, resolve(&url, next)));
                    }
                }
            }
        }
        url = best.ok_or_else(|| AppError::Extraction("empty HLS master playlist".into()))?.1;
        text = fetch_text(&url, &headers)?;
    }

    let mut init = None;
    let mut segments = Vec::new();
    let mut starts = Vec::new();
    let (mut t, mut next_dur) = (0.0f64, 0.0f64);
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with("#EXT-X-KEY") && !l.contains("METHOD=NONE") {
            return Err(AppError::Drm("encrypted HLS stream".into()));
        } else if l.starts_with("#EXT-X-MAP") {
            init = attr(l, "URI").map(|u| resolve(&url, u));
        } else if let Some(d) = l.strip_prefix("#EXTINF:") {
            next_dur = d.split(',').next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
        } else if !l.is_empty() && !l.starts_with('#') {
            segments.push(resolve(&url, l));
            starts.push(t);
            t += next_dur;
            next_dur = 0.0;
        }
    }
    if segments.is_empty() {
        return Err(AppError::Extraction("HLS playlist has no segments".into()));
    }

    let shared = Shared::temp()?;
    if let Some(i) = &init {
        fetch_segment(&shared, i, &headers)?;
    }
    let init_len = shared.state.lock().unwrap().seq_len();
    let offsets = Arc::new(Mutex::new(vec![None; segments.len()]));
    offsets.lock().unwrap()[0] = Some(init_len);
    fetch_segment(&shared, &segments[0], &headers)?;

    let s2 = shared.clone();
    let rest: Vec<String> = segments[1..].to_vec();
    let offs = offsets.clone();
    std::thread::spawn(move || {
        let mut err = None;
        for (i, seg) in rest.into_iter().enumerate() {
            if s2.cancelled() {
                break;
            }
            let start = s2.state.lock().unwrap().seq_len();
            offs.lock().unwrap()[i + 1] = Some(start);
            s2.cond.notify_all();
            if let Err(e) = fetch_segment(&s2, &seg, &headers) {
                if !matches!(e, AppError::Cancelled) {
                    err = Some(e.to_string());
                }
                break;
            }
        }
        s2.finish(err);
    });

    let mut reader = StreamReader::new(shared.clone(), false);
    reader.seekable = false;
    Ok(OpenedStream {
        reader,
        shared,
        hint: info.ext.clone().or(Some("mp4".into())),
        hls: Some(HlsIndex { init_len, starts, offsets, total_duration: t }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves() {
        assert_eq!(resolve("https://a.com/x/y/p.m3u8?t=1", "s1.m4s"), "https://a.com/x/y/s1.m4s");
        assert_eq!(resolve("https://a.com/x/y/p.m3u8", "/z/s1.m4s"), "https://a.com/z/s1.m4s");
        assert_eq!(resolve("https://a.com/p.m3u8", "https://b.com/s"), "https://b.com/s");
    }

    #[test]
    fn ranges_merge_and_gaps() {
        let mut st = State::default();
        st.add(0, 10);
        st.add(20, 30);
        st.add(10, 15);
        assert_eq!(st.ranges, vec![(0, 15), (20, 30)]);
        assert_eq!(st.available_until(5), Some(15));
        assert_eq!(st.available_until(16), None);
        assert_eq!(st.next_gap(0, 40), Some(15));
        assert_eq!(st.next_gap(25, 40), Some(30));
        assert_eq!(st.next_gap(35, 35), Some(15)); // wraps
        st.add(15, 20);
        st.add(30, 40);
        assert!(st.complete(40));
    }
}
