//! Chart loading and the highway WebSocket payload.
//!
//! Ported from upstream lib/song.py (wire format), lib/sloppak.py (`load_song`)
//! and lib/routers/ws_highway.py. A pack's arrangement JSON is already in the
//! wire format; it is parsed into typed structs here so that malformed fields
//! are normalised exactly as the original server did before re-serialising.
//! Drum tabs, notation and rigs are not supported (RiffBoard serves guitar and
//! bass charts), so their flags are always false/empty.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::pack;
use crate::util::{as_f64, as_i64, finite_num, num, opt_i64, py_str, round_to, truthy};

// ── Wire model ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct Note {
    pub time: f64,
    pub string: i64,
    pub fret: i64,
    sustain: f64,
    slide_to: i64,
    slide_unpitch_to: i64,
    bend: f64,
    bend_intent: i64,
    bend_values: Option<Vec<(f64, f64)>>,
    flags: [bool; 11], // ho po hm hp pm mt vb tr ac tp | (index 10 unused)
    link_next: bool,
    fret_hand_mute: bool,
    pluck: bool,
    slap: bool,
    right_hand: i64,
    pick_direction: i64,
    fret_finger: i64,
    strum_group: i64,
    scale_degree: i64,
    ignore: bool,
    hand: Option<&'static str>,
}

const FLAG_KEYS: [&str; 10] = ["ho", "po", "hm", "hp", "pm", "mt", "vb", "tr", "ac", "tp"];

fn bend_curve(raw: Option<&Value>) -> Option<Vec<(f64, f64)>> {
    let arr = raw?.as_array()?;
    let mut out: Vec<(f64, f64)> = arr
        .iter()
        .filter_map(|p| Some((finite_num(p.get("t"))?, finite_num(p.get("v"))?)))
        .collect();
    if out.is_empty() {
        return None;
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(out)
}

pub fn note_from_wire(d: &Value, time: Option<f64>) -> Note {
    let mut flags = [false; 11];
    for (i, k) in FLAG_KEYS.iter().enumerate() {
        flags[i] = truthy(d.get(*k));
    }
    // `vb` falls back to a spelled-out `vibrato`.
    if d.get("vb").is_none() {
        flags[6] = truthy(d.get("vibrato"));
    }
    Note {
        time: match d.get("t") {
            Some(t) => as_f64(Some(t), 0.0),
            None => time.unwrap_or(0.0),
        },
        string: as_i64(d.get("s"), 0),
        fret: as_i64(d.get("f"), 0),
        sustain: as_f64(d.get("sus"), 0.0),
        slide_to: as_i64(d.get("sl"), -1),
        slide_unpitch_to: as_i64(d.get("slu"), -1),
        bend: as_f64(d.get("bn"), 0.0),
        bend_intent: opt_i64(d.get("bt"), 0),
        bend_values: bend_curve(d.get("bnv")),
        flags,
        link_next: truthy(d.get("ln")),
        fret_hand_mute: truthy(d.get("fhm")),
        pluck: truthy(d.get("plk")),
        slap: truthy(d.get("slp")),
        right_hand: opt_i64(d.get("rh"), -1),
        pick_direction: opt_i64(d.get("pkd"), -1),
        fret_finger: opt_i64(d.get("fg"), -1),
        strum_group: opt_i64(d.get("ch"), -1),
        scale_degree: opt_i64(d.get("sd"), -1),
        ignore: truthy(d.get("ig")),
        hand: match d.get("hand").and_then(Value::as_str) {
            Some("lh") => Some("lh"),
            Some("rh") => Some("rh"),
            _ => None,
        },
    }
}

pub fn note_to_wire(n: &Note, with_time: bool) -> Map<String, Value> {
    let mut o = Map::new();
    if with_time {
        o.insert("t".into(), num(round_to(n.time, 3)));
    }
    o.insert("s".into(), json!(n.string));
    o.insert("f".into(), json!(n.fret));
    o.insert("sus".into(), num(round_to(n.sustain, 3)));
    o.insert("sl".into(), json!(n.slide_to));
    o.insert("slu".into(), json!(n.slide_unpitch_to));
    o.insert(
        "bn".into(),
        if n.bend != 0.0 {
            num(round_to(n.bend, 1))
        } else {
            json!(0)
        },
    );
    for (i, k) in FLAG_KEYS.iter().enumerate() {
        o.insert((*k).into(), json!(n.flags[i]));
    }
    if n.link_next {
        o.insert("ln".into(), json!(true));
    }
    if n.fret_hand_mute {
        o.insert("fhm".into(), json!(true));
    }
    if n.pluck {
        o.insert("plk".into(), json!(true));
    }
    if n.slap {
        o.insert("slp".into(), json!(true));
    }
    if n.right_hand != -1 {
        o.insert("rh".into(), json!(n.right_hand));
    }
    if n.pick_direction != -1 {
        o.insert("pkd".into(), json!(n.pick_direction));
    }
    if n.ignore {
        o.insert("ig".into(), json!(true));
    }
    if n.bend_intent != 0 {
        o.insert("bt".into(), json!(n.bend_intent));
    }
    if let Some(bv) = &n.bend_values {
        let pts: Vec<Value> = bv
            .iter()
            .map(|(t, v)| json!({"t": num(round_to(*t, 3)), "v": num(round_to(*v, 1))}))
            .collect();
        o.insert("bnv".into(), Value::Array(pts));
    }
    if n.fret_finger != -1 {
        o.insert("fg".into(), json!(n.fret_finger));
    }
    if n.strum_group != -1 {
        o.insert("ch".into(), json!(n.strum_group));
    }
    if n.scale_degree != -1 {
        o.insert("sd".into(), json!(n.scale_degree));
    }
    if let Some(h) = n.hand {
        o.insert("hand".into(), json!(h));
    }
    o
}

#[derive(Clone, Debug, Default)]
pub struct Chord {
    pub time: f64,
    chord_id: i64,
    pub notes: Vec<Note>,
    high_density: bool,
    func: Option<Value>,
}

fn validate_fn(raw: Option<&Value>) -> Option<Value> {
    let o = raw?.as_object()?;
    let rn = o.get("rn")?.as_str()?.trim();
    let q = o.get("q")?.as_str()?.trim();
    let deg = o.get("deg")?;
    if rn.is_empty() || q.is_empty() || deg.is_boolean() {
        return None;
    }
    let deg = deg.as_i64().filter(|d| (0..=11).contains(d))?;
    Some(json!({"rn": rn, "q": q, "deg": deg}))
}

fn chord_from_wire(d: &Value) -> Chord {
    let t = as_f64(d.get("t"), 0.0);
    Chord {
        time: t,
        chord_id: as_i64(d.get("id"), 0),
        high_density: truthy(d.get("hd")),
        notes: arr(d, "notes")
            .iter()
            .map(|n| note_from_wire(n, Some(t)))
            .collect(),
        func: validate_fn(d.get("fn")),
    }
}

fn chord_to_wire(c: &Chord) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("t".into(), num(round_to(c.time, 3)));
    o.insert("id".into(), json!(c.chord_id));
    o.insert("hd".into(), json!(c.high_density));
    o.insert(
        "notes".into(),
        Value::Array(
            c.notes
                .iter()
                .map(|n| Value::Object(note_to_wire(n, false)))
                .collect(),
        ),
    );
    if let Some(f) = &c.func {
        o.insert("fn".into(), f.clone());
    }
    o
}

#[derive(Clone, Debug)]
struct Anchor {
    time: f64,
    fret: i64,
    width: i64,
}

#[derive(Clone, Debug)]
struct HandShape {
    chord_id: i64,
    start: f64,
    end: f64,
    arp: bool,
}

#[derive(Clone, Debug)]
struct Template {
    name: Value,
    display_name: Value,
    arp: bool,
    fingers: Value,
    frets: Value,
    voicing: String,
    caged: String,
    guide_tones: Vec<i64>,
}

#[derive(Clone, Debug)]
struct Level {
    difficulty: i64,
    notes: Vec<Note>,
    chords: Vec<Chord>,
    anchors: Vec<Anchor>,
    hand_shapes: Vec<HandShape>,
}

#[derive(Clone, Debug)]
struct Phrase {
    start: f64,
    end: f64,
    max_difficulty: i64,
    levels: Vec<Level>,
}

#[derive(Clone, Debug, Default)]
pub struct Arrangement {
    pub name: String,
    pub tuning: Vec<Value>,
    pub capo: i64,
    pub notes: Vec<Note>,
    pub chords: Vec<Chord>,
    anchors: Vec<Anchor>,
    hand_shapes: Vec<HandShape>,
    templates: Vec<Template>,
    phrases: Option<Vec<Phrase>>,
    tones: Option<Map<String, Value>>,
    pub kind: String,
    cent_offset: f64,
    tempos: Option<Vec<Value>>,
}

fn arr<'a>(d: &'a Value, k: &str) -> &'a [Value] {
    d.get(k)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn anchors_from(d: &Value) -> Vec<Anchor> {
    arr(d, "anchors")
        .iter()
        .map(|a| Anchor {
            time: as_f64(a.get("time"), 0.0),
            fret: as_i64(a.get("fret"), 0),
            width: as_i64(a.get("width"), 4),
        })
        .collect()
}

fn hand_shapes_from(d: &Value) -> Vec<HandShape> {
    arr(d, "handshapes")
        .iter()
        .map(|h| HandShape {
            chord_id: as_i64(h.get("chord_id"), 0),
            start: as_f64(h.get("start_time"), 0.0),
            end: as_f64(h.get("end_time"), 0.0),
            arp: truthy(h.get("arp")),
        })
        .collect()
}

fn finite_or(v: Option<&Value>, default: f64) -> f64 {
    let f = as_f64(v, default);
    if f.is_finite() { f } else { default }
}

/// `sanitize_tempos`: finite time, finite bpm > 0, sorted.
pub fn sanitize_tempos(v: Option<&Value>) -> Vec<Value> {
    let mut out: Vec<(f64, f64)> = v
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|e| {
            let t = finite_num(e.get("time"))?;
            let bpm = finite_num(e.get("bpm")).filter(|b| *b > 0.0)?;
            Some((t, bpm))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter()
        .map(|(t, b)| json!({"time": num(t), "bpm": num(b)}))
        .collect()
}

fn sanitize_time_signatures(v: Option<&Value>) -> Vec<Value> {
    let mut out: Vec<(f64, i64, i64)> = v
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|e| {
            let t = finite_num(e.get("time"))?;
            let ts = e.get("ts")?.as_array()?;
            if ts.len() != 2 {
                return None;
            }
            let a = ts[0].as_i64().filter(|x| *x >= 1)?;
            let b = ts[1].as_i64().filter(|x| *x >= 1)?;
            Some((t, a, b))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter()
        .map(|(t, a, b)| json!({"time": num(t), "ts": [a, b]}))
        .collect()
}

fn template_from(ct: &Value) -> Template {
    let name = ct.get("name").cloned().unwrap_or(json!(""));
    Template {
        display_name: ct
            .get("displayName")
            .cloned()
            .unwrap_or_else(|| name.clone()),
        name,
        arp: truthy(ct.get("arp")),
        fingers: ct
            .get("fingers")
            .cloned()
            .unwrap_or_else(|| json!([-1, -1, -1, -1, -1, -1])),
        frets: ct
            .get("frets")
            .cloned()
            .unwrap_or_else(|| json!([-1, -1, -1, -1, -1, -1])),
        voicing: ct
            .get("voicing")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        caged: ct
            .get("caged")
            .and_then(Value::as_str)
            .filter(|c| ["C", "A", "G", "E", "D"].contains(c))
            .unwrap_or("")
            .to_string(),
        guide_tones: ct
            .get("guideTones")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|v| !v.is_boolean())
                    .filter_map(Value::as_i64)
                    .filter(|v| (0..=11).contains(v))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn template_to_wire(t: &Template) -> Value {
    let display = match &t.display_name {
        Value::String(s) if s.is_empty() => t.name.clone(),
        Value::Null => t.name.clone(),
        v => v.clone(),
    };
    let mut o = Map::new();
    o.insert("name".into(), t.name.clone());
    o.insert("displayName".into(), display);
    o.insert("arp".into(), json!(t.arp));
    o.insert("fingers".into(), t.fingers.clone());
    o.insert("frets".into(), t.frets.clone());
    if !t.voicing.is_empty() {
        o.insert("voicing".into(), json!(t.voicing));
    }
    if !t.caged.is_empty() {
        o.insert("caged".into(), json!(t.caged));
    }
    if !t.guide_tones.is_empty() {
        o.insert("guideTones".into(), json!(t.guide_tones));
    }
    Value::Object(o)
}

fn level_from(d: &Value) -> Level {
    Level {
        difficulty: as_i64(d.get("difficulty"), 0),
        notes: arr(d, "notes")
            .iter()
            .map(|n| note_from_wire(n, None))
            .collect(),
        chords: arr(d, "chords").iter().map(chord_from_wire).collect(),
        anchors: anchors_from(d),
        hand_shapes: hand_shapes_from(d),
    }
}

pub fn arrangement_from_wire(d: &Value) -> Arrangement {
    let phrases = arr(d, "phrases");
    Arrangement {
        name: py_str(d.get("name"), ""),
        tuning: d
            .get("tuning")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![json!(0); 6]),
        capo: as_i64(d.get("capo"), 0),
        cent_offset: finite_or(d.get("centOffset"), 0.0),
        tempos: Some(sanitize_tempos(d.get("tempos"))).filter(|t| !t.is_empty()),
        notes: arr(d, "notes")
            .iter()
            .map(|n| note_from_wire(n, None))
            .collect(),
        chords: arr(d, "chords").iter().map(chord_from_wire).collect(),
        anchors: anchors_from(d),
        hand_shapes: hand_shapes_from(d),
        templates: arr(d, "templates").iter().map(template_from).collect(),
        phrases: (!phrases.is_empty()).then(|| {
            phrases
                .iter()
                .map(|p| Phrase {
                    start: as_f64(p.get("start_time"), 0.0),
                    end: as_f64(p.get("end_time"), 0.0),
                    max_difficulty: as_i64(p.get("max_difficulty"), 0),
                    levels: arr(p, "levels").iter().map(level_from).collect(),
                })
                .collect()
        }),
        tones: d
            .get("tones")
            .and_then(Value::as_object)
            .filter(|t| !t.is_empty())
            .cloned(),
        kind: String::new(),
    }
}

fn anchor_wire(a: &Anchor) -> Value {
    json!({"time": num(a.time), "fret": a.fret, "width": a.width})
}

fn hand_shape_wire(h: &HandShape) -> Value {
    json!({"chord_id": h.chord_id, "start_time": num(h.start), "end_time": num(h.end), "arp": h.arp})
}

fn phrase_wire(p: &Phrase) -> Value {
    json!({
        "start_time": num(round_to(p.start, 3)),
        "end_time": num(round_to(p.end, 3)),
        "max_difficulty": p.max_difficulty,
        "levels": p.levels.iter().map(|l| json!({
            "difficulty": l.difficulty,
            "notes": l.notes.iter().map(|n| Value::Object(note_to_wire(n, true))).collect::<Vec<_>>(),
            "chords": l.chords.iter().map(|c| Value::Object(chord_to_wire(c))).collect::<Vec<_>>(),
            "anchors": l.anchors.iter().map(anchor_wire).collect::<Vec<_>>(),
            "handshapes": l.hand_shapes.iter().map(hand_shape_wire).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

// ── Arrangement helpers ─────────────────────────────────────────────────────

pub fn is_bass(a: &Arrangement) -> bool {
    a.kind.trim().eq_ignore_ascii_case("bass") || a.name.to_lowercase().contains("bass")
}

pub fn string_count(a: &Arrangement) -> i64 {
    let max_s = a
        .notes
        .iter()
        .map(|n| n.string)
        .chain(
            a.chords
                .iter()
                .flat_map(|c| c.notes.iter().map(|n| n.string)),
        )
        .max()
        .unwrap_or(-1);
    let notes_count = if max_s >= 0 { max_s + 1 } else { 0 };
    let name_based = if is_bass(a) { 4 } else { 6 };
    let tl = a.tuning.len() as i64;
    notes_count
        .max(name_based)
        .max(if tl != 6 { tl } else { 0 })
}

pub fn judgeable(a: &Arrangement) -> usize {
    a.notes.len() + a.chords.len()
}

pub fn last_event_time(a: &Arrangement) -> Option<f64> {
    a.notes
        .iter()
        .map(|n| n.time)
        .chain(a.chords.iter().map(|c| c.time))
        .reduce(f64::max)
}

/// Smart display names ("Lead", "Alt. Bass", "Bonus Rhythm"...). Packs carry
/// no XML path flags, so this is upstream's name-based fallback.
pub fn smart_names(names: &[String]) -> Vec<Option<String>> {
    let resolve = |name: &str| -> Option<(&'static str, Option<bool>)> {
        Some(match name.trim().to_lowercase().as_str() {
            "lead" | "combo" => ("Lead", None),
            "rhythm" => ("Rhythm", None),
            "bass" | "bass 2" => ("Bass", None),
            "bonus lead" => ("Lead", Some(true)),
            "bonus rhythm" => ("Rhythm", Some(true)),
            "bonus bass" => ("Bass", Some(true)),
            "alt. lead" => ("Lead", Some(false)),
            "alt. rhythm" => ("Rhythm", Some(false)),
            "alt. bass" => ("Bass", Some(false)),
            _ => return None,
        })
    };
    let resolved: Vec<Option<(&str, bool)>> = names
        .iter()
        .map(|n| resolve(n).map(|(l, b)| (l, b.unwrap_or(false))))
        .collect();
    let mut out = vec![None; names.len()];
    for label in ["Lead", "Rhythm", "Bass"] {
        let mains: Vec<usize> = (0..names.len())
            .filter(|i| matches!(resolved[*i], Some((l, false)) if l == label))
            .collect();
        // represent is always 0 for packs: the first main is standard, the rest alts.
        if let Some((&first, alts)) = mains.split_first() {
            out[first] = Some(label.to_string());
            for (j, &i) in alts.iter().enumerate() {
                out[i] = Some(if alts.len() == 1 {
                    format!("Alt. {label}")
                } else {
                    format!("Alt. {label} {}", j + 1)
                });
            }
        }
        let bonus: Vec<usize> = (0..names.len())
            .filter(|i| matches!(resolved[*i], Some((l, true)) if l == label))
            .collect();
        for (j, &i) in bonus.iter().enumerate() {
            out[i] = Some(if bonus.len() == 1 {
                format!("Bonus {label}")
            } else {
                format!("Bonus {label} {}", j + 1)
            });
        }
    }
    out
}

/// `_arr_smart_sort_key`.
pub fn smart_sort_key(sn: Option<&str>) -> (i64, i64) {
    let Some(sn) = sn.filter(|s| !s.is_empty()) else {
        return (99, 0);
    };
    for (label, base) in [("Lead", 0), ("Rhythm", 10), ("Bass", 20)] {
        if sn == label {
            return (base, 0);
        }
        for (prefix, add) in [(format!("Alt. {label}"), 1), (format!("Bonus {label}"), 2)] {
            if sn == prefix {
                return (base + add, 0);
            }
            if let Some(rest) = sn.strip_prefix(&format!("{prefix} ")) {
                return (base + add, rest.parse().unwrap_or(0));
            }
        }
    }
    (99, 0)
}

// ── Loading a pack ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Stem {
    pub id: String,
    pub file: String,
    pub default: bool,
    pub extra: Map<String, Value>,
}

pub struct Loaded {
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub beats: Vec<Value>,
    pub sections: Vec<Value>,
    pub arrangements: Vec<Arrangement>,
    pub lyrics: Vec<Value>,
    pub lyrics_source: String,
    pub stems: Vec<Stem>,
    pub full_mix: Option<String>,
    pub manifest: Value,
    pub tempos: Option<Vec<Value>>,
    pub time_sigs: Option<Vec<Value>>,
    pub keys: Option<(i64, Vec<Value>)>,
}

pub fn stem_default_on(v: Option<&Value>) -> bool {
    match v {
        None => true,
        Some(Value::String(s)) => !["off", "false", "0", "no"].contains(&s.to_lowercase().as_str()),
        other => truthy(other),
    }
}

/// Stems from the manifest, with the reserved `full` mixdown lifted out when
/// the pack also has instrument stems (spec §5.3).
pub fn manifest_stems(manifest: &Value) -> (Option<Stem>, Vec<Stem>) {
    let stems: Vec<Stem> = arr(manifest, "stems")
        .iter()
        .filter_map(|s| {
            s.as_object()?;
            let id = py_str(s.get("id"), "");
            let file = py_str(s.get("file"), "");
            if id.is_empty() || file.is_empty() {
                return None;
            }
            let mut extra = Map::new();
            for k in ["name", "description"] {
                if let Some(v) = s
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|v| !v.trim().is_empty())
                {
                    extra.insert(k.into(), json!(v));
                }
            }
            Some(Stem {
                id,
                file,
                default: stem_default_on(s.get("default")),
                extra,
            })
        })
        .collect();
    if stems.len() < 2 {
        return (None, stems);
    }
    match stems.iter().find(|s| s.id == "full").cloned() {
        Some(full) => (
            Some(full),
            stems.into_iter().filter(|s| s.id != "full").collect(),
        ),
        None => (None, stems),
    }
}

pub fn load(source_dir: &Path, manifest: Value) -> Loaded {
    let mut beats: Vec<Value> = Vec::new();
    let mut sections: Vec<Value> = Vec::new();
    let mut arrangements = Vec::new();
    for entry in arr(&manifest, "arrangements") {
        if !entry.is_object() {
            continue;
        }
        let rel = entry
            .get("file")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        let etype = py_str(entry.get("type"), "").trim().to_lowercase();
        let has_notation = entry
            .get("notation")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        if etype == "drums" || etype == "drum" || (rel.is_empty() && !has_notation) {
            continue; // drum parts / pointer entries: not served by RiffBoard
        }
        let data = if rel.is_empty() {
            None
        } else {
            match pack::read_pack_json(source_dir, rel) {
                Some(d) => Some(d),
                None => continue,
            }
        };
        let mut a = arrangement_from_wire(data.as_ref().unwrap_or(&json!({})));
        if truthy(entry.get("name")) {
            a.name = py_str(entry.get("name"), "");
        }
        if truthy(entry.get("type")) {
            a.kind = etype.clone();
        }
        if let Some(t) = entry.get("tuning") {
            a.tuning = t.as_array().cloned().unwrap_or_default();
        }
        if entry.get("capo").is_some() {
            a.capo = as_i64(entry.get("capo"), 0);
        }
        if entry.get("centOffset").is_some() {
            a.cent_offset = finite_or(entry.get("centOffset"), 0.0);
        }
        if let Some(t) = entry
            .get("tones")
            .and_then(Value::as_object)
            .filter(|t| !t.is_empty())
        {
            a.tones = Some(t.clone());
        }
        if let Some(d) = &data {
            if beats.is_empty() {
                beats = arr(d, "beats").iter().map(|b| json!({
                    "time": num(as_f64(b.get("time"), 0.0)), "measure": as_i64(b.get("measure"), -1)})).collect();
            }
            if sections.is_empty() {
                sections = arr(d, "sections")
                    .iter()
                    .map(|s| {
                        json!({
                    "name": py_str(s.get("name"), ""),
                    "time": num(as_f64(s.get("time").or_else(|| s.get("start_time")), 0.0))})
                    })
                    .collect();
            }
        }
        arrangements.push(a);
    }

    let mut tempos = None;
    let mut time_sigs = None;
    if let Some(rel) = manifest
        .get("song_timeline")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        && let Some(raw) = pack::read_pack_json(source_dir, rel).filter(Value::is_object)
    {
        if raw.get("beats").is_some_and(Value::is_array)
            && raw.get("sections").is_some_and(Value::is_array)
        {
            beats = arr(&raw, "beats").iter().filter(|b| b.is_object()).map(|b| json!({
                    "time": num(finite_or(b.get("time"), 0.0)), "measure": as_i64(b.get("measure"), -1)})).collect();
            sections = arr(&raw, "sections")
                .iter()
                .filter(|s| s.is_object())
                .map(|s| {
                    json!({
                    "name": py_str(s.get("name"), ""),
                    "time": num(finite_or(s.get("time").or_else(|| s.get("start_time")), 0.0))})
                })
                .collect();
        }
        tempos = Some(sanitize_tempos(raw.get("tempos"))).filter(|t| !t.is_empty());
        time_sigs =
            Some(sanitize_time_signatures(raw.get("time_signatures"))).filter(|t| !t.is_empty());
    }

    let mut lyrics = Vec::new();
    let mut lyrics_source = String::new();
    if let Some(rel) = manifest
        .get("lyrics")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        && let Some(Value::Array(raw)) = pack::read_pack_json(source_dir, rel)
    {
        lyrics = raw
            .into_iter()
            .filter(|e| {
                e.get("w").is_some_and(Value::is_string)
                    && e.get("t").is_some_and(Value::is_number)
                    && e.get("d").is_some_and(Value::is_number)
            })
            .collect();
        if !lyrics.is_empty() {
            let src = match manifest.get("lyrics_source").and_then(Value::as_str) {
                Some("sng") => "notechart",
                Some("whisperx") => "transcribed",
                Some(s) => s,
                None => "",
            };
            lyrics_source =
                if ["xml", "notechart", "user", "authored", "transcribed"].contains(&src) {
                    src.to_string()
                } else {
                    "xml".into()
                };
        }
    }

    let (full_stem, stems) = manifest_stems(&manifest);
    let full_mix = match full_stem {
        Some(f) => Some(f.file),
        None if stems.iter().any(|s| s.id == "full") => None,
        None => manifest
            .get("original_audio")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|r| {
                !r.is_empty() && pack::resolve_in(source_dir, r).is_some_and(|p| p.is_file())
            })
            .map(String::from),
    };

    let mut keys = None;
    if let Some(rel) = manifest
        .get("keys")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        && let Some(raw) = pack::read_pack_json(source_dir, rel).filter(Value::is_object)
        && let Some(events) = raw.get("events").and_then(Value::as_array)
    {
        let mut clean: Vec<(f64, Value)> = events
            .iter()
            .filter_map(|e| {
                let t = finite_num(e.get("t"))?;
                let key = e.get("key")?.as_str().filter(|k| !k.is_empty())?;
                let mut o = json!({"t": num(t), "key": key});
                if let Some(s) = e
                    .get("scale")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    o["scale"] = json!(s);
                }
                Some((t, o))
            })
            .collect();
        clean.sort_by(|a, b| a.0.total_cmp(&b.0));
        let ver = raw
            .get("version")
            .filter(|v| !v.is_boolean())
            .and_then(Value::as_i64)
            .unwrap_or(1);
        keys = Some((ver, clean.into_iter().map(|(_, v)| v).collect()));
    }

    Loaded {
        title: py_str(manifest.get("title"), ""),
        artist: py_str(manifest.get("artist"), ""),
        duration: as_f64(manifest.get("duration"), 0.0),
        beats,
        sections,
        arrangements,
        lyrics,
        lyrics_source,
        stems,
        full_mix,
        tempos,
        time_sigs,
        keys,
        manifest,
    }
}

// ── Highway messages ────────────────────────────────────────────────────────

fn authors(manifest: &Value) -> Vec<Value> {
    arr(manifest, "authors")
        .iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?.trim();
            if name.is_empty() {
                return None;
            }
            let role = e
                .get("role")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|r| !r.is_empty());
            Some(json!({"name": name, "role": role}))
        })
        .collect()
}

fn key_tonic(key: &str) -> Option<i64> {
    let s = key.trim();
    let mut chars = s.chars();
    let mut pc: i64 = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    for ch in chars {
        match ch {
            '#' | '♯' => pc += 1,
            'b' | '♭' => pc -= 1,
            _ => break,
        }
    }
    Some(pc.rem_euclid(12))
}

fn base_midis(sc: i64, bass: bool) -> &'static [i64] {
    match (sc, bass) {
        (4, true) => &[28, 33, 38, 43],
        (5, true) => &[23, 28, 33, 38, 43],
        (7, _) => &[35, 40, 45, 50, 55, 59, 64],
        (8, _) => &[30, 35, 40, 45, 50, 55, 59, 64],
        _ => &[40, 45, 50, 55, 59, 64],
    }
}

pub struct Selection {
    pub index: usize,
    pub smart: Vec<Option<String>>,
}

/// Which arrangement plays: explicit request > operator preference > most notes.
pub fn select(song: &Loaded, requested: i64, naming_mode: &str, cfg: &Value) -> Selection {
    let names: Vec<String> = song.arrangements.iter().map(|a| a.name.clone()).collect();
    let smart = smart_names(&names);
    let n = song.arrangements.len();
    if requested >= 0 && (requested as usize) < n {
        return Selection {
            index: requested as usize,
            smart,
        };
    }
    let pref = cfg
        .get("default_arrangement")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let instrument = cfg
        .get("instrument")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let label = |i: usize| -> String {
        if naming_mode == "smart" {
            smart[i].clone().unwrap_or_default()
        } else {
            names[i].clone()
        }
    };
    if instrument == "bass" {
        let bass: Vec<usize> = (0..n)
            .filter(|&i| {
                is_bass(&song.arrangements[i])
                    || smart[i]
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .starts_with("bass")
            })
            .collect();
        if !bass.is_empty() {
            if !pref.is_empty()
                && let Some(&i) = bass.iter().find(|&&i| label(i) == pref)
            {
                return Selection { index: i, smart };
            }
            let i = bass
                .iter()
                .copied()
                .find(|&i| smart[i].as_deref() == Some("Bass"))
                .unwrap_or(bass[0]);
            return Selection { index: i, smart };
        }
    }
    if !pref.is_empty() && pref.to_lowercase() != "auto" {
        if naming_mode == "smart" {
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by_key(|&i| smart_sort_key(smart[i].as_deref()));
            let alt = format!("Alt. {pref}");
            let bonus = format!("Bonus {pref}");
            let pick = order
                .iter()
                .copied()
                .find(|&i| smart[i].as_deref() == Some(pref.as_str()))
                .or_else(|| {
                    order.iter().copied().find(|&i| {
                        smart[i]
                            .as_deref()
                            .is_some_and(|s| s == alt || s.starts_with(&format!("{alt} ")))
                    })
                })
                .or_else(|| {
                    order.iter().copied().find(|&i| {
                        smart[i]
                            .as_deref()
                            .is_some_and(|s| s == bonus || s.starts_with(&format!("{bonus} ")))
                    })
                })
                .or_else(|| order.first().copied());
            if let Some(i) = pick {
                return Selection { index: i, smart };
            }
        } else if let Some(i) = (0..n).find(|&i| names[i] == pref) {
            return Selection { index: i, smart };
        }
    }
    let mut best = 0;
    let mut best_count = 0;
    for (i, a) in song.arrangements.iter().enumerate() {
        let c = a.notes.len() + a.chords.iter().map(|c| c.notes.len()).sum::<usize>();
        if c > best_count {
            best_count = c;
            best = i;
        }
    }
    Selection { index: best, smart }
}

fn chunked(kind: &str, items: Vec<Value>, size: usize, out: &mut Vec<Value>) {
    let total = items.len();
    for chunk in items.chunks(size) {
        out.push(json!({"type": kind, "data": chunk, "total": total}));
    }
}

/// Every message the highway WebSocket sends after `loading`, in order.
pub fn highway_messages(
    song: &Loaded,
    sel: &Selection,
    url_name: &str,
    naming_mode: &str,
) -> Vec<Value> {
    let best = sel.index;
    let a = &song.arrangements[best];
    let q_fn =
        percent_encoding::utf8_percent_encode(url_name, crate::util_url::COMPONENT).to_string();
    let file_url = |rel: &str| {
        format!(
            "/api/sloppak/{q_fn}/file/{}",
            percent_encoding::utf8_percent_encode(rel, crate::util_url::PATH)
        )
    };
    let stems: Vec<Value> = song
        .stems
        .iter()
        .map(|s| {
            let mut o = Map::new();
            o.insert("id".into(), json!(s.id));
            o.insert("url".into(), json!(file_url(&s.file)));
            o.insert("default".into(), json!(s.default));
            o.extend(s.extra.clone());
            Value::Object(o)
        })
        .collect();
    let full_mix_url = song.full_mix.as_deref().map(file_url);
    let (audio_url, audio_error) = if let Some(first) = stems.first() {
        (first["url"].clone(), Value::Null)
    } else if let Some(f) = &full_mix_url {
        (json!(f), Value::Null)
    } else {
        (Value::Null, json!("This sloppak has no playable stems."))
    };
    let mut arr_list: Vec<Value> = song
        .arrangements
        .iter()
        .enumerate()
        .map(|(i, x)| {
            json!({
                "index": i, "name": x.name, "smart_name": sel.smart[i],
                "notes": x.notes.len() + x.chords.iter().map(|c| c.notes.len()).sum::<usize>(),
            })
        })
        .collect();
    arr_list.sort_by_key(|v| smart_sort_key(v["smart_name"].as_str()));

    let mut out = vec![json!({
        "type": "song_info",
        "title": song.title,
        "artist": song.artist,
        "duration": num(song.duration),
        "arrangement": a.name,
        "arrangement_smart_name": sel.smart[best],
        "arrangement_index": best,
        "naming_mode": if naming_mode == "smart" { "smart" } else { "legacy" },
        "arrangements": arr_list,
        "audio_url": audio_url,
        "audio_error": audio_error,
        "tuning": a.tuning,
        "stringCount": string_count(a),
        "capo": a.capo,
        "centOffset": num(a.cent_offset),
        "offset": num(0.0),
        "format": "sloppak",
        "authors": authors(&song.manifest),
        "stems": stems,
        "full_mix_url": full_mix_url,
        "has_full_mix": full_mix_url.is_some(),
        "has_stems": !song.stems.is_empty(),
        "original_audio_url": full_mix_url,
        "has_original_audio": full_mix_url.is_some(),
        "has_drum_tab": false,
        "drum_parts": [],
        "has_notation": false,
        "has_keys": song.keys.is_some(),
    })];
    out.push(json!({"type": "beats", "data": song.beats}));
    out.push(json!({"type": "sections", "data": song.sections}));
    if let Some((ver, events)) = &song.keys {
        out.push(json!({"type": "keys", "version": ver, "data": events}));
    }
    if let Some(t) = a.tempos.as_ref().or(song.tempos.as_ref()) {
        out.push(json!({"type": "tempos", "data": t}));
    }
    if let Some(ts) = &song.time_sigs {
        out.push(json!({"type": "time_signatures", "data": ts}));
    }
    out.push(
        json!({"type": "anchors", "data": a.anchors.iter().map(anchor_wire).collect::<Vec<_>>()}),
    );
    out.push(json!({"type": "chord_templates", "data": a.templates.iter().map(template_to_wire).collect::<Vec<_>>()}));
    if !song.lyrics.is_empty() {
        let mut m = json!({"type": "lyrics", "data": song.lyrics});
        if !song.lyrics_source.is_empty() {
            m["source"] = json!(song.lyrics_source);
        }
        out.push(m);
    }
    if let Some(tones) = &a.tones {
        let base = tones
            .get("base")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let base_rig = tones
            .get("base_rig")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let mut changes: Vec<(f64, Value)> = tones
            .get("changes")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter_map(|c| {
                let name = c.get("name")?.as_str().filter(|n| !n.is_empty())?;
                let t = match c.get("t")? {
                    Value::Number(n) => n.as_f64()?,
                    Value::String(s) => s.trim().parse().ok()?,
                    Value::Bool(b) => f64::from(u8::from(*b)),
                    _ => return None,
                };
                if !t.is_finite() {
                    return None;
                }
                let mut o = json!({"t": num(round_to(t, 3)), "name": name});
                if let Some(rig) = c
                    .get("rig")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                {
                    o["rig"] = json!(rig);
                }
                Some((round_to(t, 3), o))
            })
            .collect();
        changes.sort_by(|x, y| x.0.total_cmp(&y.0));
        if !changes.is_empty() || !base.is_empty() {
            let mut m = json!({"type": "tone_changes", "base": base,
                               "data": changes.into_iter().map(|c| c.1).collect::<Vec<_>>()});
            if !base_rig.is_empty() {
                m["base_rig"] = json!(base_rig);
            }
            out.push(m);
        }
    }

    // Scale degrees (teaching marks) from the key track, when not authored.
    let key_events: Vec<(f64, Option<i64>)> = song
        .keys
        .as_ref()
        .map(|(_, ev)| {
            ev.iter()
                .map(|e| {
                    (
                        e["t"].as_f64().unwrap_or(0.0),
                        e["key"].as_str().and_then(key_tonic),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let tuning: Vec<i64> = a.tuning.iter().map(|v| as_i64(Some(v), 0)).collect();
    let base = base_midis(string_count(a), is_bass(a));
    let fill_sd = |w: &mut Map<String, Value>, n: &Note, t: f64| {
        if w.contains_key("sd") || key_events.is_empty() {
            return;
        }
        let idx = key_events.partition_point(|(kt, _)| *kt <= t);
        if idx == 0 {
            return;
        }
        let Some(tonic) = key_events[idx - 1].1 else {
            return;
        };
        if n.string < 0 || n.string as usize >= tuning.len() || base.is_empty() {
            return;
        }
        let root = base
            .get(n.string as usize)
            .copied()
            .unwrap_or(*base.last().unwrap());
        let midi = root + tuning[n.string as usize] + a.capo + n.fret;
        w.insert("sd".into(), json!((midi - tonic).rem_euclid(12)));
    };
    let notes: Vec<Value> = a
        .notes
        .iter()
        .map(|n| {
            let mut w = note_to_wire(n, true);
            fill_sd(&mut w, n, n.time);
            Value::Object(w)
        })
        .collect();
    chunked("notes", notes, 500, &mut out);
    let chords: Vec<Value> = a
        .chords
        .iter()
        .map(|c| {
            let mut w = chord_to_wire(c);
            if let Some(Value::Array(ns)) = w.get_mut("notes") {
                for (cn, nw) in c.notes.iter().zip(ns.iter_mut()) {
                    if let Value::Object(m) = nw {
                        fill_sd(m, cn, c.time);
                    }
                }
            }
            Value::Object(w)
        })
        .collect();
    chunked("chords", chords, 500, &mut out);
    chunked(
        "handshapes",
        a.hand_shapes.iter().map(hand_shape_wire).collect(),
        500,
        &mut out,
    );
    if let Some(ph) = &a.phrases {
        chunked(
            "phrases",
            ph.iter().map(phrase_wire).collect(),
            20,
            &mut out,
        );
    }
    out.push(json!({"type": "ready"}));
    out
}
