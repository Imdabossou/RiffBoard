//! Builds one frame of the highway as instance lists for the renderer.
//!
//! Pure Rust (no browser APIs), so the layout is unit-tested natively. Every
//! list is cleared and refilled in place each frame: no allocation once the
//! vectors have grown to their working size.

use riffboard_core::chart::{Chart, Note};

use crate::math::{self, Mat4};

/// Floats per box instance: centre(3) size(3) colour(4) glow(1).
pub const BOX_FLOATS: usize = 11;
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

const FRETS: i32 = 24;
const STRING_GAP: f32 = 0.42;
const STRING_BASE: f32 = 0.32;
/// Seconds of chart shown ahead of the hit line.
const VIEW_S: f64 = 3.2;
/// Seconds a note stays drawn after passing the hit line.
const TAIL_S: f64 = 0.35;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// World units per second of chart.
    pub speed: f32,
    /// Draw the lowest string at the bottom instead of the top.
    pub low_string_bottom: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            speed: 11.0,
            low_string_bottom: false,
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
}

#[derive(Default)]
pub struct Batches {
    pub opaque: Vec<f32>,
    pub clear: Vec<f32>,
    pub text: Vec<f32>,
    pub hud: Vec<f32>,
}

impl Batches {
    fn clear_all(&mut self) {
        self.opaque.clear();
        self.clear.clear();
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
    v.extend_from_slice(&[
        c[0], c[1], c[2], s[0], s[1], s[2], col[0], col[1], col[2], col[3], glow,
    ]);
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
    let mut x = match align {
        Align::Left => pos[0],
        Align::Centre => pos[0] - w / 2.0,
        Align::Right => pos[0] - w,
    };
    for b in s.bytes() {
        let g = (b.clamp(32, 127) - 32) as usize;
        let a = font.adv[g] * h;
        if b != b' ' {
            push_glyph(v, [x + a / 2.0, pos[1], pos[2]], [h, h], g, col, hud);
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
    // Narrow (portrait) screens pull the camera back so a playable fret
    // window still fits across.
    let pull = (1.2 / aspect).max(1.0).powf(0.25);
    let top = STRING_BASE + strings as f32 * STRING_GAP;
    // Framed so the strings sit in the lower fifth of the screen and the
    // far end of the highway around three quarters up. Narrow screens move
    // the camera back along the same line of sight, so the framing holds.
    // Tall screens look down more steeply from higher up, so the highway
    // runs up the extra height instead of leaving it empty.
    let tall = (1.0 / aspect - 1.0).clamp(0.0, 1.2);
    let target = [cam_x, 0.0, -7.0 - 4.0 * tall];
    let eye = [
        cam_x,
        (top + 3.25) * pull * (1.0 + 0.7 * tall),
        -7.0 + 13.0 * pull,
    ];
    let view = math::look_at(eye, target, [0.0, 1.0, 0.0]);
    let proj = math::perspective(0.9, aspect, 0.1, 120.0);
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
    let (vp, _eye) = camera(f.cam_x, f.width, f.height, strings);
    let speed = opts.speed;
    let z_of = |t: f64| -((t - f.t) as f32) * speed;
    let far = z_of(f.t + VIEW_S);
    let near_z = 30.0;
    let depth = near_z - far;
    let mid_z = (near_z + far) / 2.0;
    let board_w = FRETS as f32 + 1.0;
    let top = STRING_BASE + (strings - 1) as f32 * STRING_GAP;

    // Board, and the lit fret window of the current anchor.
    push_box(
        &mut out.opaque,
        [board_w / 2.0 - 0.5, -0.06, mid_z],
        [board_w + 1.0, 0.1, depth],
        rgba([0.07, 0.075, 0.1], 1.0),
        0.0,
    );
    let anchor = ch.anchor_at(f.t.max(0.0));
    let lo = (anchor.fret - 1).max(0) as f32;
    let aw = anchor.width.max(4) as f32;
    push_box(
        &mut out.opaque,
        [lo + aw / 2.0, -0.005, mid_z],
        [aw, 0.02, depth],
        rgba([0.13, 0.14, 0.19], 1.0),
        0.0,
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
            [note_x(fr), 0.28, 0.75],
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
            (0.06, [0.36, 0.38, 0.5])
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

    // Hit line.
    push_box(
        &mut out.opaque,
        [board_w / 2.0 - 0.5, 0.02, 0.0],
        [board_w, 0.03, 0.07],
        rgba([0.85, 0.87, 0.95], 1.0),
        0.4,
    );

    // Strings at the hit line; a string lights up while a note on it is due.
    let mut due = [false; 8];
    let n0 = ch.notes.partition_point(|n| n.time < f.t - 0.12);
    for n in ch.notes[n0..].iter().take_while(|n| n.time <= f.t + 0.12) {
        due[n.string.min(7)] = true;
    }
    let c0 = ch.chords.partition_point(|c| c.time < f.t - 0.12);
    for c in ch.chords[c0..].iter().take_while(|c| c.time <= f.t + 0.12) {
        for n in &c.notes {
            due[n.string.min(7)] = true;
        }
    }
    for (s, lit) in due.iter().enumerate().take(strings) {
        let y = string_y(s, strings, opts);
        let c = STRING_RGB[s % 8];
        let (k, g) = if *lit { (1.0, 0.8) } else { (0.55, 0.0) };
        push_box(
            &mut out.opaque,
            [board_w / 2.0 - 0.5, y, 0.0],
            [board_w, 0.035, 0.035],
            rgba(scale(c, k), 1.0),
            g,
        );
    }

    // Notes and chords, far to near (so nearer glyphs draw over farther ones).
    let lo_t = f.t - TAIL_S;
    let hi_t = f.t + VIEW_S;
    let first_n = ch.notes.partition_point(|n| n.time + n.sustain < lo_t);
    let first_c = ch.chords.partition_point(|c| c.time < lo_t - 2.0);
    let last_n = ch.notes.partition_point(|n| n.time <= hi_t);
    let last_c = ch.chords.partition_point(|c| c.time <= hi_t);
    for i in (first_c..last_c).rev() {
        let c = &ch.chords[i];
        let res = f.chord_res.get(i).copied().flatten();
        let sus = c.notes.iter().map(|n| n.sustain).fold(0.0, f64::max);
        if c.time + sus < lo_t {
            continue;
        }
        let z = z_of(c.time);
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
        if c.time >= f.t - 0.05 || res.is_none() {
            // Chord frame around the strings it uses.
            let fretted = c.notes.iter().filter(|n| n.fret > 0).map(|n| n.fret);
            let (mn, mx) = fretted.fold((i32::MAX, i32::MIN), |(a, b), x| (a.min(x), b.max(x)));
            let (x0, x1) = if mn > mx {
                (lo, lo + aw)
            } else {
                (mn as f32 - 1.0, mx as f32)
            };
            let fade = fade(f.t, c.time);
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
    }

    build_hud(f, font, out);
    vp
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
            let ta = if hit || past { 0.75 } else { 0.5 };
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
                push_box(
                    &mut out.clear,
                    [x, y - 0.06, (z0 + z1) / 2.0],
                    [w, 0.06, (z0 - z1).abs() + 0.02],
                    rgba(col, ta * a),
                    if hit { 0.5 } else { 0.1 },
                );
            }
        }
    }

    // Once hit, the gem is gone (the burst replaces it).
    if hit && past {
        return;
    }
    if past && n.time < f.t - TAIL_S {
        return;
    }
    let glow = if (n.time - f.t).abs() < 0.08 && !missed {
        0.6
    } else {
        0.08
    };
    if n.fret == 0 {
        // Open string: a bar across the fret window.
        let cx = anchor_centre(anchor_fret, anchor_width) - 0.5;
        let w = anchor_width.max(4) as f32;
        push_box(
            &mut out.clear,
            [cx, y, z],
            [w, 0.12, 0.22],
            rgba(col, a),
            glow,
        );
        return;
    }
    let x = note_x(n.fret);
    push_box(
        &mut out.clear,
        [x, y, z],
        [0.78, 0.3, 0.26],
        rgba(col, a),
        glow,
    );
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

fn build_hud(f: &Frame, font: &Font, out: &mut Batches) {
    let d = f.dpr.max(1.0);
    let m = 16.0 * d;
    let h = f.hud;
    let big = (30.0 * d).min(f.height * 0.07);
    let small = big * 0.55;
    let white = [1.0, 1.0, 1.0, 0.95];
    let dim = [0.75, 0.77, 0.85, 0.9];

    // Progress along the very top.
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

    let score = Num::new(h.score, b"");
    push_text(
        &mut out.hud,
        font,
        score.as_str(),
        [m, m + big * 0.6, 0.0],
        big,
        white,
        Align::Left,
        true,
    );
    if h.judged > 0 {
        let acc = Num::new((h.accuracy * 100.0).round() as i64, b"%");
        push_text(
            &mut out.hud,
            font,
            acc.as_str(),
            [m, m + big * 1.25 + small * 0.4, 0.0],
            small,
            dim,
            Align::Left,
            true,
        );
    }
    if h.streak >= 2 {
        let s = Num::new(h.streak, b" streak");
        push_text(
            &mut out.hud,
            font,
            s.as_str(),
            [f.width - m, m + big * 0.6, 0.0],
            small * 1.1,
            white,
            Align::Right,
            true,
        );
    }
    if !f.section.is_empty() {
        push_text(
            &mut out.hud,
            font,
            f.section,
            [f.width / 2.0, m + small * 0.6, 0.0],
            small,
            dim,
            Align::Centre,
            true,
        );
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

    #[test]
    fn no_allocation_after_warmup() {
        let c = chart();
        let nr = vec![None; c.notes.len()];
        let cr = vec![None; c.chords.len()];
        let mut out = Batches::default();
        let mut caps = (0, 0, 0, 0);
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
            };
            build(&f, &Options::default(), &Font::default(), &mut out);
            if k == 100 {
                caps = (
                    out.opaque.capacity(),
                    out.clear.capacity(),
                    out.text.capacity(),
                    out.hud.capacity(),
                );
            }
        }
        // Capacity is reached early; later frames reuse it.
        assert!(out.opaque.capacity() == caps.0 && out.hud.capacity() == caps.3);
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
