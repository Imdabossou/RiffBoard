//! The JavaScript-facing API (wasm-bindgen).

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use riffboard_core::chart::Chart;

use crate::State;
use crate::gl::Renderer;

#[wasm_bindgen]
pub struct Game {
    renderer: Renderer,
    state: Option<State>,
}

#[wasm_bindgen]
impl Game {
    /// Sets up WebGL2 on `canvas`. Fails if the browser has no WebGL2.
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<Game, JsValue> {
        Ok(Game {
            renderer: Renderer::new(canvas)?,
            state: None,
        })
    }

    /// Loads a chart from the highway messages (a JSON array). Returns a JSON
    /// summary: title, artist, duration, judgeable, last_event, arrangements.
    pub fn load(&mut self, messages_json: &str) -> Result<String, JsValue> {
        let chart = Chart::from_json(messages_json).map_err(|e| JsValue::from_str(&e))?;
        let summary = serde_json::json!({
            "title": chart.title,
            "artist": chart.artist,
            "duration": chart.duration,
            "arrangement": chart.arrangement,
            "arrangement_index": chart.arrangement_index,
            "judgeable": chart.judgeable(),
            "last_event": chart.last_event_time(),
            "first_event": chart.notes.first().map(|n| n.time).unwrap_or(0.0)
                .min(chart.chords.first().map(|c| c.time).unwrap_or(f64::MAX)),
            "audio_url": chart.audio_url,
            "stems": chart.stems.iter().map(|s| serde_json::json!({"id": s.id, "url": s.url, "default": s.default})).collect::<Vec<_>>(),
            "sections": chart.sections.iter().map(|s| serde_json::json!({"time": s.time, "name": s.name})).collect::<Vec<_>>(),
        });
        let mut st = State::new(chart);
        st.font = self.renderer.font.clone();
        if let Some(old) = &self.state {
            st.opts = old.opts;
        }
        self.state = Some(st);
        Ok(summary.to_string())
    }

    /// Shows `img` (loaded, same-origin) blurred and darkened behind the highway.
    pub fn set_backdrop(&mut self, img: &web_sys::HtmlImageElement) -> Result<(), JsValue> {
        self.renderer.set_backdrop(img)
    }

    /// Stage light colours (0..1 RGB): the band's accent and a second
    /// colour for the horizon haze.
    pub fn set_accent(&mut self, r: f32, g: f32, b: f32, r2: f32, g2: f32, b2: f32) {
        if let Some(s) = &mut self.state {
            s.opts.accent = [r, g, b];
            s.opts.accent2 = [r2, g2, b2];
        }
    }

    /// Note speed (world units per second; 11 is the default).
    pub fn set_speed(&mut self, speed: f32) {
        if let Some(s) = &mut self.state {
            s.opts.speed = speed.clamp(4.0, 30.0);
        }
    }

    /// Lowest string drawn at the bottom (true) or at the top (false, default).
    pub fn set_low_string_bottom(&mut self, on: bool) {
        if let Some(s) = &mut self.state {
            s.opts.low_string_bottom = on;
        }
    }

    /// Left-handed view (mirrored highway).
    pub fn set_lefty(&mut self, on: bool) {
        if let Some(s) = &mut self.state {
            s.opts.lefty = on;
        }
    }

    /// Starts judging from the microphone at `sample_rate`.
    pub fn enable_mic(&mut self, sample_rate: f64) {
        if let Some(s) = &mut self.state {
            s.enable_mic(sample_rate);
        }
    }

    /// Detection leniency, 0 (strict) to 1 (lenient).
    pub fn set_sensitivity(&mut self, level: f32) {
        if let Some(s) = &mut self.state {
            s.set_sensitivity(level);
        }
    }

    /// Microphone samples; `t_first` is the song time of the first sample.
    pub fn mic(&mut self, samples: &[f32], t_first: f64) {
        if let Some(s) = &mut self.state {
            s.mic(samples, t_first);
        }
    }

    /// Call after a jump in song time (practice only).
    pub fn seek(&mut self, t: f64) {
        if let Some(s) = &mut self.state {
            s.seek(t);
        }
    }

    /// Draws one frame at song time `t`. `css_w`/`css_h` are the canvas' CSS
    /// size; the drawing buffer is kept at that times `dpr`.
    pub fn frame(&mut self, t: f64, css_w: f64, css_h: f64, dpr: f64) {
        let (w, h) = self.renderer.resize(css_w, css_h, dpr);
        let Some(s) = &mut self.state else {
            return;
        };
        let vp = s.build(t, w as f32, h as f32, dpr as f32);
        let speed = s.opts.speed;
        // Fade over the far 40% of the visible stretch.
        let fog = [speed * 3.2 * 0.55, speed * 3.2 * 0.98];
        self.renderer
            .draw(&s.batches, &vp, w, h, fog, (t.rem_euclid(1000.0)) as f32);
    }

    /// Song time to jump to for a click at CSS pixel (x, y) on a canvas of
    /// CSS width `css_w`: the start of the section clicked on the navigator.
    /// NaN when the point isn't on the navigator.
    pub fn nav_seek(&self, x: f64, y: f64, css_w: f64, dpr: f64) -> f64 {
        let Some(s) = &self.state else {
            return f64::NAN;
        };
        let d = dpr as f32;
        crate::scene::nav_seek(
            &s.chart,
            s.end(),
            (css_w * dpr) as f32,
            d,
            x as f32 * d,
            y as f32 * d,
        )
        .unwrap_or(f64::NAN)
    }

    /// [hits, misses, streak, best_streak, score, accuracy].
    pub fn tally(&self) -> Vec<f64> {
        let Some(s) = &self.state else {
            return vec![0.0; 6];
        };
        let t = s.judge.tally;
        vec![
            t.hits as f64,
            t.misses as f64,
            t.streak as f64,
            t.best_streak as f64,
            t.score() as f64,
            t.accuracy(),
        ]
    }

    /// Judges every remaining event as a miss (end of a run).
    pub fn finish(&mut self) {
        if let Some(s) = &mut self.state {
            if s.detector.is_some() {
                for i in 0..s.judge.events.len() {
                    if s.judge.events[i].result.is_none() {
                        s.judge.record(i, false);
                    }
                }
            }
            s.judge.fresh.clear();
        }
    }
}
