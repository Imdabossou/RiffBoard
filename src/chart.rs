//! Reading a pack into the shared chart model (`riffboard_core::chart`).
//!
//! A pack is a manifest (`manifest.yaml`/`.json`) naming its arrangement
//! files, audio stems, lyrics and an optional song timeline. Each arrangement
//! becomes one `Chart` with the song-level parts filled in, ready to send to
//! the game as JSON.

use std::path::Path;

use riffboard_core::chart::{self as core, Chart, Section};
use serde_json::{Value, json};

use crate::pack;
use crate::util::{as_f64, py_str, truthy};

fn arr<'a>(d: &'a Value, k: &str) -> &'a [Value] {
    d.get(k)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// An audio file the pack names.
#[derive(Clone, Debug)]
pub struct PackStem {
    pub id: String,
    pub file: String,
    pub default: bool,
}

fn stem_default_on(v: Option<&Value>) -> bool {
    match v {
        None => true,
        Some(Value::String(s)) => !["off", "false", "0", "no"].contains(&s.to_lowercase().as_str()),
        other => truthy(other),
    }
}

/// Stems from the manifest, with the reserved `full` mixdown lifted out when
/// the pack also has instrument stems (spec §5.3).
pub fn manifest_stems(manifest: &Value) -> (Option<PackStem>, Vec<PackStem>) {
    let stems: Vec<PackStem> = arr(manifest, "stems")
        .iter()
        .filter_map(|s| {
            let id = py_str(s.get("id"), "");
            let file = py_str(s.get("file"), "");
            (s.is_object() && !id.is_empty() && !file.is_empty()).then(|| PackStem {
                id,
                file,
                default: stem_default_on(s.get("default")),
            })
        })
        .collect();
    if stems.len() < 2 {
        return (None, stems);
    }
    match stems.iter().position(|s| s.id == "full") {
        Some(i) => {
            let mut stems = stems;
            let full = stems.remove(i);
            (Some(full), stems)
        }
        None => (None, stems),
    }
}

/// The manifest's arrangement entries RiffBoard plays: guitar and bass parts
/// with a chart file (drum parts are skipped). Their order is the
/// arrangement index everywhere (song page, chart, leaderboard).
pub fn playable_entries(manifest: &Value) -> Vec<&Value> {
    arr(manifest, "arrangements")
        .iter()
        .filter(|e| {
            let kind = py_str(e.get("type"), "").trim().to_lowercase();
            let file = e.get("file").and_then(Value::as_str).unwrap_or("").trim();
            e.is_object() && !file.is_empty() && kind != "drums" && kind != "drum"
        })
        .collect()
}

/// An arrangement's display name: its `name`, else its `id`, else "Part N".
pub fn entry_name(e: &Value, index: usize) -> String {
    let name = py_str(e.get("name"), "");
    let name = if name.trim().is_empty() {
        py_str(e.get("id"), "")
    } else {
        name
    };
    if name.trim().is_empty() {
        format!("Part {}", index + 1)
    } else {
        name.trim().chars().take(60).collect()
    }
}

/// Every playable arrangement of a pack, as charts. `file_url` turns a path
/// inside the pack into the URL the game fetches it from. An arrangement
/// whose file can't be read stays in the list, empty, so indices still match
/// the song page.
pub fn load(source_dir: &Path, manifest: &Value, file_url: impl Fn(&str) -> String) -> Vec<Chart> {
    let entries = playable_entries(manifest);
    let mut charts: Vec<(Chart, String)> = Vec::new();
    // Beats and sections are song-wide: from the timeline file when it has
    // both, else from the first arrangement that has them.
    let mut beats = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let rel = e.get("file").and_then(Value::as_str).unwrap_or("").trim();
        let data = pack::read_pack_json(source_dir, rel).unwrap_or_else(|| json!({}));
        let mut c = Chart::from_pack(&data);
        if beats.is_empty() {
            beats.clone_from(&c.beats);
        }
        if sections.is_empty() {
            sections.clone_from(&c.sections);
        }
        c.arrangement = entry_name(e, i);
        c.arrangement_index = i;
        if let Some(t) = e.get("tuning").and_then(Value::as_array) {
            c.tuning = t
                .iter()
                .map(|v| as_f64(Some(v), 0.0).clamp(-24.0, 24.0) as i32)
                .collect();
        }
        if e.get("capo").is_some() {
            c.capo = as_f64(e.get("capo"), 0.0).clamp(0.0, 24.0) as i32;
        }
        if e.get("centOffset").is_some() {
            c.cent_offset = as_f64(e.get("centOffset"), 0.0);
        }
        charts.push((c, py_str(e.get("type"), "")));
    }

    if let Some(raw) = manifest
        .get("song_timeline")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .and_then(|rel| pack::read_pack_json(source_dir, rel))
        && raw.get("beats").is_some_and(Value::is_array)
        && raw.get("sections").is_some_and(Value::is_array)
    {
        beats = core::beats(arr(&raw, "beats"));
        sections = core::sections(arr(&raw, "sections"));
    }

    let lyrics = manifest
        .get("lyrics")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .and_then(|rel| pack::read_pack_json(source_dir, rel))
        .and_then(|v| v.as_array().map(|a| core::lyric_lines(a)))
        .unwrap_or_default();

    let (full, stems) = manifest_stems(manifest);
    let full_mix = full.map(|f| f.file).or_else(|| {
        // Older packs name a plain `original_audio` file instead of stems.
        manifest
            .get("original_audio")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|r| {
                stems.is_empty()
                    && !r.is_empty()
                    && pack::resolve_in(source_dir, r).is_some_and(|p| p.is_file())
            })
            .map(String::from)
    });
    let stems: Vec<core::Stem> = stems
        .iter()
        .map(|s| core::Stem {
            id: s.id.clone(),
            url: file_url(&s.file),
            default: s.default,
        })
        .collect();
    let audio_url = stems
        .first()
        .map(|s| s.url.clone())
        .or_else(|| full_mix.as_deref().map(&file_url));

    let title = py_str(manifest.get("title"), "");
    let artist = py_str(manifest.get("artist"), "");
    let duration = as_f64(manifest.get("duration"), 0.0).max(0.0);
    charts
        .into_iter()
        .map(|(mut c, kind)| {
            c.title.clone_from(&title);
            c.artist.clone_from(&artist);
            c.duration = duration;
            c.beats.clone_from(&beats);
            c.sections.clone_from(&sections);
            c.lyrics.clone_from(&lyrics);
            c.stems.clone_from(&stems);
            c.audio_url.clone_from(&audio_url);
            c.finish(&kind);
            c
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_and_names() {
        let m = json!({"arrangements": [
            {"name": "Lead", "file": "a.json"},
            {"type": "drums", "file": "d.json"},
            {"id": "bass_x", "file": "b.json"},
            {"name": "No file"},
            {"name": " ", "file": "c.json"},
        ]});
        let e = playable_entries(&m);
        let names: Vec<String> = e
            .iter()
            .enumerate()
            .map(|(i, e)| entry_name(e, i))
            .collect();
        assert_eq!(names, vec!["Lead", "bass_x", "Part 3"]);
    }

    #[test]
    fn full_mix_is_lifted_out_of_stems() {
        let m = json!({"stems": [{"id": "full", "file": "f.ogg"}, {"id": "bass", "file": "b.ogg", "default": "off"}]});
        let (full, stems) = manifest_stems(&m);
        assert_eq!(full.unwrap().file, "f.ogg");
        assert_eq!(stems.len(), 1);
        assert!(!stems[0].default);
        let (full, stems) = manifest_stems(&json!({"stems": [{"id": "full", "file": "f.ogg"}]}));
        assert!(full.is_none());
        assert_eq!(stems[0].id, "full");
    }
}
