//! Runs the note detector over a recording against a chart and prints the
//! hit rate. Usage:
//!   ffmpeg -i song.ogg -ac 1 -ar 48000 -f f32le song.f32
//!   cargo run -p riffboard-core --example detect_bench --release -- chart.json song.f32 [offset_s] [level]
//! `chart.json` is the highway messages as a JSON array.

use riffboard_core::chart::Chart;
use riffboard_core::detect::{Detector, Sensitivity};
use riffboard_core::judge::Judge;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let chart = Chart::from_json(&std::fs::read_to_string(&a[1]).expect("chart")).expect("parse");
    // "synth" renders the chart itself as a clean plucked bass (a best case).
    let samples: Vec<f32> = if a[2] == "synth" {
        synth(&chart)
    } else {
        let raw = std::fs::read(&a[2]).expect("audio");
        raw.as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect()
    };
    let offset: f64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let level: f32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.5);
    // Optional: judge against the chart shifted by this many semitones
    // (a "wrong fret" player).
    let shift: i32 = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut judged = chart.clone();
    for t in judged.tuning.iter_mut() {
        *t += shift;
    }
    if judged.tuning.is_empty() {
        judged.tuning = vec![shift; judged.string_count];
    }
    let mut j = Judge::new(&judged);
    let mut d = Detector::new(48000.0, j.events.len());
    d.sensitivity = Sensitivity::from_level(level);
    let t0 = std::time::Instant::now();
    for (k, block) in samples.chunks(1024).enumerate() {
        d.push(block, (k * 1024) as f64 / 48000.0 + offset, &mut j);
    }
    let el = t0.elapsed().as_secs_f64();
    let t = j.tally;
    println!(
        "events {} hits {} misses {} open {} rate {:.1}% | {:.2}s audio in {:.3}s ({:.0}x real time)",
        j.events.len(),
        t.hits,
        t.misses,
        j.events.len() as i64 - t.hits - t.misses,
        100.0 * t.hits as f64 / (t.hits + t.misses).max(1) as f64,
        samples.len() as f64 / 48000.0,
        el,
        samples.len() as f64 / 48000.0 / el
    );
}

fn synth(chart: &Chart) -> Vec<f32> {
    let sr = 48000.0;
    let len = ((chart.last_event_time() + 2.0) * sr) as usize;
    let mut out = vec![0.0f32; len];
    let mut seed: u32 = 1;
    for o in out.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *o = (seed as f32 / u32::MAX as f32 - 0.5) * 0.004;
    }
    let notes = chart
        .notes
        .iter()
        .chain(chart.chords.iter().flat_map(|c| c.notes.iter()));
    for n in notes {
        let f0 = riffboard_core::pitch::freq(chart.midi(n) as f64, 0.0);
        let start = (n.time * sr) as usize;
        let dur = ((n.sustain.max(0.25) + 0.1) * sr) as usize;
        for i in 0..dur.min(len.saturating_sub(start)) {
            let t = i as f64 / sr;
            let env = ((-t * 4.0).exp() * (t * 400.0).min(1.0)) as f32 * 0.25;
            let mut v = 0.0;
            for h in 1..=10 {
                // Weak fundamental, like a bass through a small microphone.
                let w = if h == 1 { 0.3 } else { 1.0 / h as f64 };
                v += (std::f64::consts::TAU * f0 * h as f64 * t).sin() * w;
            }
            out[start + i] += env * v as f32;
        }
    }
    out
}
