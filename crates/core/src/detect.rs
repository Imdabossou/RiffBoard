//! Microphone note detection against the chart.
//!
//! The detector does not transcribe: it knows which notes are due, so for each
//! event it only asks "is this pitch sounding, freshly, around its time?".
//! Every hop it takes the spectrum of the last `N` samples and, for each event
//! whose window is open, measures the harmonic salience of the expected pitch
//! and of the semitones either side. An event is a hit when, inside its window,
//!
//! 1. the input is clearly above the noise floor,
//! 2. the expected pitch stands out from its neighbours (wrong fret = miss), and
//! 3. it rose compared with just before the window (a fresh attack, so a note
//!    still ringing from earlier does not count).
//!
//! Muted and harmonic notes are judged on 1 and 3 only.

use crate::fft::Fft;
use crate::judge::{EARLY, Judge, LATE};
use crate::pitch::freq;

/// Spectrum size. 4096 at 48 kHz gives 11.7 Hz bins (85 ms frames).
pub const N: usize = 4096;
/// Samples between spectra (~10.7 ms at 48 kHz).
pub const HOP: usize = 512;
/// Seconds before the window during which the "before" level is measured.
const PRE: f64 = 0.12;
const MIN_HZ: f64 = 55.0;
const MAX_HZ: f64 = 4500.0;
const HARMONICS: usize = 16;

#[derive(Clone, Copy, Debug)]
pub struct Sensitivity {
    /// Expected pitch must beat its louder neighbour by this ratio.
    pub clarity: f32,
    /// Input must be this many times the noise floor (energy).
    pub gate: f32,
    /// Salience must rise by this ratio over the pre-window level.
    pub rise: f32,
}

impl Default for Sensitivity {
    fn default() -> Self {
        Sensitivity {
            clarity: 1.35,
            gate: 4.0,
            rise: 1.3,
        }
    }
}

impl Sensitivity {
    /// `level` 0 (strict) ..= 1 (lenient); 0.5 is the default.
    pub fn from_level(level: f32) -> Sensitivity {
        let l = level.clamp(0.0, 1.0);
        let lerp = |strict: f32, lenient: f32| strict + (lenient - strict) * l;
        Sensitivity {
            clarity: lerp(1.65, 1.05),
            gate: lerp(8.0, 2.0),
            rise: lerp(1.6, 1.05),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Acc {
    pre_min: f32,
    pre_seen: bool,
    best: f32,
    /// Per expected pitch (up to 8): passed the clarity test in some frame.
    clear: u8,
    loud: bool,
}

pub struct Detector {
    sample_rate: f64,
    fft: Fft,
    ring: Vec<f32>,
    frame: Vec<f32>,
    mag: Vec<f32>,
    write: usize,
    since_hop: usize,
    filled: usize,
    /// Song time of the newest sample.
    newest: f64,
    floor: f32,
    acc: Vec<Acc>,
    cursor: usize,
    pub sensitivity: Sensitivity,
    /// Last frame's input level relative to the noise floor (for a meter).
    pub level: f32,
}

impl Detector {
    pub fn new(sample_rate: f64, events: usize) -> Detector {
        Detector {
            sample_rate,
            fft: Fft::new(N),
            ring: vec![0.0; N],
            frame: vec![0.0; N],
            mag: vec![0.0; N / 2],
            write: 0,
            since_hop: 0,
            filled: 0,
            newest: f64::NEG_INFINITY,
            floor: 1e-6,
            acc: vec![Acc::default(); events],
            cursor: 0,
            sensitivity: Sensitivity::default(),
            level: 0.0,
        }
    }

    /// Forget accumulated evidence (after a seek). Keeps the noise floor.
    pub fn reset(&mut self, judge: &Judge, t: f64) {
        self.acc.iter_mut().for_each(|a| *a = Acc::default());
        self.cursor = judge.events.partition_point(|e| e.time + LATE < t);
        self.filled = 0;
        self.since_hop = 0;
        self.newest = f64::NEG_INFINITY;
    }

    /// Feed microphone samples. `t_first` is the song time of `samples[0]`
    /// (already corrected for input latency). Judges events as their windows close.
    pub fn push(&mut self, samples: &[f32], t_first: f64, judge: &mut Judge) {
        let dt = 1.0 / self.sample_rate;
        for (k, &x) in samples.iter().enumerate() {
            self.ring[self.write] = x;
            self.write = (self.write + 1) % N;
            self.filled = (self.filled + 1).min(N);
            self.since_hop += 1;
            if self.since_hop >= HOP && self.filled == N {
                self.since_hop = 0;
                let t_new = t_first + k as f64 * dt;
                // Centre of the frame.
                let t = t_new - (N as f64 / 2.0) * dt;
                self.analyse(t, judge);
            }
        }
        if !samples.is_empty() {
            self.newest = t_first + (samples.len() - 1) as f64 * dt;
        }
    }

    fn analyse(&mut self, t: f64, judge: &mut Judge) {
        let (a, b) = self.ring.split_at(self.write);
        self.frame[..b.len()].copy_from_slice(b);
        self.frame[b.len()..].copy_from_slice(a);
        self.fft.magnitudes(&self.frame, &mut self.mag);

        let bin_hz = self.sample_rate / N as f64;
        let lo = (MIN_HZ / bin_hz) as usize;
        let hi = ((MAX_HZ / bin_hz) as usize).min(self.mag.len() - 1);
        let energy: f32 = self.mag[lo..=hi].iter().map(|m| m * m).sum::<f32>() + 1e-12;
        // Noise floor: falls quickly to quiet frames, rises slowly.
        if energy < self.floor {
            self.floor = self.floor * 0.8 + energy * 0.2;
        } else {
            self.floor *= 1.003;
        }
        self.level = energy / self.floor.max(1e-12);
        let loud = self.level >= self.sensitivity.gate;

        // Finalise events whose window has closed.
        while let Some(e) = judge.events.get(self.cursor) {
            if e.time + LATE >= t {
                break;
            }
            if e.result.is_none() {
                let acc = self.acc[self.cursor];
                let hit = self.decide(&acc, e.midis.len(), e.pitched);
                judge.record(self.cursor, hit);
            }
            self.cursor += 1;
        }

        // Accumulate evidence for events whose (pre-)window contains t.
        let sens = self.sensitivity;
        let mut i = self.cursor;
        while let Some(e) = judge.events.get(i) {
            if e.time - EARLY - PRE > t {
                break;
            }
            if e.result.is_none() {
                let total = if e.pitched {
                    e.midis
                        .iter()
                        .map(|&m| self.salience(m as f64))
                        .sum::<f32>()
                } else {
                    energy
                };
                let mut clear = 0u8;
                if t >= e.time - EARLY && loud && e.pitched {
                    for (k, &m) in e.midis.iter().take(8).enumerate() {
                        let m = m as f64;
                        let own = self.salience(m);
                        let nb = self.salience(m - 1.0).max(self.salience(m + 1.0));
                        if own >= sens.clarity * nb {
                            clear |= 1 << k;
                        }
                    }
                }
                let acc = &mut self.acc[i];
                if t < e.time - EARLY {
                    acc.pre_min = if acc.pre_seen {
                        acc.pre_min.min(total)
                    } else {
                        total
                    };
                    acc.pre_seen = true;
                } else {
                    acc.best = acc.best.max(total);
                    acc.loud |= loud;
                    acc.clear |= clear;
                }
            }
            i += 1;
        }
    }

    fn decide(&self, acc: &Acc, pitches: usize, pitched: bool) -> bool {
        let s = self.sensitivity;
        let rose = !acc.pre_seen || acc.best >= s.rise * acc.pre_min;
        if !acc.loud || !rose {
            return false;
        }
        if !pitched {
            return true;
        }
        let need = pitches
            .min(8)
            .div_ceil(3)
            .max(pitches.min(8) * 3 / 5)
            .max(1) as u32;
        acc.clear.count_ones() >= need
    }

    /// Harmonic salience of a (fractional) MIDI pitch in the current spectrum.
    ///
    /// Sums the spectrum at each harmonic (interpolated between bins). A
    /// harmonic counts in proportion to how far apart semitones are there, in
    /// bins: low harmonics of bass notes are too blurred to tell neighbouring
    /// frets apart, higher ones are sharp.
    fn salience(&self, midi: f64) -> f32 {
        let bin_hz = self.sample_rate / N as f64;
        let f0 = freq(midi, 0.0);
        let last = self.mag.len() - 2;
        let mut sum = 0.0f32;
        for h in 1..=HARMONICS {
            let fh = f0 * h as f64;
            if fh < MIN_HZ {
                continue;
            }
            if fh > MAX_HZ {
                break;
            }
            let x = fh / bin_hz;
            let k = x.floor() as usize;
            if k >= last {
                break;
            }
            let fr = (x - k as f64) as f32;
            let m = self.mag[k] * (1.0 - fr) + self.mag[k + 1] * fr;
            let spacing = (fh * 0.0595 / bin_hz) as f32;
            sum += m * (spacing / 2.5).min(1.0);
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Chart, Note};

    const SR: f64 = 48000.0;

    /// A plucked-string-like tone: harmonics with decay, starting at `start`.
    fn render(out: &mut [f32], midi: f64, start: f64, amp: f32) {
        let f0 = freq(midi, 0.0);
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f64 / SR - start;
            if t < 0.0 {
                continue;
            }
            let env = (-t * 3.0).exp() as f32 * amp;
            for h in 1..=8 {
                let ph = std::f64::consts::TAU * f0 * h as f64 * t;
                *o += env * ph.sin() as f32 / h as f32;
            }
        }
    }

    fn noise(out: &mut [f32], amp: f32) {
        let mut s: u32 = 12345;
        for o in out.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *o += (s as f32 / u32::MAX as f32 - 0.5) * amp;
        }
    }

    fn chart(notes: &[(f64, usize, i32)]) -> Chart {
        Chart {
            string_count: 4,
            bass: true,
            tuning: vec![0; 4],
            notes: notes
                .iter()
                .map(|&(time, string, fret)| Note {
                    time,
                    string,
                    fret,
                    ..Note::default()
                })
                .collect(),
            ..Chart::default()
        }
    }

    fn run(chart: &Chart, audio: &[f32]) -> Vec<Option<bool>> {
        let mut j = Judge::new(chart);
        let mut d = Detector::new(SR, j.events.len());
        for (k, block) in audio.chunks(128).enumerate() {
            d.push(block, (k * 128) as f64 / SR, &mut j);
        }
        j.events.iter().map(|e| e.result).collect()
    }

    #[test]
    fn right_note_hits_wrong_fret_misses() {
        // Chart: A1 (E string 5th fret) at 1.0 s, D2 (A string 5th) at 2.0 s,
        // and G2 at 3.0 s. Audio: A1 at 1.0, C#2 (wrong fret) at 2.0, silence at 3.0.
        let c = chart(&[(1.0, 0, 5), (2.0, 1, 5), (3.0, 2, 5)]);
        let mut a = vec![0.0f32; (4.0 * SR) as usize];
        noise(&mut a, 0.002);
        render(&mut a, 33.0, 1.0, 0.3);
        render(&mut a, 37.0, 2.0, 0.3);
        assert_eq!(run(&c, &a), vec![Some(true), Some(false), Some(false)]);
    }

    #[test]
    fn ringing_note_is_not_a_new_hit() {
        // Same pitch twice, but only plucked once.
        let c = chart(&[(1.0, 0, 5), (1.6, 0, 5)]);
        let mut a = vec![0.0f32; (2.5 * SR) as usize];
        noise(&mut a, 0.002);
        render(&mut a, 33.0, 1.0, 0.3);
        assert_eq!(run(&c, &a), vec![Some(true), Some(false)]);
        // Plucked twice: both hit.
        render(&mut a, 33.0, 1.6, 0.3);
        assert_eq!(run(&c, &a), vec![Some(true), Some(true)]);
    }

    #[test]
    fn timing_window() {
        // Played 0.4 s late: outside the window.
        let c = chart(&[(1.0, 1, 3)]);
        let mut a = vec![0.0f32; (2.5 * SR) as usize];
        noise(&mut a, 0.002);
        render(&mut a, 36.0, 1.4, 0.3);
        assert_eq!(run(&c, &a), vec![Some(false)]);
        // 0.08 s early: inside.
        let mut a = vec![0.0f32; (2.5 * SR) as usize];
        noise(&mut a, 0.002);
        render(&mut a, 36.0, 0.92, 0.3);
        assert_eq!(run(&c, &a), vec![Some(true)]);
    }
}
