//! Builds one frame of the highway as instance lists for the renderer.
//!
//! Pure Rust (no browser APIs), so the layout is unit-tested natively. Every
//! list is cleared and refilled in place each frame: no allocation once the
//! vectors have grown to their working size.

use std::cell::Cell;

use riffboard_core::chart::{Chart, Note};

use crate::math::{self, Mat4};

/// Floats per box instance: centre(3) size(3) colour(4) glow(1) style(1).
pub const BOX_FLOATS: usize = 12;
/// Box styles, shaded differently in gl.rs.
pub const STYLE_MATTE: f32 = 0.0;
pub const STYLE_GEM: f32 = 1.0;
pub const STYLE_RIBBON: f32 = 2.0;
/// Floats per glyph instance: centre(3) size(2) uv(4) colour(4) mode(1).
pub const GLYPH_FLOATS: usize = 14;

pub const ATLAS_COLS: usize = 16;
pub const ATLAS_ROWS: usize = 7;
/// Atlas cell holding a solid white square (row 6, col 0).
pub const CELL_SOLID: usize = 96;
/// Atlas cell holding a soft round glow (row 6, col 1).
pub const CELL_GLOW: usize = 97;
/// Atlas cell holding a ring (row 6, col 2).
pub const CELL_RING: usize = 98;

/// String colours, lowest string first (the usual tab colour order).
pub const STRING_RGB: [[f32; 3]; 8] = [
    [0.90, 0.20, 0.22],
    [0.93, 0.79, 0.22],
    [0.18, 0.56, 0.92],
    [0.95, 0.52, 0.16],
    [0.30, 0.78, 0.24],
    [0.70, 0.30, 0.86],
    [0.98, 0.45, 0.80],
    [0.45, 0.95, 0.88],
];

pub const BG: [f32; 3] = [0.035, 0.04, 0.06];

/// RiffBoard's own stage colours (from its icon), for unbranded sites.
pub const RIFFBOARD_CYAN: [f32; 3] = [0.13, 0.83, 0.93];
pub const RIFFBOARD_VIOLET: [f32; 3] = [0.49, 0.23, 0.93];

const FRETS: i32 = 24;
const STRING_GAP: f32 = 0.42;
const STRING_BASE: f32 = 0.32;
/// Seconds of chart shown ahead of the hit line.
const VIEW_S: f64 = 3.2;
/// Seconds a note stays drawn after passing the hit line.
const TAIL_S: f64 = 0.35;
/// How far back to look for sustains still ringing.
const SUSTAIN_LOOKBACK_S: f64 = 12.0;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// World units per second of chart.
    pub speed: f32,
    /// Draw the lowest string at the bottom instead of the top.
    pub low_string_bottom: bool,
    /// Left-handed: mirror the highway (frets run right to left).
    pub lefty: bool,
    /// Stage light colour (the band's accent, or RiffBoard's), and a second
    /// colour for the haze on the horizon.
    pub accent: [f32; 3],
    pub accent2: [f32; 3],
}

impl Default for Options {
    fn default() -> Self {
        Options {
            speed: 11.0,
            low_string_bottom: false,
            lefty: false,
            accent: RIFFBOARD_CYAN,
            accent2: RIFFBOARD_VIOLET,
        }
    }
}

/// Glyph advance widths, as a fraction of the cell height, for ASCII 32..128.
#[derive(Clone, Debug)]
pub struct Font {
    pub adv: [f32; 96],
}

impl Default for Font {
    fn default() -> Self {
        Font { adv: [0.6; 96] }
    }
}

/// A hit burst at the hit line.
#[derive(Clone, Copy, Debug)]
pub struct Effect {
    pub t: f64,
    pub x: f32,
    pub y: f32,
    pub string: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Hud {
    pub score: i64,
    pub streak: i64,
    pub accuracy: f64,
    pub judged: i64,
    /// Mic level relative to the gate (None = mic off).
    pub mic: Option<f32>,
    pub progress: f32,
    /// Song length used for the progress bar and section navigator.
    pub end: f64,
}

#[derive(Default)]
pub struct Batches {
    pub opaque: Vec<f32>,
    pub clear: Vec<f32>,
    /// Additive glow sprites (glyph instances), drawn over the boxes.
    pub glow: Vec<f32>,
    pub text: Vec<f32>,
    pub hud: Vec<f32>,
}

impl Batches {
    /// Room for a busy frame up front, so playing never grows these.
    pub fn reserved() -> Batches {
        Batches {
            opaque: Vec::with_capacity(2048 * BOX_FLOATS),
            clear: Vec::with_capacity(2048 * BOX_FLOATS),
            glow: Vec::with_capacity(1024 * GLYPH_FLOATS),
            text: Vec::with_capacity(2048 * GLYPH_FLOATS),
            hud: Vec::with_capacity(512 * GLYPH_FLOATS),
        }
    }

    fn clear_all(&mut self) {
        self.opaque.clear();
        self.clear.clear();
        self.glow.clear();
        self.text.clear();
        self.hud.clear();
    }
}

pub struct Frame<'a> {
    pub chart: &'a Chart,
    pub note_res: &'a [Option<bool>],
    pub chord_res: &'a [Option<bool>],
    pub t: f64,
    pub cam_x: f32,
    pub width: f32,
    pub height: f32,
    pub dpr: f32,
    pub effects: &'a [Effect],
    pub hud: Hud,
    pub section: &'a str,
    /// Notes per second of each section, scaled 0..1 (for the navigator).
    pub density: &'a [f32],
}

pub fn fret_x(fret: f32) -> f32 {
    fret
}

/// Centre of the playing position for a fretted note.
pub fn note_x(fret: i32) -> f32 {
    fret_x(fret as f32 - 0.5)
}

pub fn string_y(s: usize, strings: usize, opts: &Options) -> f32 {
    let row = if opts.low_string_bottom {
        s
    } else {
        strings.saturating_sub(1).saturating_sub(s)
    };
    STRING_BASE + row as f32 * STRING_GAP
}

/// Where the camera looks, horizontally, for an anchor (fret window).
pub fn anchor_centre(fret: i32, width: i32) -> f32 {
    let lo = (fret - 1).max(0) as f32;
    let hi = (fret - 1 + width.max(4)) as f32;
    (lo + hi) / 2.0 + 0.5
}

fn push_box(v: &mut Vec<f32>, c: [f32; 3], s: [f32; 3], col: [f32; 4], glow: f32) {
    push_styled(v, c, s, col, glow, STYLE_MATTE);
}

fn push_styled(v: &mut Vec<f32>, c: [f32; 3], s: [f32; 3], col: [f32; 4], glow: f32, style: f32) {
    v.extend_from_slice(&[
        c[0], c[1], c[2], s[0], s[1], s[2], col[0], col[1], col[2], col[3], glow, style,
    ]);
}

/// A soft additive glow, standing upright in the world (or, with `haze`,
/// not faded by distance).
fn push_glow(v: &mut Vec<f32>, c: [f32; 3], size: [f32; 2], col: [f32; 3], a: f32, haze: bool) {
    let uv = cell_uv(CELL_GLOW);
    v.extend_from_slice(&[
        c[0],
        c[1],
        c[2],
        size[0],
        size[1],
        uv[0],
        uv[1],
        uv[2],
        uv[3],
        col[0],
        col[1],
        col[2],
        a,
        if haze { 2.0 } else { 0.0 },
    ]);
}

fn mix3(a: [f32; 3], b: [f32; 3], k: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * k,
        a[1] + (b[1] - a[1]) * k,
        a[2] + (b[2] - a[2]) * k,
    ]
}

fn add3(a: [f32; 3], b: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}

fn cell_uv(cell: usize) -> [f32; 4] {
    let (cx, cy) = ((cell % ATLAS_COLS) as f32, (cell / ATLAS_COLS) as f32);
    let (w, h) = (1.0 / ATLAS_COLS as f32, 1.0 / ATLAS_ROWS as f32);
    // Half-texel-ish inset to avoid bleeding from neighbours.
    let e = 0.002;
    [
        cx * w + e,
        cy * h + e,
        (cx + 1.0) * w - e,
        (cy + 1.0) * h - e,
    ]
}

fn push_glyph(
    v: &mut Vec<f32>,
    c: [f32; 3],
    size: [f32; 2],
    cell: usize,
    col: [f32; 4],
    hud: bool,
) {
    let uv = cell_uv(cell);
    v.extend_from_slice(&[
        c[0],
        c[1],
        c[2],
        size[0],
        size[1],
        uv[0],
        uv[1],
        uv[2],
        uv[3],
        col[0],
        col[1],
        col[2],
        col[3],
        if hud { 1.0 } else { 0.0 },
    ]);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Centre,
    Right,
}

/// Width of `s` at cell height `h`.
pub fn text_width(font: &Font, s: &str, h: f32) -> f32 {
    s.bytes()
        .map(|b| font.adv[(b.clamp(32, 127) - 32) as usize] * h)
        .sum()
}

/// Lays out ASCII text. Cells are square (`h` x `h`), advances from the font.
#[allow(clippy::too_many_arguments)]
fn push_text(
    v: &mut Vec<f32>,
    font: &Font,
    s: &str,
    pos: [f32; 3],
    h: f32,
    col: [f32; 4],
    align: Align,
    hud: bool,
) {
    let w = text_width(font, s, h);
    // Mirrored scenes negate every world x afterwards (see `mirror`), so lay
    // world text out reversed about its centre to keep it readable.
    let flip = !hud && MIRROR.with(Cell::get);
    let mut x = match align {
        Align::Left => pos[0],
        Align::Centre => pos[0] - w / 2.0,
        Align::Right => pos[0] - w,
    };
    for b in s.bytes() {
        let g = (b.clamp(32, 127) - 32) as usize;
        let a = font.adv[g] * h;
        if b != b' ' {
            let gx = x + a / 2.0;
            let gx = if flip { 2.0 * pos[0] - gx } else { gx };
            push_glyph(v, [gx, pos[1], pos[2]], [h, h], g, col, hud);
        }
        x += a;
    }
}

fn hud_rect(v: &mut Vec<f32>, x: f32, y: f32, w: f32, h: f32, col: [f32; 4]) {
    push_glyph(
        v,
        [x + w / 2.0, y + h / 2.0, 0.0],
        [w, h],
        CELL_SOLID,
        col,
        true,
    );
}

fn rgba(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
}

fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

/// Camera for a frame. Returns (view-projection, eye).
pub fn camera(cam_x: f32, width: f32, height: f32, strings: usize) -> (Mat4, [f32; 3]) {
    let aspect = (width / height.max(1.0)).max(0.3);
    // Looking down the neck: the strings at the hit line run big across the
    // bottom fifth of the screen and the highway narrows to a point about
    // two thirds up. Six- to eight-string parts sit a little further back.
    // Narrow (portrait) screens move back along the same line of sight and
    // look down more steeply, so a playable fret window still fits.
    let k = (strings as f32 - 4.0).max(0.0);
    let pull = (1.2 / aspect).max(1.0).powf(0.25);
    let tall = (1.0 / aspect - 1.0).clamp(0.0, 1.2);
    let top_string = STRING_BASE + (strings.max(1) - 1) as f32 * STRING_GAP;
    let target = [cam_x, 0.0, -11.0 - k - 4.0 * tall];
    let eye_y = (top_string + 2.0 + 0.4 * k) * pull * (1.0 + 0.9 * tall);
    let eye_z = target[2] + (6.5 + 0.25 * k - target[2]) * pull;
    let eye = [cam_x, eye_y, eye_z];
    let view = math::look_at(eye, target, [0.0, 1.0, 0.0]);
    let proj = math::perspective(0.7 + 0.05 * k, aspect, 0.1, 150.0);
    (math::mul(&proj, &view), eye)
}

fn technique(n: &Note) -> Option<&'static str> {
    if n.hammer_on {
        Some("H")
    } else if n.pull_off {
        Some("P")
    } else if n.tap {
        Some("T")
    } else if n.palm_mute {
        Some("PM")
    } else if n.harmonic {
        Some("<>")
    } else if n.bend > 0.0 {
        Some("^")
    } else if n.tremolo {
        Some("~~")
    } else if n.vibrato {
        Some("~")
    } else {
        None
    }
}

/// Builds every instance for one frame. Returns the view-projection matrix.
pub fn build(f: &Frame, opts: &Options, font: &Font, out: &mut Batches) -> Mat4 {
    out.clear_all();
    let ch = f.chart;
    let strings = ch.string_count.max(1);
    MIRROR.with(|m| m.set(opts.lefty));
    let cam_x = if opts.lefty { -f.cam_x } else { f.cam_x };
    let (vp, _eye) = camera(cam_x, f.width, f.height, strings);
    let speed = opts.speed;
    let z_of = |t: f64| -((t - f.t) as f32) * speed;
    let far = z_of(f.t + VIEW_S);
    let near_z = 30.0;
    let depth = near_z - far;
    let mid_z = (near_z + far) / 2.0;
    let board_w = FRETS as f32 + 1.0;
    let top = STRING_BASE + (strings - 1) as f32 * STRING_GAP;

    // Board (slightly see-through, so a backdrop shows faintly beneath it;
    // first in the blended list, so it sits under every note), and the lit
    // fret window of the current anchor.
    let acc = opts.accent;
    push_box(
        &mut out.clear,
        [board_w / 2.0 - 0.5, -0.06, mid_z],
        [board_w + 1.0, 0.1, depth],
        rgba(add3([0.045, 0.05, 0.07], acc, 0.025), 0.86),
        0.0,
    );
    let anchor = ch.anchor_at(f.t.max(0.0));
    let lo = (anchor.fret - 1).max(0) as f32;
    let aw = anchor.width.max(4) as f32;
    push_box(
        &mut out.opaque,
        [lo + aw / 2.0, -0.005, mid_z],
        [aw, 0.02, depth],
        rgba(add3([0.09, 0.095, 0.13], acc, 0.11), 1.0),
        0.0,
    );
    // Lit edges of the fret window, running off into the distance.
    for x in [lo, lo + aw] {
        push_box(
            &mut out.opaque,
            [x, 0.012, mid_z],
            [0.06, 0.02, depth],
            rgba(acc, 1.0),
            0.6,
        );
    }
    // Haze on the horizon, in the second stage colour.
    push_glow(
        &mut out.glow,
        [f.cam_x, 2.0, far * 0.9],
        [70.0, 22.0],
        opts.accent2,
        0.24,
        true,
    );
    push_glow(
        &mut out.glow,
        [f.cam_x, 0.5, far * 0.85],
        [30.0, 8.0],
        acc,
        0.2,
        true,
    );

    // Fret wires and inlay dots.
    for fr in 0..=FRETS {
        let inside = (fr as f32) >= lo && (fr as f32) <= lo + aw;
        let c = if inside {
            [0.42, 0.45, 0.55]
        } else {
            [0.22, 0.24, 0.3]
        };
        push_box(
            &mut out.opaque,
            [fret_x(fr as f32), 0.01, mid_z],
            [0.035, 0.03, depth],
            rgba(c, 1.0),
            0.0,
        );
    }
    // Upright fret wires on the fretboard at the hit line; the current fret
    // window's wires are lit, so it is easy to see where a note lands.
    let top_string = STRING_BASE + (strings - 1) as f32 * STRING_GAP;
    let wire_h = top_string + 0.12;
    for fr in 0..=FRETS {
        let inside = (fr as f32) >= lo && (fr as f32) <= lo + aw;
        let (c, a, g) = if inside {
            (mix3([1.0, 1.0, 1.0], acc, 0.45), 0.95, 0.35)
        } else {
            ([0.32, 0.34, 0.42], 0.55, 0.0)
        };
        push_box(
            &mut out.opaque,
            [fret_x(fr as f32), wire_h / 2.0, -0.03],
            [0.028, wire_h, 0.028],
            rgba(c, a),
            g,
        );
        if inside {
            push_glow(
                &mut out.glow,
                [fret_x(fr as f32), wire_h / 2.0, 0.0],
                [0.5, wire_h * 1.25],
                acc,
                0.26,
                false,
            );
        }
    }
    for fr in [3, 5, 7, 9, 12, 15, 17, 19, 21, 24] {
        let x = note_x(fr);
        let n = if fr % 12 == 0 { 2 } else { 1 };
        for k in 0..n {
            let dx = if n == 2 { (k as f32 - 0.5) * 0.35 } else { 0.0 };
            push_box(
                &mut out.opaque,
                [x + dx, 0.02, 0.9],
                [0.14, 0.02, 0.14],
                rgba([0.3, 0.32, 0.4], 1.0),
                0.0,
            );
        }
    }

    // Fret numbers in front of the hit line, brighter inside the fret window.
    for fr in 1..=FRETS {
        let inside = (fr as f32) > lo && (fr as f32) <= lo + aw;
        let a = if inside { 0.95 } else { 0.35 };
        push_text(
            &mut out.text,
            font,
            fret_label(fr),
            [note_x(fr), 0.16, 0.3],
            0.4,
            [0.85, 0.87, 0.95, a],
            Align::Centre,
            false,
        );
    }

    // Beat and bar lines.
    let b0 = ch.beats.partition_point(|b| b.time < f.t - 0.1);
    for b in ch.beats[b0..].iter().take_while(|b| b.time <= f.t + VIEW_S) {
        let bar = b.measure.is_some();
        let (th, c) = if bar {
            (0.07, mix3([0.3, 0.32, 0.42], acc, 0.55))
        } else {
            (0.025, [0.2, 0.21, 0.28])
        };
        push_box(
            &mut out.opaque,
            [board_w / 2.0 - 0.5, 0.012, z_of(b.time)],
            [board_w, 0.02, th],
            rgba(c, 1.0),
            0.0,
        );
    }

    // (The strings and upright fret wires mark the hit line.)

    // Strings at the hit line; a string lights up while a note on it is due.
    let mut due = [false; 8];
    // Where and when the latest note on each string was played (for the
    // string's vibration).
    let mut due_x = [0.0f32; 8];
    let mut due_t = [f64::NEG_INFINITY; 8];
    // (Look back far enough to catch long sustains still ringing.)
    let rings = |time: f64, sus: f64| time - 0.12 <= f.t && f.t <= time + sus.max(0.12);
    let n0 = ch.notes.partition_point(|n| n.time < f.t - 12.0);
    for n in ch.notes[n0..].iter().take_while(|n| n.time <= f.t + 0.12) {
        if rings(n.time, n.sustain) {
            let s = n.string.min(7);
            due[s] = true;
            if n.time <= f.t && n.time > due_t[s] {
                due_t[s] = n.time;
                due_x[s] = if n.fret == 0 {
                    lo + aw / 2.0
                } else {
                    note_x(n.fret)
                };
            }
        }
    }
    let c0 = ch.chords.partition_point(|c| c.time < f.t - 12.0);
    for c in ch.chords[c0..].iter().take_while(|c| c.time <= f.t + 0.12) {
        for n in &c.notes {
            if rings(c.time, n.sustain) {
                let s = n.string.min(7);
                due[s] = true;
                if c.time <= f.t && c.time > due_t[s] {
                    due_t[s] = c.time;
                    due_x[s] = if n.fret == 0 {
                        lo + aw / 2.0
                    } else {
                        note_x(n.fret)
                    };
                }
            }
        }
    }
    for (s, lit) in due.iter().enumerate().take(strings) {
        let y = string_y(s, strings, opts);
        let c = STRING_RGB[s % 8];
        if !*lit {
            push_box(
                &mut out.opaque,
                [board_w / 2.0 - 0.5, y, 0.0],
                [board_w, 0.035, 0.035],
                rgba(scale(c, 0.55), 1.0),
                0.0,
            );
            continue;
        }
        // A ringing string glows and vibrates around where it was played.
        push_glow(
            &mut out.glow,
            [due_x[s], y, 0.05],
            [board_w * 0.9, 0.42],
            c,
            0.4,
            false,
        );
        let age = (f.t - due_t[s]).max(0.0) as f32;
        let amp = if due_t[s].is_finite() {
            0.07 * (-age * 2.2).exp()
        } else {
            0.0
        };
        // Vibration looks like a blurred band: the bright string itself, plus
        // faint copies either side that close in as the note decays.
        push_box(
            &mut out.opaque,
            [board_w / 2.0 - 0.5, y, 0.0],
            [board_w, 0.045, 0.045],
            rgba(c, 1.0),
            0.9,
        );
        let wob = 0.6 + 0.4 * (f.t as f32 * 47.0).sin().abs();
        for side in [-1.0f32, 1.0] {
            push_box(
                &mut out.clear,
                [board_w / 2.0 - 0.5, y + side * amp * wob, 0.01],
                [board_w, 0.03, 0.03],
                rgba(c, 0.45 * (amp / 0.07)),
                0.8,
            );
        }
    }

    // Notes and chords, far to near (so nearer glyphs draw over farther ones).
    let lo_t = f.t - TAIL_S;
    let hi_t = f.t + VIEW_S;
    // Sorted by start, not end: look back far enough for long sustains (each
    // one is still checked against its own end below).
    let first_n = ch
        .notes
        .partition_point(|n| n.time < lo_t - SUSTAIN_LOOKBACK_S);
    let first_c = ch
        .chords
        .partition_point(|c| c.time < lo_t - SUSTAIN_LOOKBACK_S);
    let last_n = ch.notes.partition_point(|n| n.time <= hi_t);
    let last_c = ch.chords.partition_point(|c| c.time <= hi_t);
    for i in (first_c..last_c).rev() {
        let c = &ch.chords[i];
        let res = f.chord_res.get(i).copied().flatten();
        let sus = c.notes.iter().map(|n| n.sustain).fold(0.0, f64::max);
        if c.time + sus < lo_t {
            continue;
        }
        let held = sus > 0.05 && c.time < f.t && f.t <= c.time + sus && res != Some(false);
        let z = if held { 0.0 } else { z_of(c.time) };
        for n in &c.notes {
            draw_note(
                out,
                f,
                opts,
                font,
                n,
                res,
                strings,
                anchor.fret,
                anchor.width,
                false,
            );
        }
        if held || c.time >= f.t - 0.05 || res.is_none() {
            // Chord frame around the strings it uses.
            let fretted = c.notes.iter().filter(|n| n.fret > 0).map(|n| n.fret);
            let (mn, mx) = fretted.fold((i32::MAX, i32::MIN), |(a, b), x| (a.min(x), b.max(x)));
            let (x0, x1) = if mn > mx {
                (lo, lo + aw)
            } else {
                (mn as f32 - 1.0, mx as f32)
            };
            let fade = if held { 1.0 } else { fade(f.t, c.time) };
            let col = rgba(
                if res == Some(false) {
                    [0.4, 0.4, 0.45]
                } else {
                    [0.92, 0.93, 1.0]
                },
                fade,
            );
            let y_top = top + 0.32;
            push_box(
                &mut out.clear,
                [x0 - 0.04, (y_top) / 2.0, z],
                [0.06, y_top, 0.06],
                col,
                0.2,
            );
            push_box(
                &mut out.clear,
                [x1 + 0.04, (y_top) / 2.0, z],
                [0.06, y_top, 0.06],
                col,
                0.2,
            );
            push_box(
                &mut out.clear,
                [(x0 + x1) / 2.0, y_top, z],
                [x1 - x0 + 0.14, 0.06, 0.06],
                col,
                0.2,
            );
            if let Some(name) = ch.chord_names.get(c.template).filter(|s| !s.is_empty()) {
                push_text(
                    &mut out.text,
                    font,
                    name,
                    [(x0 + x1) / 2.0, y_top + 0.3, z],
                    0.42,
                    [1.0, 1.0, 1.0, fade],
                    Align::Centre,
                    false,
                );
            }
        }
    }
    for i in (first_n..last_n).rev() {
        let n = &ch.notes[i];
        if n.time + n.sustain < lo_t {
            continue;
        }
        let res = f.note_res.get(i).copied().flatten();
        draw_note(
            out,
            f,
            opts,
            font,
            n,
            res,
            strings,
            anchor.fret,
            anchor.width,
            true,
        );
    }

    // Hit bursts.
    for e in f.effects {
        let age = (f.t - e.t) as f32;
        if !(0.0..0.35).contains(&age) {
            continue;
        }
        let k = age / 0.35;
        let c = STRING_RGB[e.string % 8];
        let a = (1.0 - k) * 0.9;
        let s = 0.7 + k * 1.4;
        push_glyph(
            &mut out.text,
            [e.x, e.y, 0.05],
            [s, s],
            CELL_GLOW,
            [c[0] * 0.6 + 0.4, c[1] * 0.6 + 0.4, c[2] * 0.6 + 0.4, a],
            false,
        );
        push_glyph(
            &mut out.text,
            [e.x, e.y, 0.06],
            [s * 0.8, s * 0.8],
            CELL_RING,
            [1.0, 1.0, 1.0, a * 0.8],
            false,
        );
        // Sparks thrown up and out, falling back (no state: a function of age).
        for i in 0..10 {
            let ang = i as f32 * 0.628 + e.x * 1.7;
            let sp = 2.2 + (i % 3) as f32 * 0.9;
            let px = e.x + ang.cos() * sp * age;
            let py = e.y + (ang.sin().abs() * sp + 1.5) * age - 6.0 * age * age;
            push_glow(
                &mut out.glow,
                [px, py, 0.1 + 0.6 * age],
                [0.22, 0.22],
                mix3(c, [1.0, 1.0, 1.0], 0.4),
                (1.0 - k) * 0.9,
                false,
            );
        }
    }

    if opts.lefty {
        mirror(out);
    }
    build_hud(f, font, out, opts.accent);
    vp
}

thread_local! {
    /// Set while building a mirrored (left-handed) frame.
    static MIRROR: Cell<bool> = const { Cell::new(false) };
}

/// Left-handed view: negate x of every world-space instance.
fn mirror(out: &mut Batches) {
    for v in [&mut out.opaque, &mut out.clear] {
        for inst in v.as_chunks_mut::<BOX_FLOATS>().0 {
            inst[0] = -inst[0];
        }
    }
    for v in [&mut out.text, &mut out.glow] {
        for inst in v.as_chunks_mut::<GLYPH_FLOATS>().0 {
            inst[0] = -inst[0];
        }
    }
}

fn fade(now: f64, t: f64) -> f32 {
    // Fade in at the far end, fade out after the hit line.
    let ahead = (t - now) / VIEW_S;
    let fin = ((1.0 - ahead) / 0.12).clamp(0.0, 1.0) as f32;
    let past = now - t;
    let fout = if past > 0.0 {
        (1.0 - past / TAIL_S).clamp(0.0, 1.0) as f32
    } else {
        1.0
    };
    fin * fout
}

#[allow(clippy::too_many_arguments)]
fn draw_note(
    out: &mut Batches,
    f: &Frame,
    opts: &Options,
    font: &Font,
    n: &Note,
    res: Option<bool>,
    strings: usize,
    anchor_fret: i32,
    anchor_width: i32,
    label: bool,
) {
    let speed = opts.speed;
    let z = -((n.time - f.t) as f32) * speed;
    let y = string_y(n.string, strings, opts);
    let base = STRING_RGB[n.string % 8];
    let past = f.t > n.time;
    let hit = res == Some(true);
    let missed = res == Some(false);
    let col = if missed {
        [0.32, 0.32, 0.36]
    } else if n.mute {
        scale(base, 0.45)
    } else {
        base
    };
    let a = fade(f.t, n.time);

    // Sustain tail (drawn from the hit line on once the note has started).
    if n.sustain > 0.05 {
        let t0 = n.time.max(f.t);
        let t1 = n.time + n.sustain;
        if t1 > t0 {
            let (za, zb) = (-((t0 - f.t) as f32) * speed, -((t1 - f.t) as f32) * speed);
            let x_at = |t: f64| -> f32 {
                let fx = |fr: i32| {
                    if fr == 0 {
                        anchor_centre(anchor_fret, anchor_width) - 0.5
                    } else {
                        note_x(fr)
                    }
                };
                match n.slide_to {
                    Some(to) if n.sustain > 0.0 => {
                        let k = ((t - n.time) / n.sustain).clamp(0.0, 1.0) as f32;
                        fx(n.fret) + (fx(to) - fx(n.fret)) * k
                    }
                    _ => fx(n.fret),
                }
            };
            let sliding = n.slide_to.is_some();
            let segs = if sliding { 8 } else { 1 };
            // The tail runs to its end whatever happened to the head: lit
            // while held, dimmed (and grey) if the note was missed. Only its
            // far end fades in; it never takes the head's fade-out.
            let ta = if missed {
                0.35
            } else if hit || past {
                0.75
            } else {
                0.5
            } * fade(f.t, t0);
            for k in 0..segs {
                let (s0, s1) = (k as f64 / segs as f64, (k + 1) as f64 / segs as f64);
                let (ta_t, tb_t) = (t0 + (t1 - t0) * s0, t0 + (t1 - t0) * s1);
                let (z0, z1) = (za + (zb - za) * s0 as f32, za + (zb - za) * s1 as f32);
                let x = (x_at(ta_t) + x_at(tb_t)) / 2.0;
                let w = if n.fret == 0 && !sliding {
                    anchor_width.max(4) as f32 * 0.9
                } else {
                    0.16
                };
                push_styled(
                    &mut out.clear,
                    [x, y - 0.06, (z0 + z1) / 2.0],
                    [w, 0.07, (z0 - z1).abs() + 0.02],
                    rgba(col, ta),
                    if missed {
                        0.0
                    } else if hit || past {
                        0.8
                    } else {
                        0.25
                    },
                    STYLE_RIBBON,
                );
            }
        }
    }

    // Landing marker: a ring on the string where the note will land,
    // growing brighter as it approaches.
    let ahead = n.time - f.t;
    if n.fret > 0 && ahead > 0.0 && ahead < 1.2 && res.is_none() {
        let k = (1.0 - ahead / 1.2) as f32;
        let s = 0.75 - 0.25 * k;
        push_glyph(
            &mut out.text,
            [note_x(n.fret), y, 0.06],
            [s, s],
            CELL_RING,
            rgba(base, 0.25 + 0.6 * k),
            false,
        );
    }

    // A sustained note stays on the hit line, lit, until it ends.
    let held = n.sustain > 0.05 && past && f.t <= n.time + n.sustain && !missed;
    let (z, a) = if held {
        (0.0, 1.0)
    } else if past {
        // Passed without a hit: fade out just past the line instead of
        // flying on towards the camera over the fret numbers.
        let k = (1.0 - (f.t - n.time - n.sustain.max(0.0)) / 0.15).clamp(0.0, 1.0) as f32;
        (z.min(0.5), a.min(1.0) * k)
    } else {
        (z, a)
    };
    // Once hit, a short note is gone (the burst replaces it).
    if hit && past && !held {
        return;
    }
    if past && !held && n.time + n.sustain.max(0.0) < f.t - TAIL_S {
        return;
    }
    let glow = if held {
        0.55 + 0.15 * ((f.t * 12.0).sin() as f32)
    } else if (n.time - f.t).abs() < 0.08 && !missed {
        0.6
    } else {
        0.08
    };
    // A halo builds up as the note comes in, strongest while held.
    let halo = if missed {
        0.0
    } else if held {
        0.6 + 0.15 * ((f.t * 12.0).sin() as f32)
    } else {
        let ahead = (n.time - f.t) as f32;
        if (0.0..1.0).contains(&ahead) {
            0.45 * (1.0 - ahead)
        } else {
            0.0
        }
    };
    if n.fret == 0 {
        // Open string: a bar across the fret window.
        let cx = anchor_centre(anchor_fret, anchor_width) - 0.5;
        let w = anchor_width.max(4) as f32;
        push_styled(
            &mut out.clear,
            [cx, y, z],
            [w, 0.14, 0.22],
            rgba(col, a),
            glow,
            STYLE_GEM,
        );
        if halo > 0.0 {
            push_glow(
                &mut out.glow,
                [cx, y, z + 0.1],
                [w * 1.1, 0.7],
                col,
                halo * a,
                false,
            );
        }
        return;
    }
    let x = note_x(n.fret);
    push_styled(
        &mut out.clear,
        [x, y, z],
        [0.8, 0.32, 0.26],
        rgba(col, a),
        glow,
        STYLE_GEM,
    );
    if halo > 0.0 {
        push_glow(
            &mut out.glow,
            [x, y, z + 0.15],
            [1.7, 1.0],
            col,
            halo * a,
            false,
        );
    }
    if label {
        let txt = fret_label(n.fret);
        push_text(
            &mut out.text,
            font,
            txt,
            [x, y - 0.01, z + 0.16],
            0.4,
            [0.04, 0.04, 0.07, a],
            Align::Centre,
            false,
        );
    }
    if n.mute {
        push_text(
            &mut out.text,
            font,
            "x",
            [x, y + 0.3, z + 0.14],
            0.3,
            [1.0, 1.0, 1.0, a],
            Align::Centre,
            false,
        );
    } else if let Some(tech) = technique(n) {
        push_text(
            &mut out.text,
            font,
            tech,
            [x, y + 0.3, z + 0.14],
            0.26,
            [1.0, 1.0, 1.0, a * 0.9],
            Align::Centre,
            false,
        );
    }
}

fn fret_label(fret: i32) -> &'static str {
    const L: [&str; 31] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30",
    ];
    L[fret.clamp(0, 30) as usize]
}

/// Formats an integer into a stack buffer (the HUD must not allocate).
struct Num {
    buf: [u8; 24],
    len: usize,
}

impl Num {
    fn new(mut v: i64, suffix: &[u8]) -> Num {
        let mut tmp = [0u8; 20];
        let mut n = 0;
        let neg = v < 0;
        if neg {
            v = -v;
        }
        loop {
            tmp[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        let mut buf = [0u8; 24];
        let mut len = 0;
        if neg {
            buf[0] = b'-';
            len = 1;
        }
        for k in (0..n).rev() {
            buf[len] = tmp[k];
            len += 1;
        }
        for &b in suffix.iter().take(24 - len) {
            buf[len] = b;
            len += 1;
        }
        Num { buf, len }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

/// Where the section navigator sits, in drawing-buffer pixels: (x, y, w, h).
pub fn nav_rect(width: f32, dpr: f32) -> (f32, f32, f32, f32) {
    let d = dpr.max(1.0);
    let m = 12.0 * d;
    (m, 8.0 * d, (width - 2.0 * m).max(1.0), 34.0 * d)
}

/// Song time to jump to for a click at drawing-buffer pixel (x, y) on the
/// navigator: the start of the section under it. `None` if not on the bar.
pub fn nav_seek(chart: &Chart, end: f64, width: f32, dpr: f32, x: f32, y: f32) -> Option<f64> {
    let (nx, ny, nw, nh) = nav_rect(width, dpr);
    // A little extra height so it is easy to hit on a touch screen.
    if chart.sections.is_empty() || x < nx || x > nx + nw || y < 0.0 || y > ny + nh + 6.0 * dpr {
        return None;
    }
    let t = ((x - nx) / nw) as f64 * end.max(1.0);
    let i = chart.sections.partition_point(|s| s.time <= t);
    Some(if i <= 1 {
        0.0
    } else {
        chart.sections[i - 1].time
    })
}

fn build_hud(f: &Frame, font: &Font, out: &mut Batches, acc: [f32; 3]) {
    let d = f.dpr.max(1.0);
    let m = 16.0 * d;
    let h = f.hud;
    let big = (30.0 * d).min(f.height * 0.07);
    let small = big * 0.55;
    let white = [1.0, 1.0, 1.0, 0.95];
    let dim = [0.75, 0.77, 0.85, 0.9];

    // Section navigator (or a plain progress bar when there are no sections).
    let (nx, ny, nw, nh) = nav_rect(f.width, f.dpr);
    let secs = &f.chart.sections;
    let end = h.end.max(1.0);
    let px = |t: f64| nx + nw * (t / end).clamp(0.0, 1.0) as f32;
    let top = if secs.is_empty() {
        hud_rect(
            &mut out.hud,
            0.0,
            0.0,
            f.width,
            4.0 * d,
            [1.0, 1.0, 1.0, 0.12],
        );
        hud_rect(
            &mut out.hud,
            0.0,
            0.0,
            f.width * h.progress.clamp(0.0, 1.0),
            4.0 * d,
            [1.0, 1.0, 1.0, 0.6],
        );
        m
    } else {
        hud_rect(&mut out.hud, nx, ny, nw, nh, [1.0, 1.0, 1.0, 0.05]);
        let cur = secs.partition_point(|x| x.time <= f.t).saturating_sub(1);
        let label_h = (nh * 0.38).min(13.0 * d);
        for (i, sec) in secs.iter().enumerate() {
            let x0 = if i == 0 { nx } else { px(sec.time) };
            let x1 = secs.get(i + 1).map_or(nx + nw, |n| px(n.time));
            // Bar height shows how busy the section is.
            let busy = f.density.get(i).copied().unwrap_or(0.5);
            let bh = nh * (0.22 + 0.78 * busy);
            let played = secs.get(i + 1).is_some_and(|n| n.time <= f.t);
            // Played sections in the stage colour, the current one bright.
            let bar_col = if i == cur {
                [acc[0], acc[1], acc[2], 0.95]
            } else if played {
                [acc[0], acc[1], acc[2], 0.4]
            } else {
                [1.0, 1.0, 1.0, 0.2]
            };
            hud_rect(
                &mut out.hud,
                x0 + d,
                ny + nh - bh,
                (x1 - x0 - 2.0 * d).max(1.0),
                bh,
                bar_col,
            );
            if !sec.name.is_empty() && text_width(font, &sec.name, label_h) < x1 - x0 - 8.0 * d {
                let col = if i == cur {
                    [0.05, 0.05, 0.08, 0.95]
                } else {
                    dim
                };
                let ly = if bh > label_h * 1.3 {
                    ny + nh - label_h * 0.65
                } else {
                    ny + nh - bh - label_h * 0.6
                };
                push_text(
                    &mut out.hud,
                    font,
                    &sec.name,
                    [(x0 + x1) / 2.0, ly, 0.0],
                    label_h,
                    col,
                    Align::Centre,
                    true,
                );
            }
        }
        let p = px(f.t.max(0.0));
        hud_rect(
            &mut out.hud,
            nx,
            ny + nh - 3.0 * d,
            p - nx,
            3.0 * d,
            [1.0, 1.0, 1.0, 0.55],
        );
        hud_rect(&mut out.hud, p - d, ny, 2.0 * d, nh, [1.0, 1.0, 1.0, 0.95]);
        ny + nh + 8.0 * d
    };

    // Score, accuracy and streak in the top-right corner.
    let score = Num::new(h.score, b"");
    push_text(
        &mut out.hud,
        font,
        score.as_str(),
        [f.width - m, top + big * 0.6, 0.0],
        big,
        white,
        Align::Right,
        true,
    );
    let mut row = top + big * 1.25 + small * 0.4;
    if h.judged > 0 {
        let acc = Num::new((h.accuracy * 100.0).round() as i64, b"%");
        push_text(
            &mut out.hud,
            font,
            acc.as_str(),
            [f.width - m, row, 0.0],
            small,
            dim,
            Align::Right,
            true,
        );
        row += small * 1.2;
    }
    if h.streak >= 2 {
        let s = Num::new(h.streak, b" streak");
        push_text(
            &mut out.hud,
            font,
            s.as_str(),
            [f.width - m, row, 0.0],
            small,
            white,
            Align::Right,
            true,
        );
    }

    // Lyrics in the top-left: the current line (sung words lit) and the next.
    let lines = &f.chart.lyrics;
    let li = lines
        .partition_point(|l| l.start <= f.t + 0.6)
        .saturating_sub(1);
    if let Some(line) = lines
        .get(li)
        .filter(|l| l.start <= f.t + 0.6 && f.t <= l.end + 1.2)
    {
        let base = big * 0.8;
        let max_w = f.width * 0.55;
        let mut y = top + base * 0.6;
        for (k, l) in [Some(line), lines.get(li + 1)]
            .into_iter()
            .flatten()
            .enumerate()
        {
            let w: f32 = l
                .words
                .iter()
                .map(|w| text_width(font, &w.text, base))
                .sum();
            let hh = if w > max_w { base * max_w / w } else { base };
            let mut x = m;
            for word in &l.words {
                let lit = k == 0 && word.time <= f.t;
                let col = if lit {
                    [
                        0.5 + 0.5 * acc[0],
                        0.5 + 0.5 * acc[1],
                        0.5 + 0.5 * acc[2],
                        1.0,
                    ]
                } else if k == 0 {
                    white
                } else {
                    [0.75, 0.77, 0.85, 0.55]
                };
                push_text(
                    &mut out.hud,
                    font,
                    &word.text,
                    [x, y, 0.0],
                    hh,
                    col,
                    Align::Left,
                    true,
                );
                x += text_width(font, &word.text, hh);
            }
            y += hh * 1.15;
        }
    }

    // Mic level meter (bottom-left), so players can see they are heard.
    if let Some(level) = h.mic {
        let w = 90.0 * d;
        let hh = 6.0 * d;
        let y = f.height - m - hh;
        let k = (level.max(1e-6).log10() / 3.0).clamp(0.0, 1.0);
        hud_rect(&mut out.hud, m, y, w, hh, [1.0, 1.0, 1.0, 0.15]);
        hud_rect(&mut out.hud, m, y, w * k, hh, [0.5, 0.95, 0.6, 0.85]);
        push_text(
            &mut out.hud,
            font,
            "mic",
            [m + w + 8.0 * d, y + hh / 2.0, 0.0],
            small * 0.75,
            dim,
            Align::Left,
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riffboard_core::chart::{Anchor, Beat, Chord};

    fn chart() -> Chart {
        Chart {
            string_count: 4,
            bass: true,
            tuning: vec![0; 4],
            anchors: vec![Anchor {
                time: 0.0,
                fret: 3,
                width: 4,
            }],
            beats: (0..20)
                .map(|i| Beat {
                    time: i as f64 * 0.5,
                    measure: (i % 4 == 0).then_some(i / 4),
                })
                .collect(),
            notes: (0..10)
                .map(|i| Note {
                    time: 1.0 + i as f64 * 0.5,
                    string: i % 4,
                    fret: (i % 5) as i32,
                    sustain: 0.3,
                    ..Note::default()
                })
                .collect(),
            chords: vec![Chord {
                time: 2.25,
                template: 0,
                notes: vec![
                    Note {
                        time: 2.25,
                        string: 0,
                        fret: 3,
                        ..Note::default()
                    },
                    Note {
                        time: 2.25,
                        string: 1,
                        fret: 5,
                        ..Note::default()
                    },
                ],
            }],
            chord_names: vec!["G5".into()],
            ..Chart::default()
        }
    }

    #[test]
    fn builds_visible_notes_only() {
        let c = chart();
        let nr = vec![None; c.notes.len()];
        let cr = vec![None; c.chords.len()];
        let mut out = Batches::default();
        let f = Frame {
            chart: &c,
            note_res: &nr,
            chord_res: &cr,
            t: 0.0,
            cam_x: 4.0,
            width: 1280.0,
            height: 720.0,
            dpr: 1.0,
            effects: &[],
            hud: Hud::default(),
            section: "Intro",
            density: &[],
        };
        build(&f, &Options::default(), &Font::default(), &mut out);
        assert_eq!(out.opaque.len() % BOX_FLOATS, 0);
        assert_eq!(out.clear.len() % BOX_FLOATS, 0);
        assert_eq!(out.text.len() % GLYPH_FLOATS, 0);
        let gems_t0 = out.clear.len() / BOX_FLOATS;
        // Later in the song, fewer notes remain ahead.
        let f2 = Frame { t: 5.0, ..f };
        build(&f2, &Options::default(), &Font::default(), &mut out);
        assert!(out.clear.len() / BOX_FLOATS < gems_t0);
        // HUD always has the progress bar and score.
        assert!(out.hud.len() / GLYPH_FLOATS >= 3);
    }

    /// Ribbon instances (style, alpha, near z, far z) in the blended batch.
    fn ribbons(out: &Batches) -> Vec<(f32, f32, f32)> {
        out.clear
            .as_chunks::<BOX_FLOATS>()
            .0
            .iter()
            .filter(|b| b[11] == STYLE_RIBBON)
            .map(|b| (b[9], b[2] + b[5] / 2.0, b[2] - b[5] / 2.0))
            .collect()
    }

    #[test]
    fn sustain_tail_lasts_to_its_end() {
        // One long note, then short ones after it (which once hid it from
        // the visible-range search).
        let mut c = chart();
        c.notes = vec![Note {
            time: 1.0,
            string: 1,
            fret: 3,
            sustain: 2.0,
            ..Note::default()
        }];
        c.notes.extend((0..6).map(|i| Note {
            time: 1.2 + i as f64 * 0.1,
            string: 0,
            fret: 1,
            ..Note::default()
        }));
        c.chords.clear();
        let cr: Vec<Option<bool>> = vec![];
        let opts = Options::default();
        for res in [None, Some(true), Some(false)] {
            let mut nr = vec![None; c.notes.len()];
            nr[0] = res;
            let mut out = Batches::default();
            let f = Frame {
                chart: &c,
                note_res: &nr,
                chord_res: &cr,
                t: 2.0, // one second into a two-second note
                cam_x: 4.0,
                width: 1280.0,
                height: 720.0,
                dpr: 1.0,
                effects: &[],
                hud: Hud::default(),
                section: "",
                density: &[],
            };
            build(&f, &opts, &Font::default(), &mut out);
            let r = ribbons(&out);
            assert_eq!(r.len(), 1, "{res:?}: {r:?}");
            let (alpha, near, far) = r[0];
            assert!(alpha > 0.2, "{res:?}: tail faded to {alpha}");
            // Runs from the hit line out to one second's travel away.
            assert!(near.abs() < 0.05, "{res:?}: near end at {near}");
            assert!((far + opts.speed).abs() < 0.05, "{res:?}: far end at {far}");
            // After the note ends, nothing is left.
            build(&Frame { t: 3.5, ..f }, &opts, &Font::default(), &mut out);
            assert!(ribbons(&out).is_empty());
        }
    }

    #[test]
    fn no_allocation_after_warmup() {
        let c = chart();
        let nr = vec![None; c.notes.len()];
        let cr = vec![None; c.chords.len()];
        let mut out = Batches::reserved();
        let caps = (
            out.opaque.capacity(),
            out.clear.capacity(),
            out.glow.capacity(),
            out.text.capacity(),
            out.hud.capacity(),
        );
        for k in 0..200 {
            let f = Frame {
                chart: &c,
                note_res: &nr,
                chord_res: &cr,
                t: k as f64 * 0.016,
                cam_x: 4.0,
                width: 800.0,
                height: 600.0,
                dpr: 2.0,
                effects: &[],
                hud: Hud {
                    score: k * 37,
                    streak: k,
                    ..Hud::default()
                },
                section: "Verse",
                density: &[],
            };
            build(&f, &Options::default(), &Font::default(), &mut out);
        }
        // The reserved room is enough: no frame grew a buffer.
        let now = (
            out.opaque.capacity(),
            out.clear.capacity(),
            out.glow.capacity(),
            out.text.capacity(),
            out.hud.capacity(),
        );
        assert_eq!(now, caps);
    }

    #[test]
    fn lefty_mirrors_but_keeps_text_order() {
        let c = chart();
        let nr = vec![None; c.notes.len()];
        let cr = vec![None; c.chords.len()];
        let f = Frame {
            chart: &c,
            note_res: &nr,
            chord_res: &cr,
            t: 0.9,
            cam_x: 4.0,
            width: 1280.0,
            height: 720.0,
            dpr: 1.0,
            effects: &[],
            hud: Hud::default(),
            section: "",
            density: &[],
        };
        let font = Font::default();
        let mut a = Batches::default();
        let mut b = Batches::default();
        build(&f, &Options::default(), &font, &mut a);
        build(
            &f,
            &Options {
                lefty: true,
                ..Options::default()
            },
            &font,
            &mut b,
        );
        assert_eq!(a.opaque.len(), b.opaque.len());
        assert_eq!(a.opaque[0], -b.opaque[0]);
        // "10" (fret label): the '1' stays left of the '0' on screen order,
        // i.e. its x is still the smaller one after mirroring.
        let xs = |v: &Vec<f32>| v.chunks(GLYPH_FLOATS).map(|g| g[0]).collect::<Vec<_>>();
        let (ta, tb) = (xs(&a.text), xs(&b.text));
        assert_eq!(ta.len(), tb.len());
        let i10 = 9; // labels 1..9 are one glyph each; "10" starts after them
        assert!(ta[i10] < ta[i10 + 1]);
        assert!(tb[i10] < tb[i10 + 1]);
    }

    #[test]
    fn navigator_jumps_to_section_starts() {
        let mut c = chart();
        c.sections = vec![
            riffboard_core::chart::Section {
                time: 0.0,
                name: "Intro".into(),
            },
            riffboard_core::chart::Section {
                time: 10.0,
                name: "Verse".into(),
            },
            riffboard_core::chart::Section {
                time: 30.0,
                name: "Chorus".into(),
            },
        ];
        let (nx, ny, nw, _) = nav_rect(1000.0, 1.0);
        let at = |c: &Chart, frac: f32| nav_seek(c, 40.0, 1000.0, 1.0, nx + nw * frac, ny + 5.0);
        assert_eq!(at(&c, 0.1), Some(0.0));
        assert_eq!(at(&c, 0.5), Some(10.0)); // t = 20 s, inside the verse
        assert_eq!(at(&c, 0.9), Some(30.0));
        assert_eq!(nav_seek(&c, 40.0, 1000.0, 1.0, 500.0, 300.0), None);
        c.sections.clear();
        assert_eq!(at(&c, 0.5), None);
    }

    #[test]
    fn layout() {
        let o = Options::default();
        // Low string on top by default.
        assert!(string_y(0, 4, &o) > string_y(3, 4, &o));
        let o2 = Options {
            low_string_bottom: true,
            ..o
        };
        assert!(string_y(0, 4, &o2) < string_y(3, 4, &o2));
        assert_eq!(note_x(5), 4.5);
        assert_eq!(Num::new(1234, b"%").as_str(), "1234%");
        assert_eq!(Num::new(0, b"").as_str(), "0");
    }
}
