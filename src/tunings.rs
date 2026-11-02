//! Tuning names, presets, per-role "perspectives" and instrument profiles.
//! Ported from upstream lib/tunings.py.

use serde_json::{Map, Value, json};

use crate::util::{num, round_to};

pub const DEFAULT_REFERENCE_PITCH: f64 = 440.0;

pub fn standard_open_midis(key: &str) -> Option<&'static [i64]> {
    Some(match key {
        "guitar-6" => &[40, 45, 50, 55, 59, 64],
        "guitar-7" => &[35, 40, 45, 50, 55, 59, 64],
        "guitar-8" => &[30, 35, 40, 45, 50, 55, 59, 64],
        "bass-4" => &[28, 33, 38, 43],
        "bass-5" => &[23, 28, 33, 38, 43],
        "bass-6" => &[23, 28, 33, 38, 43, 48],
        _ => return None,
    })
}

type Presets = &'static [(&'static str, &'static [i64])];

pub const PRESETS: &[(&str, Presets)] = &[
    (
        "guitar-6",
        &[
            ("Standard", &[40, 45, 50, 55, 59, 64]),
            ("Eb Standard", &[39, 44, 49, 54, 58, 63]),
            ("D Standard", &[38, 43, 48, 53, 57, 62]),
            ("C# Standard", &[37, 42, 47, 52, 56, 61]),
            ("C Standard", &[36, 41, 46, 51, 55, 60]),
            ("Drop D", &[38, 45, 50, 55, 59, 64]),
            ("Drop C", &[36, 43, 48, 53, 57, 62]),
            ("Drop B", &[35, 42, 47, 52, 56, 61]),
            ("Drop A", &[33, 40, 45, 50, 54, 59]),
            ("Drop Ab", &[32, 39, 44, 49, 53, 58]),
            ("Open G", &[38, 43, 50, 55, 59, 62]),
            ("Open D", &[38, 45, 50, 54, 57, 62]),
            ("DADGAD", &[38, 45, 50, 55, 57, 62]),
            ("Open E", &[40, 47, 52, 56, 59, 64]),
        ],
    ),
    (
        "guitar-7",
        &[
            ("Standard", &[35, 40, 45, 50, 55, 59, 64]),
            ("Bb Standard", &[34, 39, 44, 49, 54, 58, 63]),
            ("A Standard", &[33, 38, 43, 48, 53, 57, 62]),
            ("G Standard", &[31, 36, 41, 46, 51, 55, 60]),
            ("Drop A", &[33, 40, 45, 50, 55, 59, 64]),
            ("Drop G", &[31, 38, 43, 48, 53, 57, 62]),
            ("Drop F#", &[30, 37, 42, 47, 52, 56, 61]),
        ],
    ),
    (
        "guitar-8",
        &[
            ("Standard", &[30, 35, 40, 45, 50, 55, 59, 64]),
            ("Drop E", &[28, 35, 40, 45, 50, 55, 59, 64]),
            ("Drop A + Drop E", &[28, 33, 40, 45, 50, 55, 59, 64]),
            ("E Standard", &[28, 33, 38, 43, 48, 53, 57, 62]),
            ("Eb Standard", &[27, 32, 37, 42, 47, 52, 56, 61]),
            ("Drop D", &[26, 33, 38, 43, 48, 53, 57, 62]),
        ],
    ),
    (
        "bass-4",
        &[
            ("Standard", &[28, 33, 38, 43]),
            ("Eb Standard", &[27, 32, 37, 42]),
            ("D Standard", &[26, 31, 36, 41]),
            ("C# Standard", &[25, 30, 35, 40]),
            ("C Standard", &[24, 29, 34, 39]),
            ("Drop D", &[26, 33, 38, 43]),
            ("Drop C", &[24, 31, 36, 41]),
            ("BEAD", &[23, 28, 33, 38]),
        ],
    ),
    (
        "bass-5",
        &[
            ("Standard", &[23, 28, 33, 38, 43]),
            ("High C", &[28, 33, 38, 43, 48]),
            ("Eb Standard", &[22, 27, 32, 37, 42]),
            ("D Standard", &[21, 26, 31, 36, 41]),
            ("C# Standard", &[20, 25, 30, 35, 40]),
            ("C Standard", &[19, 24, 29, 34, 39]),
            ("Drop A", &[21, 28, 33, 38, 43]),
        ],
    ),
    (
        "bass-6",
        &[
            ("Standard", &[23, 28, 33, 38, 43, 48]),
            ("Eb Standard", &[22, 27, 32, 37, 42, 47]),
            ("D Standard", &[21, 26, 31, 36, 41, 46]),
            ("C# Standard", &[20, 25, 30, 35, 40, 45]),
            ("C Standard", &[19, 24, 29, 34, 39, 44]),
        ],
    ),
];

fn presets_for(key: &str) -> Option<Presets> {
    PRESETS.iter().find(|(k, _)| *k == key).map(|(_, p)| *p)
}

fn midi_to_freq(midi: i64, reference: f64) -> f64 {
    reference * 2f64.powf((midi as f64 - 69.0) / 12.0)
}

/// `GET /api/tunings` payload.
pub fn tunings_payload(reference: f64) -> Value {
    let scale = reference / DEFAULT_REFERENCE_PITCH;
    let mut tunings = Map::new();
    let mut midis = Map::new();
    for (key, presets) in PRESETS {
        let mut t = Map::new();
        let mut m = Map::new();
        for (name, notes) in *presets {
            let freqs: Vec<Value> = notes
                .iter()
                .map(|n| {
                    num(round_to(
                        round_to(midi_to_freq(*n, DEFAULT_REFERENCE_PITCH), 2) * scale,
                        4,
                    ))
                })
                .collect();
            t.insert((*name).into(), Value::Array(freqs));
            m.insert((*name).into(), json!(notes));
        }
        tunings.insert((*key).into(), Value::Object(t));
        midis.insert((*key).into(), Value::Object(m));
    }
    json!({"referencePitch": reference, "tunings": tunings, "tuningMidis": midis})
}

pub fn tuning_name(offsets: &[i64]) -> String {
    let n = offsets.len();
    if (n == 4 || n == 6) && offsets.iter().all(|o| *o == offsets[0]) {
        let name = match offsets[0] {
            0 => Some("E Standard"),
            -1 => Some("Eb Standard"),
            -2 => Some("D Standard"),
            -3 => Some("C# Standard"),
            -4 => Some("C Standard"),
            -5 => Some("B Standard"),
            -6 => Some("Bb Standard"),
            -7 => Some("A Standard"),
            1 => Some("F Standard"),
            2 => Some("F# Standard"),
            _ => None,
        };
        if let Some(name) = name {
            return name.into();
        }
    }
    if (n == 4 || n == 6)
        && offsets[0] == offsets[1] - 2
        && offsets[1..].iter().all(|o| *o == offsets[1])
    {
        const NOTES: [&str; 12] = [
            "E", "F", "F#", "G", "Ab", "A", "Bb", "B", "C", "C#", "D", "Eb",
        ];
        return format!("Drop {}", NOTES[offsets[0].rem_euclid(12) as usize]);
    }
    if n == 6 {
        let named: &[([i64; 6], &str)] = &[
            ([-2, 0, 0, 0, 0, 0], "Drop D"),
            ([-4, -2, -2, -2, -2, -2], "Drop C"),
            ([-2, -2, 0, 0, 0, 0], "Double Drop D"),
            ([0, 0, 0, -1, 0, 0], "Open G"),
            ([-2, -2, 0, 0, -2, -2], "Open D"),
            ([-2, 0, 0, 0, -2, 0], "DADGAD"),
            ([0, 2, 2, 1, 0, 0], "Open E"),
            ([-2, 0, 0, 2, 3, 2], "Open D (alt)"),
        ];
        if let Some((_, name)) = named.iter().find(|(o, _)| o[..] == offsets[..]) {
            return (*name).into();
        }
    }
    if offsets.is_empty() {
        "Unknown".into()
    } else {
        "Custom Tuning".into()
    }
}

// ── Perspectives (lead / rhythm / bass) ──────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Perspective {
    pub id: &'static str,
    pub role: &'static str,
    pub instrument: &'static str,
    pub string_count: usize,
    pub prefix: &'static str,
    pub truncate: bool,
    pub guard_up: bool,
}

pub const LEAD: Perspective = Perspective {
    id: "guitar-lead",
    role: "",
    instrument: "guitar",
    string_count: 6,
    prefix: "",
    truncate: false,
    guard_up: false,
};
pub const RHYTHM: Perspective = Perspective {
    id: "guitar-rhythm",
    role: "rhythm",
    instrument: "guitar",
    string_count: 6,
    prefix: "rhythm_",
    truncate: false,
    guard_up: false,
};
pub const BASS: Perspective = Perspective {
    id: "bass",
    role: "bass",
    instrument: "bass",
    string_count: 4,
    prefix: "bass_",
    truncate: true,
    guard_up: true,
};

pub fn perspective(id: &str) -> Perspective {
    match id {
        "guitar-rhythm" => RHYTHM,
        "bass" => BASS,
        _ => LEAD,
    }
}

impl Perspective {
    pub fn instrument_key(&self) -> String {
        format!("{}-{}", self.instrument, self.string_count)
    }
}

pub fn offsets_from_value(v: &Value) -> Option<Vec<i64>> {
    let arr = v.as_array()?;
    if arr.is_empty() {
        return None;
    }
    arr.iter()
        .map(|o| match o {
            Value::Bool(_) => None,
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        })
        .collect()
}

pub fn normalize_offsets(raw: &Value, p: &Perspective) -> Option<Vec<i64>> {
    let vals = offsets_from_value(raw)?;
    if vals.len() < p.string_count {
        return None;
    }
    Some(if p.truncate {
        vals[..p.string_count].to_vec()
    } else {
        vals
    })
}

pub fn perspective_tuning_name(offsets: &[i64], p: &Perspective) -> String {
    if p.guard_up && offsets.iter().any(|o| *o > 1) {
        return "Custom Tuning".into();
    }
    tuning_name(offsets)
}

fn midis_from_offsets(key: &str, offsets: &[i64]) -> Option<Vec<i64>> {
    let std = standard_open_midis(key)?;
    (std.len() == offsets.len()).then(|| std.iter().zip(offsets).map(|(s, o)| s + o).collect())
}

pub fn perspective_tuning_key(offsets: &[i64], p: &Perspective) -> String {
    match midis_from_offsets(&p.instrument_key(), offsets) {
        Some(m) if !m.is_empty() => {
            let parts: Vec<String> = m.iter().map(i64::to_string).collect();
            format!("{}:{}", p.id, parts.join(":"))
        }
        _ => String::new(),
    }
}

pub fn perspective_low_pitch(offsets: &[i64], p: &Perspective) -> Option<i64> {
    midis_from_offsets(&p.instrument_key(), offsets)?
        .into_iter()
        .min()
}

pub fn offsets_str(offsets: &[i64]) -> String {
    offsets
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Instrument profiles (settings) ───────────────────────────────────────────

pub const PROFILE_IDS: [&str; 3] = ["guitar-lead", "guitar-rhythm", "bass"];
pub const PROFILE_PATHWAYS: [&str; 4] = ["songs", "practice", "learn", "studio"];

pub fn profile_default(id: &str) -> Option<Map<String, Value>> {
    let (label, instrument, role, sc) = match id {
        "guitar-lead" => ("Lead Guitar", "guitar", "lead", 6),
        "guitar-rhythm" => ("Rhythm Guitar", "guitar", "rhythm", 6),
        "bass" => ("Bass", "bass", "bass", 4),
        _ => return None,
    };
    let v = json!({
        "id": id, "label": label, "instrument": instrument, "role": role,
        "string_count": sc, "tuning": "Standard",
        "reference_pitch": DEFAULT_REFERENCE_PITCH, "pathway": "songs",
    });
    v.as_object().cloned()
}

fn default_profiles() -> Map<String, Value> {
    PROFILE_IDS
        .iter()
        .map(|id| {
            (
                (*id).to_string(),
                Value::Object(profile_default(id).unwrap()),
            )
        })
        .collect()
}

fn valid_reference_pitch(v: &Value) -> Option<f64> {
    let f = match v {
        Value::Bool(_) => return None,
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (f.is_finite() && (430.0..=450.0).contains(&f)).then_some(f)
}

fn valid_tuning_for_key(key: &str, tuning: &Value) -> Option<Value> {
    match tuning {
        Value::String(s) => {
            if s.chars().count() > 64 {
                return None;
            }
            if presets_for(key).is_some_and(|p| p.iter().any(|(n, _)| n == s)) {
                return Some(tuning.clone());
            }
            if PRESETS.iter().any(|(_, p)| p.iter().any(|(n, _)| n == s)) {
                return None;
            }
            Some(tuning.clone())
        }
        Value::Array(a) => {
            let expected = standard_open_midis(key).map_or(0, <[i64]>::len);
            if a.len() != expected {
                return None;
            }
            let ok = a
                .iter()
                .all(|o| o.as_i64().is_some_and(|x| (-12..=12).contains(&x)) && !o.is_boolean());
            ok.then(|| tuning.clone())
        }
        _ => None,
    }
}

fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite())
                .map(|f| f.trunc() as i64)
        }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn normalize_profile(id: &str, raw: Option<&Value>) -> Result<Map<String, Value>, String> {
    let base = profile_default(id).ok_or_else(|| format!("unknown instrument profile: {id}"))?;
    let raw = match raw {
        None | Some(Value::Null) => return Ok(base),
        Some(Value::Object(o)) => o,
        Some(_) => return Err(format!("instrument_profiles.{id} must be an object")),
    };
    let get = |k: &str| raw.get(k).unwrap_or(&base[k]).clone();
    let instrument = get("instrument");
    let inst = instrument
        .as_str()
        .filter(|s| *s == "guitar" || *s == "bass")
        .ok_or_else(|| format!("instrument_profiles.{id}.instrument must be 'guitar' or 'bass'"))?
        .to_string();
    let sc = py_int(&get("string_count")).ok_or_else(|| {
        format!("instrument_profiles.{id}.string_count must be valid for the instrument")
    })?;
    let key = format!("{inst}-{sc}");
    if standard_open_midis(&key).is_none() {
        return Err(format!(
            "instrument_profiles.{id}.string_count must be valid for the instrument"
        ));
    }
    let tuning = valid_tuning_for_key(&key, &get("tuning"))
        .ok_or_else(|| format!("instrument_profiles.{id}.tuning must match {key}"))?;
    let rp = valid_reference_pitch(&get("reference_pitch")).ok_or_else(|| {
        format!("instrument_profiles.{id}.reference_pitch must be a number between 430 and 450")
    })?;
    let label = get("label");
    if !label.as_str().is_some_and(|s| s.chars().count() <= 64) {
        return Err(format!(
            "instrument_profiles.{id}.label must be a short string"
        ));
    }
    let role = get("role");
    if !role.as_str().is_some_and(|s| s.chars().count() <= 32) {
        return Err(format!(
            "instrument_profiles.{id}.role must be a short string"
        ));
    }
    let pathway = get("pathway");
    if !pathway
        .as_str()
        .is_some_and(|s| PROFILE_PATHWAYS.contains(&s))
    {
        return Err(format!(
            "instrument_profiles.{id}.pathway must be one of songs, practice, learn, studio"
        ));
    }
    let mut out = base;
    out.insert("id".into(), json!(id));
    out.insert("label".into(), label);
    out.insert("instrument".into(), json!(inst));
    out.insert("role".into(), role);
    out.insert("string_count".into(), json!(sc));
    out.insert("tuning".into(), tuning);
    out.insert("reference_pitch".into(), num(rp));
    out.insert("pathway".into(), pathway);
    Ok(out)
}

pub fn normalize_profiles(raw: Option<&Value>) -> Result<Map<String, Value>, String> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(default_profiles()),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("instrument_profiles must be an object".into()),
    };
    let mut out = Map::new();
    for id in PROFILE_IDS {
        out.insert(
            id.into(),
            Value::Object(normalize_profile(id, raw.get(id))?),
        );
    }
    Ok(out)
}

fn active_id(v: Option<&Value>) -> &'static str {
    match v.and_then(Value::as_str) {
        Some("guitar-rhythm") => "guitar-rhythm",
        Some("bass") => "bass",
        Some("guitar-lead") => "guitar-lead",
        _ => "guitar-lead",
    }
}

fn profile_from_legacy(cfg: &Map<String, Value>) -> Map<String, Value> {
    let instrument = match cfg.get("instrument").and_then(Value::as_str) {
        Some("bass") => "bass",
        _ => "guitar",
    };
    let fallback = if instrument == "bass" { 4 } else { 6 };
    let mut sc = cfg.get("string_count").and_then(py_int).unwrap_or(fallback);
    if standard_open_midis(&format!("{instrument}-{sc}")).is_none() {
        sc = fallback;
    }
    let key = format!("{instrument}-{sc}");
    let tuning = valid_tuning_for_key(&key, cfg.get("tuning").unwrap_or(&json!("Standard")))
        .unwrap_or_else(|| json!("Standard"));
    let rp = valid_reference_pitch(
        cfg.get("reference_pitch")
            .unwrap_or(&json!(DEFAULT_REFERENCE_PITCH)),
    )
    .unwrap_or(DEFAULT_REFERENCE_PITCH);
    let pathway = cfg
        .get("pathway")
        .and_then(Value::as_str)
        .filter(|p| PROFILE_PATHWAYS.contains(p))
        .unwrap_or("songs");
    let id = if instrument == "bass" {
        "bass"
    } else {
        "guitar-lead"
    };
    let mut p = profile_default(id).unwrap();
    p.insert("instrument".into(), json!(instrument));
    p.insert("string_count".into(), json!(sc));
    p.insert("tuning".into(), tuning);
    p.insert("reference_pitch".into(), num(rp));
    p.insert("pathway".into(), json!(pathway));
    p
}

/// `settings_with_instrument_profiles`: canonical profiles + mirrored flat keys.
pub fn settings_with_profiles(cfg: &Map<String, Value>) -> Map<String, Value> {
    let mut out = cfg.clone();
    let mut profiles =
        normalize_profiles(out.get("instrument_profiles")).unwrap_or_else(|_| default_profiles());
    if !out.contains_key("instrument_profiles") {
        let legacy = profile_from_legacy(&out);
        let id = legacy["id"].as_str().unwrap_or("guitar-lead").to_string();
        profiles.insert(id.clone(), Value::Object(legacy));
        out.entry("active_instrument_profile").or_insert(json!(id));
    }
    let active = active_id(out.get("active_instrument_profile"));
    let sel = profiles[active].clone();
    out.insert("instrument_profiles".into(), Value::Object(profiles));
    out.insert("active_instrument_profile".into(), json!(active));
    for k in [
        "instrument",
        "string_count",
        "tuning",
        "reference_pitch",
        "pathway",
    ] {
        out.insert(k.into(), sel[k].clone());
    }
    out
}

/// `apply_flat_instrument_patch_to_profiles`.
pub fn apply_flat_patch(
    cfg: &Map<String, Value>,
    updates: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let mut out = settings_with_profiles(cfg);
    let flat = [
        "instrument",
        "string_count",
        "tuning",
        "reference_pitch",
        "pathway",
    ];
    if !flat.iter().any(|k| updates.contains_key(*k)) {
        return Ok(out);
    }
    let mut active = active_id(out.get("active_instrument_profile")).to_string();
    if let Some(inst) = updates.get("instrument") {
        active = if inst.as_str() == Some("bass") {
            "bass".into()
        } else {
            "guitar-lead".into()
        };
        out.insert("active_instrument_profile".into(), json!(active));
    }
    let mut cur = out["instrument_profiles"][&active]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(inst) = updates.get("instrument") {
        cur.insert("instrument".into(), inst.clone());
        if !updates.contains_key("string_count") {
            cur.insert(
                "string_count".into(),
                json!(if inst.as_str() == Some("bass") { 4 } else { 6 }),
            );
        }
    }
    for k in ["string_count", "reference_pitch", "pathway"] {
        if let Some(v) = updates.get(k) {
            cur.insert(k.into(), v.clone());
        }
    }
    if let Some(t) = updates.get("tuning") {
        cur.insert("tuning".into(), t.clone());
    } else {
        let key = format!(
            "{}-{}",
            cur["instrument"].as_str().unwrap_or(""),
            py_int(&cur["string_count"]).unwrap_or(0)
        );
        if valid_tuning_for_key(&key, cur.get("tuning").unwrap_or(&Value::Null)).is_none() {
            cur.insert("tuning".into(), json!("Standard"));
        }
    }
    let profile = normalize_profile(&active, Some(&Value::Object(cur)))?;
    for k in flat {
        out.insert(k.into(), profile[k].clone());
    }
    if let Some(Value::Object(ps)) = out.get_mut("instrument_profiles") {
        ps.insert(active, Value::Object(profile));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(tuning_name(&[0, 0, 0, 0, 0, 0]), "E Standard");
        assert_eq!(tuning_name(&[-2, 0, 0, 0, 0, 0]), "Drop D");
        assert_eq!(tuning_name(&[-4, -2, -2, -2]), "Drop C");
        assert_eq!(tuning_name(&[-1, -1, -1, -1]), "Eb Standard");
        assert_eq!(tuning_name(&[0, 2, 2, 1, 0, 0]), "Open E");
        assert_eq!(tuning_name(&[]), "Unknown");
        assert_eq!(
            perspective_tuning_name(&[5, 5, 5, 5], &BASS),
            "Custom Tuning"
        );
    }

    #[test]
    fn bass_key_and_low_pitch() {
        assert_eq!(
            perspective_tuning_key(&[0, 0, 0, 0], &BASS),
            "bass:28:33:38:43"
        );
        assert_eq!(perspective_low_pitch(&[-2, 0, 0, 0], &BASS), Some(26));
    }
}
