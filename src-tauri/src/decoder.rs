//! Symphonia decoder with an extended codec set: everything symphonia ships
//! (AAC, MP3, FLAC, Vorbis, PCM) plus Opus through libopus, and the MKV/WebM
//! and Ogg containers. This lets YouTube's Opus/WebM formats play.

use std::time::Duration;

use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CodecRegistry, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

use crate::error::{AppError, AppResult};
use crate::stream::StreamReader;

fn codecs() -> &'static CodecRegistry {
    static REG: std::sync::OnceLock<CodecRegistry> = std::sync::OnceLock::new();
    REG.get_or_init(|| {
        let mut reg = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut reg);
        reg.register_all::<symphonia_adapter_libopus::OpusDecoder>();
        reg
    })
}

pub struct AudioDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    spec: SignalSpec,
    buf: Vec<f32>,
    idx: usize,
    /// Reused conversion buffer (re-created only if the format/size changes).
    scratch: Option<SampleBuffer<f32>>,
    total: Option<Duration>,
    /// Samples to drop after an accurate seek (decoder lands before target).
    skip: usize,
    ended: bool,
}

impl AudioDecoder {
    pub fn new(reader: StreamReader, hint: Option<&str>) -> AppResult<AudioDecoder> {
        // MP4: probe as a non-seekable stream (fast start), seekable afterwards.
        let probing = reader.probe_flag();
        let mp4 = matches!(hint, Some("m4a" | "mp4" | "m4b"));
        probing.store(mp4, std::sync::atomic::Ordering::Relaxed);
        let mss = MediaSourceStream::new(Box::new(reader), MediaSourceStreamOptions::default());
        let mut h = Hint::new();
        if let Some(ext) = hint {
            h.with_extension(match ext {
                "webm" | "weba" => "webm",
                "m4a" | "mp4" => "mp4",
                other => other,
            });
        }
        let probed = symphonia::default::get_probe()
            .format(&h, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
            .map_err(|e| AppError::Decode(format!("unrecognized stream ({e}; format {})", hint.unwrap_or("?"))))?;
        probing.store(false, std::sync::atomic::Ordering::Relaxed);
        let format = probed.format;
        let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or_else(|| AppError::Decode("no audio track".into()))?;
        let mut params = track.codec_params.clone();
        // WebM/Matroska leaves Opus channel info in the OpusHead header
        // (byte 9) instead of the codec parameters; the decoder needs it.
        if params.codec == symphonia::core::codecs::CODEC_TYPE_OPUS {
            use symphonia::core::audio::Channels;
            if params.channels.is_none() {
                let n = params.extra_data.as_deref().filter(|d| d.len() > 9 && d.starts_with(b"OpusHead")).map(|d| d[9]).unwrap_or(2);
                params.channels = Some(if n == 1 { Channels::FRONT_CENTRE } else { Channels::FRONT_LEFT | Channels::FRONT_RIGHT });
            }
            params.sample_rate = params.sample_rate.or(Some(48_000));
        }
        let track_id = track.id;
        let decoder = codecs().make(&params, &DecoderOptions::default()).map_err(|e| AppError::Decode(format!("unsupported codec ({e})")))?;
        let total = match (params.n_frames, params.time_base) {
            (Some(n), Some(tb)) => {
                let t = tb.calc_time(n);
                Some(Duration::from_secs_f64(t.seconds as f64 + t.frac))
            }
            (Some(n), None) => params.sample_rate.map(|r| Duration::from_secs_f64(n as f64 / r as f64)),
            _ => None,
        };
        let spec = SignalSpec::new(params.sample_rate.unwrap_or(48_000), params.channels.unwrap_or(symphonia::core::audio::Channels::FRONT_LEFT | symphonia::core::audio::Channels::FRONT_RIGHT));
        let mut d = AudioDecoder { format, decoder, track_id, time_base: params.time_base, spec, buf: Vec::new(), idx: 0, scratch: None, total, skip: 0, ended: false };
        // Decode the first packet so channels/rate reflect the real stream.
        d.refill();
        if d.ended && d.buf.is_empty() {
            return Err(AppError::Decode("stream contains no audio".into()));
        }
        Ok(d)
    }

    pub fn channels(&self) -> u16 {
        self.spec.channels.count().max(1) as u16
    }

    pub fn sample_rate(&self) -> u32 {
        self.spec.rate.max(1)
    }

    pub fn total_duration(&self) -> Option<Duration> {
        self.total
    }

    /// Decode packets until we have samples (or the stream ends).
    fn refill(&mut self) {
        self.buf.clear();
        self.idx = 0;
        let mut errors = 0;
        while !self.ended {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(_) => {
                    self.ended = true;
                    break;
                }
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    let spec = *audio.spec();
                    if audio.frames() == 0 {
                        continue;
                    }
                    let need = audio.capacity() * spec.channels.count();
                    let fits = self.scratch.as_ref().map(|b| b.capacity() >= need && spec == self.spec).unwrap_or(false);
                    if !fits {
                        self.scratch = Some(SampleBuffer::<f32>::new(audio.capacity() as u64, spec));
                    }
                    self.spec = spec;
                    let sb = self.scratch.as_mut().unwrap();
                    sb.copy_interleaved_ref(audio);
                    self.buf.extend_from_slice(sb.samples());
                    if self.skip > 0 {
                        let drop = self.skip.min(self.buf.len());
                        self.buf.drain(..drop);
                        self.skip -= drop;
                        if self.buf.is_empty() {
                            continue;
                        }
                    }
                    return;
                }
                Err(SymError::DecodeError(_)) => {
                    // Corrupt packet: skip it, but give up on a broken stream.
                    errors += 1;
                    if errors > 50 {
                        self.ended = true;
                    }
                }
                Err(_) => {
                    self.ended = true;
                }
            }
        }
    }

    pub fn try_seek(&mut self, pos: Duration) -> AppResult<()> {
        let to = SeekTo::Time { time: Time::from(pos.as_secs_f64()), track_id: Some(self.track_id) };
        let seeked = self.format.seek(SeekMode::Accurate, to).map_err(|e| AppError::Decode(format!("seek failed: {e}")))?;
        self.decoder.reset();
        self.ended = false;
        self.buf.clear();
        self.idx = 0;
        // Accurate seek may land on an earlier packet; drop the difference.
        self.skip = match self.time_base {
            Some(_) if seeked.required_ts > seeked.actual_ts => {
                let frames = seeked.required_ts - seeked.actual_ts;
                frames as usize * self.channels() as usize
            }
            _ => 0,
        };
        Ok(())
    }
}

impl Iterator for AudioDecoder {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        if self.idx >= self.buf.len() {
            if self.ended {
                return None;
            }
            self.refill();
            if self.buf.is_empty() {
                return None;
            }
        }
        let s = self.buf[self.idx];
        self.idx += 1;
        Some(s)
    }
}
