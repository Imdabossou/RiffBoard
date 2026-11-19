//! RiffBoard's browser game, compiled to wasm.
//!
//! JavaScript does only what needs browser APIs: it fetches the chart, plays
//! the song through Web Audio, forwards microphone samples, and calls
//! [`Game::frame`] from `requestAnimationFrame` with the song time read from
//! the audio clock. Everything else (layout, drawing, judging, scoring) is here.

pub mod math;
pub mod scene;

#[cfg(target_arch = "wasm32")]
mod gl;
#[cfg(target_arch = "wasm32")]
mod web;

pub use scene::{Batches, Effect, Font, Frame, Hud, Options};

use riffboard_core::chart::Chart;
use riffboard_core::detect::{Detector, Sensitivity};
use riffboard_core::judge::Judge;

/// Platform-independent game state (the wasm wrapper owns one of these).
pub struct State {
    pub chart: Chart,
    pub judge: Judge,
    pub detector: Option<Detector>,
    pub note_res: Vec<Option<bool>>,
    pub chord_res: Vec<Option<bool>>,
    pub effects: Vec<Effect>,
    pub opts: Options,
    pub cam_x: f32,
    last_t: f64,
    pub batches: Batches,
    pub font: Font,
    sensitivity: f32,
}

impl State {
    pub fn new(chart: Chart) -> State {
        let judge = Judge::new(&chart);
        let a = chart.anchor_at(0.0);
        State {
            note_res: vec![None; chart.notes.len()],
            chord_res: vec![None; chart.chords.len()],
            cam_x: scene::anchor_centre(a.fret, a.width),
            judge,
            chart,
            detector: None,
            effects: Vec::with_capacity(64),
            opts: Options::default(),
            last_t: 0.0,
            batches: Batches::default(),
            font: Font::default(),
            sensitivity: 0.5,
        }
    }

    pub fn enable_mic(&mut self, sample_rate: f64) {
        let mut d = Detector::new(sample_rate, self.judge.events.len());
        d.sensitivity = Sensitivity::from_level(self.sensitivity);
        self.detector = Some(d);
    }

    pub fn set_sensitivity(&mut self, level: f32) {
        self.sensitivity = level;
        if let Some(d) = &mut self.detector {
            d.sensitivity = Sensitivity::from_level(level);
        }
    }

    pub fn mic(&mut self, samples: &[f32], t_first: f64) {
        if let Some(d) = &mut self.detector {
            d.push(samples, t_first, &mut self.judge);
        }
        self.absorb();
    }

    /// Records results from the judge into the per-note maps and spawns bursts.
    fn absorb(&mut self) {
        for i in self.judge.fresh.drain(..) {
            let e = &self.judge.events[i];
            let hit = e.result == Some(true);
            let strings = self.chart.string_count;
            let a = self.chart.anchor_at(e.time);
            if e.is_chord {
                self.chord_res[e.index] = e.result;
                if hit {
                    for n in &self.chart.chords[e.index].notes {
                        push_effect(
                            &mut self.effects,
                            e.time,
                            n.fret,
                            n.string,
                            strings,
                            &self.opts,
                            a.fret,
                            a.width,
                        );
                    }
                }
            } else {
                self.note_res[e.index] = e.result;
                if hit {
                    let n = &self.chart.notes[e.index];
                    push_effect(
                        &mut self.effects,
                        e.time,
                        n.fret,
                        n.string,
                        strings,
                        &self.opts,
                        a.fret,
                        a.width,
                    );
                }
            }
        }
    }

    /// After a jump in song time (practice seek or loop): reopen events ahead.
    pub fn seek(&mut self, t: f64) {
        self.judge.reopen_from(t);
        if let Some(d) = &mut self.detector {
            d.reset(&self.judge, t);
        }
        self.judge.fresh.clear();
        let (n, c) = self.judge.result_map(&self.chart);
        self.note_res = n;
        self.chord_res = c;
        self.effects.clear();
        let a = self.chart.anchor_at(t.max(0.0));
        self.cam_x = scene::anchor_centre(a.fret, a.width);
        self.last_t = t;
    }

    /// Advances the camera and builds the frame. Returns the view-projection.
    pub fn build(&mut self, t: f64, width: f32, height: f32, dpr: f32) -> math::Mat4 {
        let dt = (t - self.last_t).clamp(0.0, 0.25) as f32;
        self.last_t = t;
        // Follow the upcoming anchor smoothly (time constant ~0.3 s), looking a
        // little ahead so the window has moved by the time notes arrive.
        let a = self.chart.anchor_at((t + 0.35).max(0.0));
        let target = scene::anchor_centre(a.fret, a.width);
        let k = 1.0 - (-dt / 0.3).exp();
        self.cam_x += (target - self.cam_x) * k;
        if self.effects.len() > 48 {
            self.effects.retain(|e| t - e.t < 0.5);
        }

        let tally = self.judge.tally;
        let section_idx = self.chart.sections.partition_point(|s| s.time <= t);
        let section = if section_idx == 0 {
            ""
        } else {
            self.chart.sections[section_idx - 1].name.as_str()
        };
        let end = self
            .chart
            .duration
            .max(self.chart.last_event_time())
            .max(1.0);
        let frame = Frame {
            chart: &self.chart,
            note_res: &self.note_res,
            chord_res: &self.chord_res,
            t,
            cam_x: self.cam_x,
            width,
            height,
            dpr,
            effects: &self.effects,
            hud: Hud {
                score: tally.score(),
                streak: tally.streak,
                accuracy: tally.accuracy(),
                judged: tally.hits + tally.misses,
                mic: self.detector.as_ref().map(|d| d.level),
                progress: (t / end).clamp(0.0, 1.0) as f32,
                end,
            },
            section,
        };
        scene::build(&frame, &self.opts, &self.font, &mut self.batches)
    }
}

impl State {
    /// Song length the HUD uses (same as in `build`).
    pub fn end(&self) -> f64 {
        self.chart
            .duration
            .max(self.chart.last_event_time())
            .max(1.0)
    }
}

#[allow(clippy::too_many_arguments)]
fn push_effect(
    v: &mut Vec<Effect>,
    t: f64,
    fret: i32,
    string: usize,
    strings: usize,
    o: &Options,
    af: i32,
    aw: i32,
) {
    let x = if fret == 0 {
        scene::anchor_centre(af, aw) - 0.5
    } else {
        scene::note_x(fret)
    };
    v.push(Effect {
        t,
        x,
        y: scene::string_y(string, strings, o),
        string,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use riffboard_core::chart::Note;

    #[test]
    fn seek_reopens_and_camera_follows() {
        let chart = Chart {
            string_count: 4,
            bass: true,
            tuning: vec![0; 4],
            notes: vec![
                Note {
                    time: 1.0,
                    fret: 3,
                    ..Note::default()
                },
                Note {
                    time: 3.0,
                    fret: 7,
                    ..Note::default()
                },
            ],
            anchors: vec![
                riffboard_core::chart::Anchor {
                    time: 0.0,
                    fret: 1,
                    width: 4,
                },
                riffboard_core::chart::Anchor {
                    time: 2.0,
                    fret: 7,
                    width: 4,
                },
            ],
            ..Chart::default()
        };
        let mut s = State::new(chart);
        s.judge.record(0, true);
        s.absorb();
        assert_eq!(s.note_res[0], Some(true));
        assert_eq!(s.effects.len(), 1);
        s.seek(0.5);
        assert_eq!(s.note_res[0], None);
        let x0 = s.cam_x;
        for k in 0..120 {
            s.build(1.5 + k as f64 / 60.0, 1280.0, 720.0, 1.0);
        }
        assert!(s.cam_x > x0 + 4.0, "camera moved to the new anchor");
    }
}
