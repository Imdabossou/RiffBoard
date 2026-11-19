//! The chart as the game uses it, built from the highway WebSocket messages
//! (`song_info`, `beats`, `sections`, `anchors`, `chord_templates`, `notes`,
//! `chords`, ... `ready`). Unknown fields and message types are ignored, and
//! malformed values fall back to defaults, so a bad pack never breaks the game.

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Note {
    pub time: f64,
    pub string: usize,
    pub fret: i32,
    pub sustain: f64,
    /// Fret a slide ends on, if any.
    pub slide_to: Option<i32>,
    /// Bend in semitones (0 = none).
    pub bend: f64,
    pub hammer_on: bool,
    pub pull_off: bool,
    pub harmonic: bool,
    pub palm_mute: bool,
    pub mute: bool,
    pub tremolo: bool,
    pub accent: bool,
    pub tap: bool,
    pub vibrato: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chord {
    pub time: f64,
    pub template: usize,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beat {
    pub time: f64,
    /// Bar number on the first beat of a bar, else `None`.
    pub measure: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Anchor {
    pub time: f64,
    pub fret: i32,
    pub width: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub time: f64,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Stem {
    pub id: String,
    pub url: String,
    pub default: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ArrangementInfo {
    pub index: i64,
    pub name: String,
    pub smart_name: Option<String>,
    pub notes: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chart {
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub arrangement: String,
    pub arrangement_index: i64,
    pub arrangements: Vec<ArrangementInfo>,
    pub audio_url: Option<String>,
    pub stems: Vec<Stem>,
    /// Semitone offset of each string from standard, lowest string first.
    pub tuning: Vec<i32>,
    pub string_count: usize,
    pub capo: i32,
    pub cent_offset: f64,
    pub bass: bool,
    pub beats: Vec<Beat>,
    pub sections: Vec<Section>,
    pub anchors: Vec<Anchor>,
    /// Chord names by template id.
    pub chord_names: Vec<String>,
    pub notes: Vec<Note>,
    pub chords: Vec<Chord>,
}

fn f(v: &Value, k: &str) -> f64 {
    v.get(k)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .unwrap_or(0.0)
}

fn i(v: &Value, k: &str, default: i64) -> i64 {
    match v.get(k) {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|x| x.is_finite()).map(|x| x as i64))
            .unwrap_or(default),
        _ => default,
    }
}

fn b(v: &Value, k: &str) -> bool {
    match v.get(k) {
        Some(Value::Bool(x)) => *x,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        _ => false,
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn data(v: &Value) -> &[Value] {
    v.get("data")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn note(v: &Value, time: f64) -> Note {
    let sl = i(v, "sl", -1);
    Note {
        time: v
            .get("t")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite())
            .unwrap_or(time),
        string: i(v, "s", 0).clamp(0, 15) as usize,
        fret: i(v, "f", 0).clamp(0, 30) as i32,
        sustain: f(v, "sus").max(0.0),
        slide_to: (sl >= 0).then_some(sl.min(30) as i32),
        bend: f(v, "bn"),
        hammer_on: b(v, "ho"),
        pull_off: b(v, "po"),
        harmonic: b(v, "hm") || b(v, "hp"),
        palm_mute: b(v, "pm"),
        mute: b(v, "mt") || b(v, "fhm"),
        tremolo: b(v, "tr"),
        accent: b(v, "ac"),
        tap: b(v, "tp"),
        vibrato: b(v, "vb"),
    }
}

impl Chart {
    /// Builds a chart from the highway messages, in the order they arrived.
    pub fn from_messages(msgs: &[Value]) -> Chart {
        let mut c = Chart {
            string_count: 4,
            bass: true,
            ..Chart::default()
        };
        for m in msgs {
            match m.get("type").and_then(Value::as_str).unwrap_or("") {
                "song_info" => {
                    c.title = s(m, "title");
                    c.artist = s(m, "artist");
                    c.duration = f(m, "duration");
                    c.arrangement = s(m, "arrangement");
                    c.arrangement_index = i(m, "arrangement_index", 0);
                    c.arrangements = m
                        .get("arrangements")
                        .and_then(|a| serde_json::from_value(a.clone()).ok())
                        .unwrap_or_default();
                    c.audio_url = m.get("audio_url").and_then(Value::as_str).map(String::from);
                    c.stems = m
                        .get("stems")
                        .and_then(|a| serde_json::from_value(a.clone()).ok())
                        .unwrap_or_default();
                    c.tuning = m
                        .get("tuning")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .map(|x| x.as_f64().unwrap_or(0.0).clamp(-24.0, 24.0) as i32)
                                .collect()
                        })
                        .unwrap_or_default();
                    c.string_count = i(m, "stringCount", 4).clamp(1, 8) as usize;
                    c.capo = i(m, "capo", 0).clamp(0, 24) as i32;
                    c.cent_offset = f(m, "centOffset");
                    let name = c.arrangement.to_lowercase();
                    // The server reports at least 6 strings for anything not named bass.
                    c.bass = name.contains("bass") || c.string_count <= 5;
                }
                "beats" => c.beats.extend(data(m).iter().map(|v| Beat {
                    time: f(v, "time"),
                    measure: Some(i(v, "measure", -1)).filter(|x| *x >= 0),
                })),
                "sections" => c.sections.extend(data(m).iter().map(|v| Section {
                    time: f(v, "time"),
                    name: s(v, "name"),
                })),
                "anchors" => c.anchors.extend(data(m).iter().map(|v| Anchor {
                    time: f(v, "time"),
                    fret: i(v, "fret", 1).clamp(0, 30) as i32,
                    width: i(v, "width", 4).clamp(1, 24) as i32,
                })),
                "chord_templates" => c.chord_names.extend(data(m).iter().map(|v| {
                    let n = s(v, "displayName");
                    if n.is_empty() { s(v, "name") } else { n }
                })),
                "notes" => c.notes.extend(data(m).iter().map(|v| note(v, 0.0))),
                "chords" => c.chords.extend(data(m).iter().map(|v| {
                    let t = f(v, "t");
                    Chord {
                        time: t,
                        template: i(v, "id", 0).max(0) as usize,
                        notes: v
                            .get("notes")
                            .and_then(Value::as_array)
                            .map(|ns| ns.iter().map(|n| note(n, t)).collect())
                            .unwrap_or_default(),
                    }
                })),
                _ => {}
            }
        }
        for n in &mut c.notes {
            n.string = n.string.min(c.string_count - 1);
        }
        for ch in &mut c.chords {
            for n in &mut ch.notes {
                n.string = n.string.min(c.string_count - 1);
            }
        }
        c.notes.sort_by(|a, b| a.time.total_cmp(&b.time));
        c.chords.sort_by(|a, b| a.time.total_cmp(&b.time));
        c.beats.sort_by(|a, b| a.time.total_cmp(&b.time));
        c.anchors.sort_by(|a, b| a.time.total_cmp(&b.time));
        c.sections.sort_by(|a, b| a.time.total_cmp(&b.time));
        c
    }

    /// Parses a JSON array of highway messages.
    pub fn from_json(json: &str) -> Result<Chart, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let arr = v.as_array().ok_or("expected an array of messages")?;
        if let Some(e) = arr
            .iter()
            .find_map(|m| m.get("error").and_then(Value::as_str))
        {
            return Err(e.to_string());
        }
        Ok(Chart::from_messages(arr))
    }

    /// Notes + chords: the number of events the leaderboard judges.
    pub fn judgeable(&self) -> usize {
        self.notes.len() + self.chords.len()
    }

    /// Time of the last note or chord.
    pub fn last_event_time(&self) -> f64 {
        let n = self.notes.last().map_or(0.0, |n| n.time);
        let c = self.chords.last().map_or(0.0, |c| c.time);
        n.max(c)
    }

    /// The anchor (fret window) in effect at `t`.
    pub fn anchor_at(&self, t: f64) -> Anchor {
        let idx = self.anchors.partition_point(|a| a.time <= t);
        if idx == 0 {
            self.anchors.first().cloned().unwrap_or(Anchor {
                time: 0.0,
                fret: 1,
                width: 4,
            })
        } else {
            self.anchors[idx - 1].clone()
        }
    }

    /// MIDI pitch of a note in this chart.
    pub fn midi(&self, n: &Note) -> i32 {
        crate::pitch::midi(
            n.string,
            n.fret,
            &self.tuning,
            self.capo,
            self.bass,
            self.string_count,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Vec<Value> {
        vec![
            json!({"type": "loading", "stage": "x"}),
            json!({"type": "song_info", "title": "T", "artist": "A", "duration": 10.0,
                "arrangement": "Bass", "arrangement_index": 0,
                "arrangements": [{"index": 0, "name": "Bass", "smart_name": null, "notes": 3}],
                "audio_url": "/a.ogg", "stems": [{"id": "full", "url": "/a.ogg", "default": true}],
                "tuning": [-2, 0, 0, 0], "stringCount": 4, "capo": 0, "centOffset": 0}),
            json!({"type": "beats", "data": [{"time": 0, "measure": 1}, {"time": 0.5, "measure": -1}]}),
            json!({"type": "anchors", "data": [{"time": 0, "fret": 3, "width": 4}, {"time": 5, "fret": 7, "width": 4}]}),
            json!({"type": "notes", "data": [{"t": 2, "s": 0, "f": 0, "sus": 0}, {"t": 1, "s": 9, "f": 3, "sl": 5, "pm": true}], "total": 2}),
            json!({"type": "chords", "data": [{"t": 3, "id": 0, "notes": [{"s": 0, "f": 2}, {"s": 1, "f": 4}]}], "total": 1}),
            json!({"type": "ready"}),
        ]
    }

    #[test]
    fn parses_messages() {
        let c = Chart::from_messages(&sample());
        assert_eq!(c.title, "T");
        assert!(c.bass);
        assert_eq!(c.tuning, vec![-2, 0, 0, 0]);
        assert_eq!(c.notes.len(), 2);
        // Sorted by time; an out-of-range string is clamped.
        assert_eq!(c.notes[0].time, 1.0);
        assert_eq!(c.notes[0].string, 3);
        assert_eq!(c.notes[0].slide_to, Some(5));
        assert!(c.notes[0].palm_mute);
        assert_eq!(c.chords[0].notes[1].time, 3.0);
        assert_eq!(c.judgeable(), 3);
        assert_eq!(c.last_event_time(), 3.0);
        assert_eq!(c.anchor_at(6.0).fret, 7);
        assert_eq!(c.anchor_at(1.0).fret, 3);
        assert_eq!(c.beats[0].measure, Some(1));
        assert_eq!(c.beats[1].measure, None);
        // Drop D open E string is D1 (26).
        assert_eq!(c.midi(&c.notes[1]), 26);
    }

    #[test]
    fn errors_and_garbage() {
        assert!(Chart::from_json("[{\"error\": \"File not found\"}]").is_err());
        assert!(Chart::from_json("{}").is_err());
        let c =
            Chart::from_json("[{\"type\":\"notes\",\"data\":[{\"t\":\"x\",\"s\":null}]}]").unwrap();
        assert_eq!(c.notes.len(), 1);
        assert_eq!(c.notes[0].time, 0.0);
    }
}
