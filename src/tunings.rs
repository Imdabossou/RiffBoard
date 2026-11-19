//! Tuning names, presets, per-role "perspectives" and instrument profiles.
//! Ported from upstream lib/tunings.py.

use serde_json::Value;

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
