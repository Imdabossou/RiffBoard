//! The chart model shared by the server and the game.
//!
//! The server reads a pack's arrangement file with [`Chart::from_pack`]
//! (lenient: malformed values fall back to defaults, so a bad pack never
//! breaks the game), fills in the song-level parts (title, audio, beats,
//! lyrics), calls [`Chart::finish`] and sends the result as JSON. The game
//! reads that JSON back with [`Chart::from_json`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

/// One note. Field names on the wire follow the pack format (`t`, `s`, `f`,
/// `sus`, ...) and default values are left out, which keeps a chart small.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    #[serde(rename = "t")]
    pub time: f64,
    #[serde(rename = "s")]
    pub string: usize,
    #[serde(rename = "f")]
    pub fret: i32,
    #[serde(rename = "sus", skip_serializing_if = "is_zero")]
    pub sustain: f64,
    /// Fret a slide ends on, if any.
    #[serde(rename = "sl", skip_serializing_if = "Option::is_none")]
    pub slide_to: Option<i32>,
    /// Bend in semitones (0 = none).
    #[serde(rename = "bn", skip_serializing_if = "is_zero")]
    pub bend: f64,
    #[serde(rename = "ho", skip_serializing_if = "is_false")]
    pub hammer_on: bool,
    #[serde(rename = "po", skip_serializing_if = "is_false")]
    pub pull_off: bool,
    #[serde(rename = "hm", skip_serializing_if = "is_false")]
    pub harmonic: bool,
    #[serde(rename = "pm", skip_serializing_if = "is_false")]
    pub palm_mute: bool,
    #[serde(rename = "mt", skip_serializing_if = "is_false")]
    pub mute: bool,
    #[serde(rename = "tr", skip_serializing_if = "is_false")]
    pub tremolo: bool,
    #[serde(rename = "ac", skip_serializing_if = "is_false")]
    pub accent: bool,
    #[serde(rename = "tp", skip_serializing_if = "is_false")]
    pub tap: bool,
    #[serde(rename = "vb", skip_serializing_if = "is_false")]
    pub vibrato: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chord {
    #[serde(rename = "t")]
    pub time: f64,
    /// Index into [`Chart::chord_names`].
    #[serde(rename = "id")]
    pub template: usize,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beat {
    pub time: f64,
    /// Bar number on the first beat of a bar, else `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Anchor {
    pub time: f64,
    pub fret: i32,
    pub width: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Section {
    pub time: f64,
    pub name: String,
}

/// One sung word (or syllable). `text` already ends with a space unless the
/// next syllable joins on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricWord {
    pub time: f64,
    pub dur: f64,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricLine {
    pub start: f64,
    pub end: f64,
    pub words: Vec<LyricWord>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stem {
    pub id: String,
    pub url: String,
    pub default: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chart {
    pub title: String,
    pub artist: String,
    pub duration: f64,
    /// The arrangement's name and its index in the song.
    pub arrangement: String,
    pub arrangement_index: usize,
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
    pub lyrics: Vec<LyricLine>,
}

/// Groups lyric events into lines: a word ending in `+` ends its line, as
/// does a pause of more than three seconds. A trailing `-` joins the next
/// syllable without a space.
pub fn lyric_lines(events: &[Value]) -> Vec<LyricLine> {
    let mut words: Vec<(f64, f64, String)> = events
        .iter()
        .filter_map(|v| {
            let w = v.get("w").and_then(Value::as_str)?;
            Some((f(v, "t"), f(v, "d").max(0.0), w.chars().take(60).collect()))
        })
        .collect();
    words.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut cur: Vec<LyricWord> = Vec::new();
    let flush = |cur: &mut Vec<LyricWord>, lines: &mut Vec<LyricLine>| {
        if let (Some(a), Some(b)) = (cur.first(), cur.last()) {
            lines.push(LyricLine {
                start: a.time,
                end: b.time + b.dur,
                words: std::mem::take(cur),
            });
        }
    };
    for (t, d, w) in words {
        if let Some(last) = cur.last()
            && t - (last.time + last.dur) > 3.0
        {
            flush(&mut cur, &mut lines);
        }
        let end_line = w.ends_with('+');
        let w = w.trim_end_matches('+');
        let text = match w.strip_suffix('-') {
            Some(stem) => stem.to_string(),
            None => format!("{w} "),
        };
        if !text.trim().is_empty() {
            cur.push(LyricWord {
                time: t,
                dur: d,
                text,
            });
        }
        if end_line || cur.len() >= 12 {
            flush(&mut cur, &mut lines);
        }
    }
    flush(&mut cur, &mut lines);
    lines
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

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
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

/// Beats from a pack (`measure` -1 means "not the first beat of a bar").
pub fn beats(v: &[Value]) -> Vec<Beat> {
    v.iter()
        .filter(|b| b.is_object())
        .map(|b| Beat {
            time: f(b, "time"),
            measure: Some(i(b, "measure", -1)).filter(|x| *x >= 0),
        })
        .collect()
}

/// Sections from a pack (`time`, or upstream's `start_time`).
pub fn sections(v: &[Value]) -> Vec<Section> {
    v.iter()
        .filter(|x| x.is_object())
        .map(|x| Section {
            time: if x.get("time").is_some() {
                f(x, "time")
            } else {
                f(x, "start_time")
            },
            name: s(x, "name").chars().take(40).collect(),
        })
        .collect()
}

impl Chart {
    /// Reads one arrangement file of a pack. Song-level parts (title, audio,
    /// lyrics) are the caller's; call [`Chart::finish`] once they are set.
    pub fn from_pack(d: &Value) -> Chart {
        Chart {
            arrangement: s(d, "name"),
            tuning: arr(d, "tuning")
                .iter()
                .map(|v| {
                    v.as_f64()
                        .filter(|x| x.is_finite())
                        .unwrap_or(0.0)
                        .clamp(-24.0, 24.0) as i32
                })
                .collect(),
            capo: i(d, "capo", 0).clamp(0, 24) as i32,
            cent_offset: f(d, "centOffset"),
            beats: beats(arr(d, "beats")),
            sections: sections(arr(d, "sections")),
            anchors: arr(d, "anchors")
                .iter()
                .map(|v| Anchor {
                    time: f(v, "time"),
                    fret: i(v, "fret", 1).clamp(0, 30) as i32,
                    width: i(v, "width", 4).clamp(1, 24) as i32,
                })
                .collect(),
            chord_names: arr(d, "templates")
                .iter()
                .map(|v| {
                    let n = s(v, "displayName");
                    let n = if n.is_empty() { s(v, "name") } else { n };
                    n.chars().take(40).collect()
                })
                .collect(),
            notes: arr(d, "notes").iter().map(|v| note(v, 0.0)).collect(),
            chords: arr(d, "chords")
                .iter()
                .map(|v| {
                    let t = f(v, "t");
                    Chord {
                        time: t,
                        template: i(v, "id", 0).max(0) as usize,
                        notes: arr(v, "notes").iter().map(|n| note(n, t)).collect(),
                    }
                })
                .collect(),
            ..Chart::default()
        }
    }

    /// Settles what depends on everything else: whether it is a bass part
    /// (`kind` is the pack's arrangement type), the string count, string
    /// numbers in range, time order, and the hand positions.
    pub fn finish(&mut self, kind: &str) {
        self.bass = kind.trim().eq_ignore_ascii_case("bass")
            || self.arrangement.to_lowercase().contains("bass");
        let used = self
            .notes
            .iter()
            .chain(self.chords.iter().flat_map(|c| &c.notes))
            .map(|n| n.string + 1)
            .filter(|&n| n <= 8)
            .max()
            .unwrap_or(0);
        let tl = self.tuning.len();
        self.string_count = used
            .max(if self.bass { 4 } else { 6 })
            .max(if tl != 6 { tl } else { 0 })
            .clamp(1, 8);
        let last = self.string_count - 1;
        for n in self
            .notes
            .iter_mut()
            .chain(self.chords.iter_mut().flat_map(|c| c.notes.iter_mut()))
        {
            n.string = n.string.min(last);
        }
        let template_count = self.chord_names.len().max(1);
        for c in &mut self.chords {
            c.template = c.template.min(template_count - 1);
        }
        self.notes.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.chords.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.beats.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.anchors.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.sections.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.anchors = self.hand_positions();
    }

    /// Reads a chart the server sent.
    pub fn from_json(json: &str) -> Result<Chart, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// Where the fretting hand sits through the song, worked out from the
    /// notes: the window stays put while the notes fit in it and moves only
    /// when one falls outside, to the chart's own anchor if that covers what
    /// comes next, else by the smallest shift that does. Charts' anchors can
    /// flip between windows that all hold the same notes, which would slide
    /// the board for no reason.
    fn hand_positions(&self) -> Vec<Anchor> {
        // (time, lowest fret, highest fret) of every fretted note or chord.
        let mut ev: Vec<(f64, i32, i32)> = Vec::new();
        let span = |n: &Note| {
            let end = n.slide_to.filter(|f| *f > 0).unwrap_or(n.fret);
            (n.fret.min(end), n.fret.max(end))
        };
        for n in self.notes.iter().filter(|n| n.fret > 0) {
            let (lo, hi) = span(n);
            ev.push((n.time, lo, hi));
        }
        for ch in &self.chords {
            let fretted = ch.notes.iter().filter(|n| n.fret > 0).map(span);
            if let Some((lo, hi)) = fretted.reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))) {
                ev.push((ch.time, lo, hi));
            }
        }
        if ev.is_empty() {
            return self.anchors.clone();
        }
        ev.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut out: Vec<Anchor> = Vec::new();
        let mut cur: Option<(i32, i32)> = None; // (first fret, width)
        for i in 0..ev.len() {
            let (t, lo, hi) = ev[i];
            if let Some((f, w)) = cur
                && lo >= f
                && hi < f + w
            {
                continue;
            }
            // The run of notes from here that fits in one hand position.
            let (mut mn, mut mx) = (lo, hi);
            for &(_, l, h) in &ev[i + 1..] {
                if h.max(mx) - l.min(mn) + 1 > 4.max(hi - lo + 1) {
                    break;
                }
                mn = mn.min(l);
                mx = mx.max(h);
            }
            let need = (mx - mn + 1).max(4);
            let authored = self.anchor_at(t);
            let aw = authored.width.max(4);
            let pos = if !self.anchors.is_empty()
                && authored.fret >= 1
                && authored.fret <= mn
                && authored.fret + aw > mx
            {
                (authored.fret, aw)
            } else {
                let f = match cur {
                    Some((f, _)) if mx > f => mx - need + 1,
                    _ => mn,
                };
                (f.max(1), need)
            };
            cur = Some(pos);
            out.push(Anchor {
                time: if out.is_empty() { t.min(0.0) } else { t },
                fret: pos.0,
                width: pos.1,
            });
        }
        out
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

    fn sample() -> Chart {
        let mut c = Chart::from_pack(&json!({
            "name": "Bass", "tuning": [-2, 0, 0, 0], "capo": 0,
            "beats": [{"time": 0, "measure": 1}, {"time": 0.5, "measure": -1}],
            "anchors": [{"time": 0, "fret": 3, "width": 4}, {"time": 5, "fret": 7, "width": 4}],
            "templates": [{"name": "G5", "displayName": ""}],
            "notes": [{"t": 2, "s": 0, "f": 0, "sus": 0}, {"t": 1, "s": 9, "f": 3, "sl": 5, "pm": true}],
            "chords": [{"t": 3, "id": 4, "notes": [{"s": 0, "f": 2}, {"s": 1, "f": 4}]}],
        }));
        c.title = "T".into();
        c.finish("bass");
        c
    }

    #[test]
    fn reads_a_pack() {
        let c = sample();
        assert_eq!(c.arrangement, "Bass");
        assert!(c.bass);
        assert_eq!(c.string_count, 4);
        assert_eq!(c.tuning, vec![-2, 0, 0, 0]);
        assert_eq!(c.notes.len(), 2);
        // Sorted by time; an out-of-range string is clamped.
        assert_eq!(c.notes[0].time, 1.0);
        assert_eq!(c.notes[0].string, 3);
        assert_eq!(c.notes[0].slide_to, Some(5));
        assert!(c.notes[0].palm_mute);
        assert_eq!(c.chords[0].notes[1].time, 3.0);
        assert_eq!(c.chords[0].template, 0, "template id clamped");
        assert_eq!(c.chord_names, vec!["G5"]);
        assert_eq!(c.judgeable(), 3);
        assert_eq!(c.last_event_time(), 3.0);
        // Hand positions come from the notes: fret 3 (sliding to 5) then a
        // 2-4 chord fit one window from fret 2; the authored 7 has no notes.
        assert_eq!(c.anchor_at(1.0).fret, 2);
        assert_eq!(c.anchor_at(6.0).fret, 2);
        assert_eq!(c.beats[0].measure, Some(1));
        assert_eq!(c.beats[1].measure, None);
        // Drop D open E string is D1 (26).
        assert_eq!(c.midi(&c.notes[1]), 26);
    }

    #[test]
    fn wire_round_trip_is_small_and_exact() {
        let c = sample();
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(Chart::from_json(&json).unwrap(), c);
        // Defaults are left out of notes.
        assert!(json.contains(r#"{"t":2.0,"s":0,"f":0}"#), "{json}");
        assert!(!json.contains("\"ho\""), "{json}");
    }

    #[test]
    fn guitar_and_string_counts() {
        let mut c = Chart::from_pack(&json!({"name": "Lead", "notes": [{"t": 1, "s": 2, "f": 5}]}));
        c.finish("guitar");
        assert!(!c.bass);
        assert_eq!(c.string_count, 6);
        let mut b = Chart::from_pack(&json!({"name": "Part", "tuning": [0, 0, 0, 0, 0]}));
        b.finish("bass");
        assert_eq!(b.string_count, 5, "five-string bass");
    }

    fn hand_chart(anchors: Value, frets: &[(f64, i32)]) -> Chart {
        let notes: Vec<Value> = frets
            .iter()
            .map(|(t, f)| json!({"t": t, "s": 0, "f": f}))
            .collect();
        let mut c = Chart::from_pack(&json!({"name": "Bass", "anchors": anchors, "notes": notes}));
        c.finish("bass");
        c
    }

    #[test]
    fn hand_stays_put_while_notes_fit() {
        // The chart flips between windows at 1 and 3 (and blips to 5), but
        // every note is on frets 1-3: the hand never moves.
        let c = hand_chart(
            json!([{"time": 0, "fret": 3, "width": 4}, {"time": 2, "fret": 1, "width": 4},
                   {"time": 4, "fret": 5, "width": 4}, {"time": 4.2, "fret": 3, "width": 4},
                   {"time": 6, "fret": 1, "width": 4}]),
            &[
                (0.5, 3),
                (1.0, 3),
                (2.5, 1),
                (3.0, 2),
                (4.5, 3),
                (6.5, 1),
                (7.0, 3),
            ],
        );
        assert_eq!(c.anchors.len(), 1, "{:?}", c.anchors);
        assert_eq!(c.anchors[0].fret, 1);
    }

    #[test]
    fn hand_moves_when_a_note_leaves_the_window() {
        let c = hand_chart(
            json!([{"time": 0, "fret": 1, "width": 4}, {"time": 3, "fret": 7, "width": 4}]),
            // 1-4, then 5 (just outside: shift by one), then 8-9 (the
            // chart's window at 7 covers it, so take that).
            &[(1.0, 1), (1.5, 4), (2.0, 5), (2.5, 3), (3.5, 8), (4.0, 9)],
        );
        let frets: Vec<i32> = c.anchors.iter().map(|a| a.fret).collect();
        assert_eq!(frets, vec![1, 2, 7]);
        assert_eq!(c.anchor_at(2.2).fret, 2);
        assert_eq!(c.anchor_at(3.6).fret, 7);
    }

    #[test]
    fn lyrics_become_lines() {
        let lines = lyric_lines(&[
            json!({"t": 1, "d": 0.4, "w": "Hel-"}),
            json!({"t": 1.4, "d": 0.4, "w": "lo+"}),
            json!({"t": 2, "d": 0.5, "w": "big"}),
            json!({"t": 2.5, "d": 0.5, "w": "world"}),
            json!({"t": 9, "d": 0.5, "w": "later"}),
        ]);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0]
                .words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<String>(),
            "Hello "
        );
        assert_eq!(lines[1].words.len(), 2);
        assert_eq!(lines[1].end, 3.0);
        assert_eq!(lines[2].start, 9.0);
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        assert!(Chart::from_json("nope").is_err());
        let mut c = Chart::from_pack(
            &json!({"notes": [{"t": "x", "s": null}], "chords": "no", "tuning": ["a"]}),
        );
        c.finish("");
        assert_eq!(c.notes.len(), 1);
        assert_eq!(c.notes[0].time, 0.0);
        assert_eq!(c.tuning, vec![0]);
        assert!(c.chords.is_empty());
    }
}
