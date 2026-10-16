//! 10-band graphic equalizer (RBJ peaking biquads, shelves at the ends),
//! applied per sample in the engine's FeedSource.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub const BANDS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

/// Shared settings, written by the UI thread, read once per audio chunk.
pub struct EqParams {
    enabled: AtomicBool,
    gains: [AtomicU32; 10],
    version: AtomicU64,
}

impl Default for EqParams {
    fn default() -> Self {
        EqParams { enabled: AtomicBool::new(false), gains: Default::default(), version: AtomicU64::new(1) }
    }
}

impl EqParams {
    pub fn set(&self, enabled: bool, gains: &[f32]) {
        self.enabled.store(enabled, Ordering::Relaxed);
        for (i, g) in self.gains.iter().enumerate() {
            let v = gains.get(i).copied().unwrap_or(0.0).clamp(-12.0, 12.0);
            g.store(v.to_bits(), Ordering::Relaxed);
        }
        self.version.fetch_add(1, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy, Default)]
struct State {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

enum Kind {
    LowShelf,
    Peak,
    HighShelf,
}

fn design(kind: Kind, freq: f32, gain_db: f32, rate: u32) -> Coeffs {
    let a = 10f32.powf(gain_db / 40.0);
    let w0 = std::f32::consts::TAU * freq / rate as f32;
    let (sin, cos) = w0.sin_cos();
    let (b0, b1, b2, a0, a1, a2) = match kind {
        Kind::Peak => {
            let alpha = sin / (2.0 * 1.41);
            (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
        }
        Kind::LowShelf | Kind::HighShelf => {
            // Shelf slope S = 1.
            let alpha = sin / 2.0 * std::f32::consts::SQRT_2;
            let k = 2.0 * a.sqrt() * alpha;
            if matches!(kind, Kind::LowShelf) {
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - k),
                    (a + 1.0) + (a - 1.0) * cos + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - k,
                )
            } else {
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - k),
                    (a + 1.0) - (a - 1.0) * cos + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - k,
                )
            }
        }
    };
    Coeffs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

/// Per-track filter state (one per FeedSource).
pub struct Equalizer {
    version: u64,
    rate: u32,
    channels: usize,
    filters: Vec<Coeffs>,
    /// `filters.len() * channels` states, band-major per channel.
    state: Vec<State>,
    preamp: f32,
}

impl Equalizer {
    pub fn new(rate: u32, channels: u16) -> Self {
        Equalizer { version: 0, rate, channels: channels.max(1) as usize, filters: Vec::new(), state: Vec::new(), preamp: 1.0 }
    }

    /// Pick up new settings (cheap when unchanged).
    pub fn refresh(&mut self, p: &EqParams) {
        let v = p.version.load(Ordering::Acquire);
        if v == self.version {
            return;
        }
        self.version = v;
        self.filters.clear();
        let mut max_boost = 0f32;
        if p.enabled.load(Ordering::Relaxed) {
            let nyquist = self.rate as f32 / 2.0;
            for (i, f) in BANDS.iter().enumerate() {
                let g = f32::from_bits(p.gains[i].load(Ordering::Relaxed));
                if g.abs() < 0.05 || *f >= nyquist * 0.9 {
                    continue;
                }
                max_boost = max_boost.max(g);
                let kind = match i {
                    0 => Kind::LowShelf,
                    9 => Kind::HighShelf,
                    _ => Kind::Peak,
                };
                self.filters.push(design(kind, *f, g, self.rate));
            }
        }
        self.state = vec![State::default(); self.filters.len() * self.channels];
        // Leave headroom for boosts so they don't just hit the limiter.
        self.preamp = 10f32.powf(-max_boost * 0.6 / 20.0);
    }

    pub fn active(&self) -> bool {
        !self.filters.is_empty()
    }

    #[inline]
    pub fn process(&mut self, x: f32, ch: usize) -> f32 {
        if self.filters.is_empty() {
            return x;
        }
        let mut y = x * self.preamp;
        let n = self.filters.len();
        let base = (ch % self.channels) * n;
        for (i, c) in self.filters.iter().enumerate() {
            let s = &mut self.state[base + i];
            let out = c.b0 * y + c.b1 * s.x1 + c.b2 * s.x2 - c.a1 * s.y1 - c.a2 * s.y2;
            s.x2 = s.x1;
            s.x1 = y;
            s.y2 = s.y1;
            s.y1 = out;
            y = out;
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady-state gain of the chain for a sine at `freq`.
    fn gain_at(eq: &mut Equalizer, freq: f32, rate: u32) -> f32 {
        let n = rate as usize;
        let mut peak = 0f32;
        for i in 0..n {
            let x = (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin() * 0.25;
            let y = eq.process(x, 0);
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        20.0 * (peak / 0.25).log10()
    }

    #[test]
    fn bands_boost_and_cut_where_expected() {
        let p = EqParams::default();
        let mut g = [0.0f32; 10];
        g[5] = 9.0; // +9 dB at 1 kHz
        g[1] = -9.0; // -9 dB at 62 Hz
        p.set(true, &g);
        let mut eq = Equalizer::new(48_000, 1);
        eq.refresh(&p);
        let pre = 20.0 * eq.preamp.log10();
        let at_1k = gain_at(&mut eq, 1000.0, 48_000) - pre;
        let at_62 = gain_at(&mut eq, 62.0, 48_000) - pre;
        let at_8k = gain_at(&mut eq, 8000.0, 48_000) - pre;
        assert!((at_1k - 9.0).abs() < 1.0, "1 kHz {at_1k}");
        assert!((at_62 + 9.0).abs() < 1.5, "62 Hz {at_62}");
        assert!(at_8k.abs() < 1.0, "8 kHz untouched {at_8k}");
        // Disabled = bit-exact passthrough.
        p.set(false, &g);
        eq.refresh(&p);
        assert_eq!(eq.process(0.3, 0), 0.3);
    }
}
