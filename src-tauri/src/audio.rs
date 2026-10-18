//! Audio engine built on rodio.
//!
//! Layout:
//!   engine thread  — owns the output device and one rodio `Player` per active
//!                    track, executes commands, handles gapless switching and
//!                    crossfades, reports position / end of track.
//!   decode thread  — one per track; pulls samples from the decoder (which may
//!                    block on the network) into a bounded channel, and
//!                    measures loudness for normalization.
//!   FeedSource     — the `rodio::Source` handed to the device. It only does
//!                    non-blocking `try_recv`s, so a slow network can never stall
//!                    the real-time audio callback; it plays silence instead.
//!
//! The next track can be preloaded: its decode thread starts early and fills
//! its channel, so switching is instant (gapless), or overlapped (crossfade).

use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rodio::{ChannelCount, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate, Source};
use serde::Serialize;

use crate::decoder::AudioDecoder;
use crate::eq::{EqParams, Equalizer};
use crate::error::{AppError, AppResult};
use crate::stream::{OpenedStream, Shared};

const CHUNK_FRAMES: usize = 2048;
/// ~3 s of audio at 48 kHz stereo.
const CHANNEL_CHUNKS: usize = 72;
/// Loudness target (RMS) for normalization, about -18 dBFS.
const TARGET_RMS: f32 = 0.125;
/// Seconds of audio used to estimate a track's loudness.
const MEASURE_SECS: f32 = 15.0;

/// Rebuilds the decoder positioned near `t`; returns it with its actual start time.
type Reopen = Arc<dyn Fn(f64) -> AppResult<(AudioDecoder, f64)> + Send + Sync>;

/// A decoder ready to hand to the engine.
pub struct LoadedTrack {
    pub(crate) decoder: AudioDecoder,
    shared: Arc<Shared>,
    duration: Option<f64>,
    reopen: Option<Reopen>,
    pub token: u64,
}

impl LoadedTrack {
    /// Stop the background download (used when a load is superseded).
    pub fn cancel(&self) {
        self.shared.cancel();
    }
}

/// Build a decoder for an opened stream. Blocks while the container header is
/// read, so call it from a blocking context.
pub fn prepare(opened: OpenedStream, fallback_duration: Option<f64>, token: u64) -> AppResult<LoadedTrack> {
    let OpenedStream { reader, shared, hint, hls } = opened;
    let decoder = AudioDecoder::new(reader, hint.as_deref()).inspect_err(|_| shared.cancel())?;
    // HLS (SoundCloud): seeking rebuilds the decoder at the right segment.
    let hls_duration = hls.as_ref().map(|h| h.total_duration).filter(|d| *d > 0.5);
    let reopen: Option<Reopen> = hls.map(|idx| {
        let shared = shared.clone();
        let hint = hint.clone();
        Arc::new(move |t: f64| {
            let (reader, start) = idx.reader_at(&shared, t)?;
            Ok((AudioDecoder::new(reader, hint.as_deref())?, start))
        }) as Reopen
    });
    let duration = decoder.total_duration().map(|d| d.as_secs_f64()).filter(|d| *d > 0.5).or(fallback_duration).or(hls_duration);
    Ok(LoadedTrack { decoder, shared, duration, reopen, token })
}

// ---------------------------------------------------------------------------
// Decode thread <-> FeedSource plumbing
// ---------------------------------------------------------------------------

struct Chunk {
    epoch: u64,
    /// Empty = end of stream.
    samples: Vec<f32>,
}

struct FeedCtl {
    epoch: AtomicU64,
    /// Seek (target seconds, epoch it starts), set by the engine, consumed by
    /// the decoder thread. The engine bumps `epoch` right away, so audio
    /// buffered before the seek is dropped at once instead of playing on
    /// while the decoder repositions (an HLS seek may need the network).
    seek_req: Mutex<Option<(f64, u64)>>,
    stop: AtomicBool,
    /// Samples played since the current epoch began.
    played: AtomicU64,
    /// Position (s) at the start of the current epoch.
    base_pos: Mutex<f64>,
    /// Epoch the FeedSource is actually outputting.
    playing_epoch: AtomicU64,
    underrun: AtomicBool,
    /// The decoder reached the end and every sample has been queued.
    finished: AtomicBool,
    /// Normalization gain (f32 bits) computed by the decode thread.
    gain: AtomicU32,
    normalize: Arc<AtomicBool>,
    /// Samples the output device has pulled (silence included), published in
    /// steps: it stops moving when the device has gone away.
    pulled: AtomicU64,
    channels: u16,
    rate: u32,
}

impl FeedCtl {
    /// Ask the decoder thread to continue from `target` seconds.
    fn request_seek(&self, target: f64) {
        let mut req = self.seek_req.lock().unwrap();
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        *req = Some((target, epoch));
        *self.base_pos.lock().unwrap() = target;
        self.playing_epoch.store(u64::MAX, Ordering::Release);
    }

    fn position(&self) -> f64 {
        let base = *self.base_pos.lock().unwrap();
        if self.playing_epoch.load(Ordering::Acquire) != self.epoch.load(Ordering::Acquire) {
            return base;
        }
        let frames = self.played.load(Ordering::Relaxed) as f64 / self.channels as f64;
        base + frames / self.rate as f64
    }
}

enum SendResult {
    Sent,
    Interrupted,
    Closed,
}

/// Blocking send that gives up early when a seek or stop is requested.
fn send_chunk(tx: &SyncSender<Chunk>, mut c: Chunk, ctl: &FeedCtl) -> SendResult {
    loop {
        match tx.try_send(c) {
            Ok(()) => return SendResult::Sent,
            Err(TrySendError::Full(back)) => {
                if ctl.stop.load(Ordering::Relaxed) || ctl.seek_req.lock().unwrap().is_some() {
                    return SendResult::Interrupted;
                }
                c = back;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(TrySendError::Disconnected(_)) => return SendResult::Closed,
        }
    }
}

/// Running loudness estimate over the first seconds of a track.
struct Loudness {
    sum_sq: f64,
    frames: u64,
    frozen: bool,
}

impl Loudness {
    fn feed(&mut self, samples: &[f32], ch: usize, rate: u32, ctl: &FeedCtl) {
        if self.frozen {
            return;
        }
        for f in samples.chunks_exact(ch) {
            let mid: f32 = f.iter().sum::<f32>() / ch as f32;
            self.sum_sq += (mid * mid) as f64;
        }
        self.frames += (samples.len() / ch) as u64;
        let secs = self.frames as f32 / rate as f32;
        if secs >= 1.0 {
            let rms = (self.sum_sq / self.frames as f64).sqrt() as f32;
            // Quiet intros shouldn't be boosted to the moon: cap the gain.
            let gain = if rms > 1e-4 { (TARGET_RMS / rms).clamp(0.25, 2.5) } else { 1.0 };
            ctl.gain.store(gain.to_bits(), Ordering::Relaxed);
        }
        if secs >= MEASURE_SECS {
            self.frozen = true;
        }
    }
}

fn decode_loop(mut dec: AudioDecoder, reopen: Option<Reopen>, ctl: Arc<FeedCtl>, tx: SyncSender<Chunk>) {
    let ch = ctl.channels as usize;
    let mut finished = false;
    let mut loud = Loudness { sum_sq: 0.0, frames: 0, frozen: false };
    // Epoch the chunks we produce belong to, and where decoding is (seconds).
    let mut epoch = ctl.epoch.load(Ordering::Acquire);
    let mut dec_pos = 0.0f64;
    'outer: loop {
        if ctl.stop.load(Ordering::Relaxed) {
            return;
        }
        let req = ctl.seek_req.lock().unwrap().take();
        if let Some((t, e)) = req {
            let ok = match &reopen {
                Some(reopen) => match reopen(t) {
                    Ok((d, start)) => {
                        dec = d;
                        // Decode and drop audio up to the exact target.
                        let skip = ((t - start).max(0.0) * ctl.rate as f64) as usize * ch;
                        dec.by_ref().take(skip).for_each(drop);
                        true
                    }
                    Err(_) => false,
                },
                None => dec.try_seek(Duration::from_secs_f64(t)).is_ok(),
            };
            epoch = e;
            if ok {
                dec_pos = t;
            } else {
                // The decoder carries on where it was: show that, not the target
                // (unless a newer seek already replaced this one).
                let base = ctl.base_pos.lock();
                if ctl.epoch.load(Ordering::Acquire) == e {
                    *base.unwrap() = dec_pos;
                }
            }
            finished = false;
            ctl.finished.store(false, Ordering::Release);
        }
        if finished {
            // Stay alive so a seek can revive the track until the engine moves on.
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let mut samples = Vec::with_capacity(CHUNK_FRAMES * ch);
        for s in dec.by_ref() {
            samples.push(s);
            if samples.len() >= CHUNK_FRAMES * ch {
                break;
            }
        }
        let end = samples.len() < CHUNK_FRAMES * ch;
        // Keep chunks frame-aligned so channels never get swapped.
        samples.truncate(samples.len() / ch * ch);
        dec_pos += (samples.len() / ch) as f64 / ctl.rate as f64;
        loud.feed(&samples, ch, ctl.rate, &ctl);
        let mut out = Vec::with_capacity(2);
        if !samples.is_empty() {
            out.push(Chunk { epoch, samples });
        }
        if end {
            out.push(Chunk { epoch, samples: Vec::new() });
            finished = true;
        }
        for c in out {
            match send_chunk(&tx, c, &ctl) {
                SendResult::Sent => {}
                SendResult::Interrupted => {
                    finished = false;
                    continue 'outer;
                }
                SendResult::Closed => return,
            }
        }
        if finished {
            ctl.finished.store(true, Ordering::Release);
        }
    }
}

struct FeedSource {
    rx: Receiver<Chunk>,
    cur: Vec<f32>,
    /// Epoch of `cur`: a seek drops the rest of it right away.
    cur_epoch: u64,
    idx: usize,
    out_count: u64,
    ctl: Arc<FeedCtl>,
    channels: ChannelCount,
    rate: SampleRate,
    gain: f32,
    /// Gain target, refreshed once per chunk (not per sample).
    target: f32,
    /// Samples played but not yet published to `ctl.played`.
    pending: u64,
    /// Samples handed to the device but not yet added to `ctl.pulled`.
    pull_pending: u64,
    drained: Arc<AtomicBool>,
    eq: Equalizer,
    eq_params: Arc<EqParams>,
}

impl FeedSource {
    /// Publish played-sample count (cheap: once per ~10 ms of audio).
    #[inline]
    fn flush_played(&mut self) {
        if self.pending > 0 {
            self.ctl.played.fetch_add(self.pending, Ordering::Relaxed);
            self.pending = 0;
        }
    }

    fn refresh_target(&mut self) {
        self.target = if self.ctl.normalize.load(Ordering::Relaxed) { f32::from_bits(self.ctl.gain.load(Ordering::Relaxed)) } else { 1.0 };
        self.eq.refresh(&self.eq_params);
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((a - 0.9) / 0.1).tanh())
    }
}

impl Iterator for FeedSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        self.pull_pending += 1;
        if self.pull_pending >= 4096 {
            self.ctl.pulled.fetch_add(self.pull_pending, Ordering::Relaxed);
            self.pull_pending = 0;
        }
        loop {
            if self.idx < self.cur.len() && self.cur_epoch != self.ctl.epoch.load(Ordering::Relaxed) {
                self.idx = self.cur.len();
            }
            if self.idx < self.cur.len() {
                let mut s = self.cur[self.idx];
                let ch = (self.out_count % self.ctl.channels as u64) as usize;
                self.idx += 1;
                self.out_count += 1;
                s = self.eq.process(s, ch);
                self.pending += 1;
                if self.pending >= 1024 {
                    self.flush_played();
                }
                // Glide towards the target gain (~0.5 s) to avoid audible steps.
                self.gain += (self.target - self.gain) * 0.00005;
                if self.gain != 1.0 || self.eq.active() {
                    s = soft_clip(s * self.gain);
                }
                return Some(s);
            }
            self.flush_played();
            // Finish a partially emitted frame with silence before switching.
            if !self.out_count.is_multiple_of(self.ctl.channels as u64) {
                self.out_count += 1;
                return Some(0.0);
            }
            match self.rx.try_recv() {
                Ok(c) => {
                    let current = self.ctl.epoch.load(Ordering::Acquire);
                    if c.epoch != current {
                        continue; // stale data from before a seek
                    }
                    if self.ctl.playing_epoch.swap(current, Ordering::AcqRel) != current {
                        self.ctl.played.store(0, Ordering::Relaxed);
                        self.pending = 0;
                    }
                    if c.samples.is_empty() {
                        self.drained.store(true, Ordering::Release);
                        return None;
                    }
                    self.ctl.underrun.store(false, Ordering::Relaxed);
                    self.cur = c.samples;
                    self.cur_epoch = c.epoch;
                    self.idx = 0;
                    self.refresh_target();
                }
                Err(TryRecvError::Empty) => {
                    self.ctl.underrun.store(true, Ordering::Relaxed);
                    self.out_count += 1;
                    return Some(0.0);
                }
                Err(TryRecvError::Disconnected) => {
                    self.drained.store(true, Ordering::Release);
                    return None;
                }
            }
        }
    }
}

impl Source for FeedSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineState {
    Idle,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub state: EngineState,
    pub position: f64,
    pub duration: Option<f64>,
    pub buffered: Option<f64>,
    pub stalled: bool,
    pub token: u64,
    /// Token of the preloaded next track, if any.
    pub preloaded: Option<u64>,
}

pub enum EngineEvent {
    Tick,
    Ended {
        token: u64,
    },
    Error {
        token: u64,
        error: AppError,
    },
    /// The engine switched to the preloaded track by itself (gapless/crossfade).
    Advanced {
        token: u64,
    },
    /// The output device stopped taking audio (unplugged, Bluetooth gone):
    /// everything was stopped and the device closed; play `token` again from
    /// `position` to continue on the current default output.
    DeviceLost {
        token: u64,
        position: f64,
    },
}

/// How long the device may take no audio at all while playing before it
/// counts as gone.
const DEVICE_STALL: Duration = Duration::from_secs(3);

enum Cmd {
    Play(Box<LoadedTrack>),
    Preload(Box<LoadedTrack>),
    ClearPreload,
    Pause,
    Resume,
    Stop,
    Volume(f32),
    Seek(f64),
    Config {
        crossfade: f32,
        normalize: bool,
    },
    /// Switch output device (stops playback; the controller restarts it).
    Device(String),
}

#[derive(Clone)]
pub struct AudioHandle {
    tx: mpsc::Sender<Cmd>,
    status: Arc<Mutex<EngineStatus>>,
    eq: Arc<EqParams>,
}

impl AudioHandle {
    pub fn spawn(volume: f32, on_event: impl Fn(EngineEvent) + Send + 'static) -> AudioHandle {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(EngineStatus { state: EngineState::Idle, position: 0.0, duration: None, buffered: None, stalled: false, token: 0, preloaded: None }));
        let st = status.clone();
        let eq = Arc::new(EqParams::default());
        let eq2 = eq.clone();
        std::thread::Builder::new().name("audio-engine".into()).spawn(move || engine(rx, st, volume, eq2, on_event)).expect("spawn audio thread");
        AudioHandle { tx, status, eq }
    }

    pub fn set_eq(&self, enabled: bool, gains: &[f32]) {
        self.eq.set(enabled, gains);
    }

    pub fn set_device(&self, id: &str) {
        let _ = self.tx.send(Cmd::Device(id.to_string()));
    }

    pub fn play(&self, t: LoadedTrack) {
        let _ = self.tx.send(Cmd::Play(Box::new(t)));
    }
    pub fn preload(&self, t: LoadedTrack) {
        let _ = self.tx.send(Cmd::Preload(Box::new(t)));
    }
    pub fn clear_preload(&self) {
        let _ = self.tx.send(Cmd::ClearPreload);
    }
    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn resume(&self) {
        let _ = self.tx.send(Cmd::Resume);
    }
    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }
    pub fn set_volume(&self, v: f32) {
        let _ = self.tx.send(Cmd::Volume(v.clamp(0.0, 1.0)));
    }
    pub fn seek(&self, secs: f64) {
        let _ = self.tx.send(Cmd::Seek(secs.max(0.0)));
    }
    pub fn configure(&self, crossfade: f32, normalize: bool) {
        let _ = self.tx.send(Cmd::Config { crossfade: crossfade.clamp(0.0, 12.0), normalize });
    }
    pub fn status(&self) -> EngineStatus {
        self.status.lock().unwrap().clone()
    }
}

/// A track whose decode thread is running, not necessarily audible yet.
struct Prepared {
    token: u64,
    ctl: Arc<FeedCtl>,
    shared: Arc<Shared>,
    duration: Option<f64>,
    feed: FeedSource,
    drained: Arc<AtomicBool>,
}

fn start_decoding(t: LoadedTrack, normalize: Arc<AtomicBool>, eq_params: Arc<EqParams>) -> Prepared {
    let LoadedTrack { decoder, shared, duration, reopen, token } = t;
    let channels = decoder.channels();
    let rate = decoder.sample_rate();
    let ctl = Arc::new(FeedCtl {
        epoch: AtomicU64::new(0),
        seek_req: Mutex::new(None),
        stop: AtomicBool::new(false),
        played: AtomicU64::new(0),
        base_pos: Mutex::new(0.0),
        playing_epoch: AtomicU64::new(0),
        underrun: AtomicBool::new(false),
        finished: AtomicBool::new(false),
        gain: AtomicU32::new(1.0f32.to_bits()),
        normalize,
        pulled: AtomicU64::new(0),
        channels,
        rate,
    });
    let (ctx, crx) = mpsc::sync_channel(CHANNEL_CHUNKS);
    let c2 = ctl.clone();
    std::thread::Builder::new().name("audio-decode".into()).spawn(move || decode_loop(decoder, reopen, c2, ctx)).expect("spawn decode thread");
    let drained = Arc::new(AtomicBool::new(false));
    let feed = FeedSource {
        rx: crx,
        cur: Vec::new(),
        cur_epoch: 0,
        idx: 0,
        out_count: 0,
        ctl: ctl.clone(),
        channels: NonZero::new(channels).unwrap(),
        rate: NonZero::new(rate).unwrap(),
        gain: 1.0,
        target: 1.0,
        pending: 0,
        pull_pending: 0,
        drained: drained.clone(),
        eq: Equalizer::new(rate, channels),
        eq_params,
    };
    Prepared { token, ctl, shared, duration, feed, drained }
}

struct Active {
    token: u64,
    player: Player,
    ctl: Arc<FeedCtl>,
    shared: Arc<Shared>,
    duration: Option<f64>,
    drained: Arc<AtomicBool>,
    /// Fade multiplier (0..1) applied on top of the user volume.
    fade: f32,
    /// Crossfade-in in progress: (start, seconds).
    fade_in: Option<(Instant, f32)>,
}

impl Active {
    fn shutdown(self) {
        self.ctl.stop.store(true, Ordering::Relaxed);
        self.shared.cancel();
        self.player.stop();
    }
    fn remaining(&self) -> Option<f64> {
        self.duration.map(|d| d - self.ctl.position())
    }
}

fn perceptual(v: f32) -> f32 {
    v * v
}

/// Open the chosen output (cpal device id), falling back to the default.
fn open_device(id: &str) -> AppResult<MixerDeviceSink> {
    use rodio::cpal::traits::HostTrait;
    let chosen = (!id.is_empty()).then(|| id.parse::<rodio::cpal::DeviceId>().ok()).flatten().and_then(|id| rodio::cpal::default_host().device_by_id(&id));
    let opened = match chosen {
        Some(dev) => DeviceSinkBuilder::from_device(dev).and_then(|b| b.open_stream()).or_else(|_| DeviceSinkBuilder::open_default_sink()),
        None => DeviceSinkBuilder::open_default_sink(),
    };
    let mut sink = opened.map_err(|e| AppError::AudioDevice(e.to_string()))?;
    sink.log_on_drop(false);
    Ok(sink)
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// Output devices for the settings picker.
pub fn output_devices() -> Vec<OutputDevice> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    let default = host.default_output_device().and_then(|d| d.id().ok());
    let Ok(devs) = host.output_devices() else {
        return Vec::new();
    };
    // ALSA also lists every plugin PCM (resamplers, "discard all samples",
    // up/downmixers) and the same card several times; keep real outputs once.
    const NOISE: [&str; 6] = ["Plugin", "Discard all samples", "Rate Converter", "Open Sound System", "JACK Audio", "Speex"];
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<OutputDevice> = devs
        .filter_map(|d| {
            let id = d.id().ok()?;
            let name = d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| id.to_string());
            let is_default = Some(&id) == default.as_ref();
            if !is_default && (NOISE.iter().any(|n| name.contains(n)) || id.to_string().contains("usbstream") || !seen.insert(name.clone())) {
                return None;
            }
            Some(OutputDevice { default: is_default, id: id.to_string(), name })
        })
        .collect();
    out.sort_by_key(|d| !d.default);
    out
}

fn activate(p: Prepared, device: &MixerDeviceSink, volume: f32, fade: f32, paused: bool) -> Active {
    let player = Player::connect_new(device.mixer());
    player.set_volume(perceptual(volume) * fade);
    let Prepared { token, ctl, shared, duration, feed, drained } = p;
    player.append(feed);
    if !paused {
        player.play();
    } else {
        player.pause();
    }
    Active { token, player, ctl, shared, duration, drained, fade, fade_in: None }
}

struct Fade {
    out: Active,
    started: Instant,
    secs: f32,
}

fn engine(rx: Receiver<Cmd>, status: Arc<Mutex<EngineStatus>>, mut volume: f32, eq: Arc<EqParams>, on_event: impl Fn(EngineEvent)) {
    let mut device: Option<MixerDeviceSink> = None;
    let mut device_id = String::new();
    let mut cur: Option<Active> = None;
    let mut next: Option<Prepared> = None;
    let mut fading: Vec<Fade> = Vec::new();
    let mut state = EngineState::Idle;
    let mut crossfade = 0.0f32;
    let normalize = Arc::new(AtomicBool::new(true));
    let mut last_tick = Instant::now();
    // (token, samples pulled, since when) for noticing a dead output device.
    let mut watch: (u64, u64, Instant) = (0, 0, Instant::now());

    let shutdown_next = |n: &mut Option<Prepared>| {
        if let Some(p) = n.take() {
            p.ctl.stop.store(true, Ordering::Relaxed);
            p.shared.cancel();
        }
    };

    loop {
        // Tight loop near track ends / during fades, relaxed otherwise.
        let near_end = cur.as_ref().and_then(|c| c.remaining()).map(|r| r < (crossfade as f64 + 3.0)).unwrap_or(false);
        let timeout = if near_end || !fading.is_empty() { 15 } else { 100 };
        let idle = cur.is_none() && fading.is_empty();
        // Nothing playing: sleep until a command arrives.
        let msg = if idle { rx.recv().map_err(|_| RecvTimeoutError::Disconnected) } else { rx.recv_timeout(Duration::from_millis(timeout)) };
        match msg {
            Ok(Cmd::Play(t)) => {
                if let Some(c) = cur.take() {
                    c.shutdown();
                }
                for f in fading.drain(..) {
                    f.out.shutdown();
                }
                shutdown_next(&mut next);
                if device.is_none() {
                    match open_device(&device_id) {
                        Ok(d) => device = Some(d),
                        Err(e) => {
                            t.shared.cancel();
                            state = EngineState::Idle;
                            on_event(EngineEvent::Error { token: t.token, error: e });
                            continue;
                        }
                    }
                }
                let p = start_decoding(*t, normalize.clone(), eq.clone());
                cur = Some(activate(p, device.as_ref().unwrap(), volume, 1.0, false));
                state = EngineState::Playing;
            }
            Ok(Cmd::Preload(t)) => {
                shutdown_next(&mut next);
                next = Some(start_decoding(*t, normalize.clone(), eq.clone()));
            }
            Ok(Cmd::ClearPreload) => shutdown_next(&mut next),
            Ok(Cmd::Pause) => {
                if let Some(c) = &cur {
                    c.player.pause();
                    for f in &fading {
                        f.out.player.pause();
                    }
                    state = EngineState::Paused;
                }
            }
            Ok(Cmd::Resume) => {
                if let Some(c) = &cur {
                    c.player.play();
                    for f in &fading {
                        f.out.player.play();
                    }
                    state = EngineState::Playing;
                }
            }
            Ok(Cmd::Stop) => {
                if let Some(c) = cur.take() {
                    c.shutdown();
                }
                for f in fading.drain(..) {
                    f.out.shutdown();
                }
                shutdown_next(&mut next);
                state = EngineState::Idle;
            }
            Ok(Cmd::Volume(v)) => {
                volume = v;
                if let Some(c) = &cur {
                    c.player.set_volume(perceptual(v) * c.fade);
                }
            }
            Ok(Cmd::Seek(s)) => {
                if let Some(c) = &cur {
                    let target = c.duration.map(|d| s.min((d - 0.5).max(0.0))).unwrap_or(s);
                    c.ctl.request_seek(target);
                }
            }
            Ok(Cmd::Device(id)) => {
                if id != device_id {
                    device_id = id;
                    if let Some(c) = cur.take() {
                        c.shutdown();
                    }
                    for f in fading.drain(..) {
                        f.out.shutdown();
                    }
                    shutdown_next(&mut next);
                    device = None;
                    state = EngineState::Idle;
                }
            }
            Ok(Cmd::Config { crossfade: cf, normalize: n }) => {
                crossfade = cf;
                normalize.store(n, Ordering::Relaxed);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        // Fades in progress.
        let mut i = 0;
        while i < fading.len() {
            let f = &mut fading[i];
            let k = (f.started.elapsed().as_secs_f32() / f.secs.max(0.01)).min(1.0);
            f.out.fade = 1.0 - k;
            f.out.player.set_volume(perceptual(volume) * f.out.fade);
            if k >= 1.0 || f.out.player.empty() {
                fading.remove(i).out.shutdown();
            } else {
                i += 1;
            }
        }
        if let Some(c) = cur.as_mut() {
            if let Some((start, secs)) = c.fade_in {
                c.fade = (start.elapsed().as_secs_f32() / secs.max(0.01)).min(1.0);
                c.player.set_volume(perceptual(volume) * c.fade);
                if c.fade >= 1.0 {
                    c.fade_in = None;
                }
            }
        }

        // Track transitions.
        if let Some(c) = &cur {
            let ended = c.player.empty() || c.drained.load(Ordering::Acquire);
            let start_crossfade = crossfade > 0.0 && state == EngineState::Playing && next.is_some() && c.remaining().map(|r| r <= crossfade as f64 && r > 0.2).unwrap_or(false);
            if (ended || start_crossfade) && next.is_some() {
                let old = cur.take().unwrap();
                let err = old.shared.error();
                let early = old.duration.map(|d| old.ctl.position() < d - 5.0).unwrap_or(false);
                if ended && err.is_some() && early {
                    // Stream died mid-track: report instead of silently skipping.
                    let token = old.token;
                    let pos = old.ctl.position();
                    old.shutdown();
                    on_event(EngineEvent::Error { token, error: AppError::Network(format!("stream interrupted at {}s", pos as u64)) });
                } else {
                    let p = next.take().unwrap();
                    let fade_in = if start_crossfade { 0.0 } else { 1.0 };
                    let mut new = activate(p, device.as_ref().unwrap(), volume, fade_in, false);
                    if start_crossfade {
                        new.fade_in = Some((Instant::now(), crossfade));
                    }
                    let token = new.token;
                    if start_crossfade {
                        fading.push(Fade { out: old, started: Instant::now(), secs: crossfade });
                    } else {
                        old.shutdown();
                    }
                    cur = Some(new);
                    on_event(EngineEvent::Advanced { token });
                }
            } else if ended {
                let old = cur.take().unwrap();
                let token = old.token;
                let pos = old.ctl.position();
                let err = old.shared.error();
                let early = old.duration.map(|d| pos < d - 5.0).unwrap_or(false);
                old.shutdown();
                state = EngineState::Idle;
                match err {
                    Some(e) if early => on_event(EngineEvent::Error { token, error: AppError::Network(format!("stream interrupted at {}s: {e}", pos as u64)) }),
                    _ => on_event(EngineEvent::Ended { token }),
                }
            }
        }

        // A device that stopped pulling audio (unplugged) never errors out by
        // itself: playback would sit "playing" in silence forever.
        match &cur {
            Some(c) if state == EngineState::Playing => {
                let pulled = c.ctl.pulled.load(Ordering::Relaxed);
                if watch.0 != c.token || watch.1 != pulled {
                    watch = (c.token, pulled, Instant::now());
                } else if watch.2.elapsed() >= DEVICE_STALL {
                    let (token, position) = (c.token, c.ctl.position());
                    if let Some(c) = cur.take() {
                        c.shutdown();
                    }
                    for f in fading.drain(..) {
                        f.out.shutdown();
                    }
                    shutdown_next(&mut next);
                    device = None;
                    state = EngineState::Idle;
                    watch = (0, 0, Instant::now());
                    on_event(EngineEvent::DeviceLost { token, position });
                }
            }
            _ => watch = (0, 0, Instant::now()),
        }

        {
            let mut st = status.lock().unwrap();
            st.state = state;
            st.preloaded = next.as_ref().map(|n| n.token);
            match &cur {
                Some(c) => {
                    st.position = c.ctl.position();
                    st.duration = c.duration;
                    st.buffered = c.shared.buffered();
                    st.stalled = state == EngineState::Playing && c.ctl.underrun.load(Ordering::Relaxed);
                    st.token = c.token;
                }
                None => {
                    st.position = 0.0;
                    st.buffered = None;
                    st.stalled = false;
                }
            }
        }
        // ~4 Hz while playing, ~1 Hz otherwise.
        let every = if state == EngineState::Playing { 250 } else { 1000 };
        if last_tick.elapsed() >= Duration::from_millis(every) {
            last_tick = Instant::now();
            on_event(EngineEvent::Tick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::soft_clip;
    #[test]
    #[ignore]
    fn list_devices_live() {
        for d in super::output_devices() {
            println!("{}{} [{}]", d.name, if d.default { " (default)" } else { "" }, d.id);
        }
    }

    /// Audio buffered before a seek must not play after it: the very next
    /// sound is from the new position (each second here has its own level).
    #[test]
    fn seek_drops_buffered_audio_at_once() {
        use super::*;
        let (rate, secs) = (8000u32, 10u32);
        let data_len = rate * secs * 4;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        for x in [
            16u32.to_le_bytes().to_vec(),
            1u16.to_le_bytes().to_vec(),
            2u16.to_le_bytes().to_vec(),
            rate.to_le_bytes().to_vec(),
            (rate * 4).to_le_bytes().to_vec(),
            4u16.to_le_bytes().to_vec(),
            16u16.to_le_bytes().to_vec(),
        ] {
            wav.extend_from_slice(&x);
        }
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..rate * secs {
            let level = ((i / rate + 1) * 1000) as i16;
            wav.extend_from_slice(&level.to_le_bytes());
            wav.extend_from_slice(&level.to_le_bytes());
        }
        let path = std::env::temp_dir().join(format!("tunewire-seek-{}.wav", std::process::id()));
        std::fs::write(&path, &wav).unwrap();
        let t = prepare(crate::stream::open_local(&path).unwrap(), None, 1).unwrap();
        let p = start_decoding(t, Arc::new(AtomicBool::new(false)), Arc::new(EqParams::default()));
        let mut feed = p.feed;
        let second = |s: f32| (s * 32768.0 / 1000.0).round() as i32 - 1;
        let next_sound = |feed: &mut FeedSource| loop {
            match feed.next() {
                Some(s) if s != 0.0 => return s,
                Some(_) => std::thread::sleep(Duration::from_millis(1)),
                None => panic!("ended"),
            }
        };
        std::thread::sleep(Duration::from_millis(300)); // let the channel fill up
        assert_eq!(second(next_sound(&mut feed)), 0);
        for _ in 0..rate {
            feed.next();
        }
        p.ctl.request_seek(7.0);
        assert_eq!(second(next_sound(&mut feed)), 7, "old audio played after the seek");
        assert!((p.ctl.position() - 7.0).abs() < 0.05);
        p.ctl.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn soft_clip_is_bounded_and_transparent() {
        assert_eq!(soft_clip(0.5), 0.5);
        assert!(soft_clip(3.0) <= 1.0 && soft_clip(3.0) > 0.95);
        assert!(soft_clip(-3.0) >= -1.0);
    }
}

#[cfg(test)]
mod live_tests {
    //! Network tests: `cargo test -- --ignored --nocapture`
    use super::*;
    use crate::models::Platform;

    fn decode(url: &str) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let info = rt.block_on(crate::ytdlp::extract_stream(url)).expect("extract");
        println!("{url}: {:?} {:?} {:?}", info.ext, info.acodec, info.protocol);
        let t0 = std::time::Instant::now();
        let opened = crate::stream::open(&info).expect("open");
        let mut t = prepare(opened, info.duration, 1).expect("decoder");
        println!("  ready in {:?}; {} ch @ {} Hz; duration {:?}", t0.elapsed(), t.decoder.channels(), t.decoder.sample_rate(), t.duration);
        let sec = t.decoder.sample_rate() as usize * t.decoder.channels() as usize;
        let audible = t.decoder.by_ref().take(sec * 12).filter(|s| s.abs() > 1e-4).count();
        assert!(audible > sec * 3, "mostly silence");
        let target = t.duration.unwrap_or(60.0) * 0.6;
        let s0 = std::time::Instant::now();
        match t.reopen.clone() {
            Some(r) => {
                let (d, start) = r(target).expect("reopen");
                t.decoder = d;
                let skip = ((target - start) * t.decoder.sample_rate() as f64) as usize * t.decoder.channels() as usize;
                t.decoder.by_ref().take(skip).for_each(drop);
            }
            None => t.decoder.try_seek(Duration::from_secs_f64(target)).expect("seek"),
        }
        let n = t.decoder.by_ref().take(sec * 3).filter(|s| s.abs() > 1e-4).count();
        println!("  seek to {target:.0}s took {:?}, audible samples after: {n}", s0.elapsed());
        assert!(n > sec / 2);
    }

    /// Where the time goes between "play" and the first audio sample.
    #[test]
    #[ignore]
    fn load_timing_breakdown() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for url in ["https://music.youtube.com/watch?v=MvB77oYXlvU", "https://music.youtube.com/watch?v=PvM79DJ2PmM", "https://music.youtube.com/watch?v=NMRhx71bGo4"] {
            let t0 = std::time::Instant::now();
            let info = rt.block_on(crate::ytdlp::extract_stream(url)).expect("extract");
            let t1 = t0.elapsed();
            let opened = crate::stream::open(&info).expect("open");
            let t2 = t0.elapsed();
            let mut t = prepare(opened, info.duration, 1).expect("decoder");
            let t3 = t0.elapsed();
            let first = t.decoder.next();
            let t4 = t0.elapsed();
            println!("{url}\n  extract {t1:?} | open {:?} | probe {:?} | first sample {:?} | total {t4:?} ({:?} {:?}) {first:?}", t2 - t1, t3 - t2, t4 - t3, info.ext, info.protocol);
        }
    }

    #[test]
    #[ignore]
    fn youtube_music_stream() {
        decode("https://music.youtube.com/watch?v=fa5IWHDbftI");
    }

    #[test]
    #[ignore]
    fn soundcloud_hls_stream() {
        decode("https://soundcloud.com/daft-punk-id/daft-punk-one-more-time");
    }

    #[test]
    #[ignore]
    fn youtube_opus_stream() {
        // Force the Opus/WebM format to prove the libopus path works.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut info = rt.block_on(crate::ytdlp::extract_stream_with_format("https://www.youtube.com/watch?v=FGBhQbmPwH8", "bestaudio[acodec=opus]")).expect("extract");
        assert_eq!(info.acodec.as_deref(), Some("opus"));
        info.filesize = None;
        let opened = crate::stream::open(&info).expect("open");
        let mut t = prepare(opened, info.duration, 1).expect("opus decoder");
        let sec = t.decoder.sample_rate() as usize * t.decoder.channels() as usize;
        let audible = t.decoder.by_ref().take(sec * 5).filter(|s| s.abs() > 1e-4).count();
        println!("opus: {} Hz, audible {audible}", t.decoder.sample_rate());
        assert!(audible > sec);
        t.decoder.try_seek(Duration::from_secs(120)).expect("seek");
        assert!(t.decoder.by_ref().take(sec).count() == sec);
    }

    #[test]
    #[ignore]
    fn search_all() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cache = std::sync::Arc::new(crate::search::SearchCache::default());
        let r = rt.block_on(crate::search::search("daft punk one more time".into(), "all".into(), 8, Platform::ALL.to_vec(), cache, true, false)).unwrap();
        for t in &r.tracks {
            let p: Vec<_> = t.versions.iter().map(|v| format!("{:?}", v.platform)).collect();
            println!("{:5.2} {} — {} {:?} [{}]", t.score, t.artist, t.title, t.duration, p.join(","));
        }
    }
}
