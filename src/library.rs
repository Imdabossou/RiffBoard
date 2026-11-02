//! The song library: an in-memory index of the packs in the library folder,
//! rebuilt by a cheap manifest-only scan, plus the read-only queries the
//! browser makes. Ported from upstream lib/metadata_db.py query_* (a SQLite
//! cache is unnecessary at RiffBoard's library sizes).

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::chart;
use crate::pack;
use crate::tunings::{self, Perspective};
use crate::util::{as_f64, nocase, num, py_str};

#[derive(Clone, Debug, Default)]
pub struct RoleTuning {
    pub name: String,
    pub offsets: String,
    pub sort_key: i64,
    pub key: String,
    pub low_pitch: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub filename: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub track: Option<i64>,
    pub disc: Option<i64>,
    pub duration: f64,
    pub arrangements: Vec<Value>,
    pub has_lyrics: bool,
    pub stem_ids: Vec<String>,
    pub tuning_name: String,
    pub tuning_offsets: String,
    pub tuning_sort_key: i64,
    pub tuning_low_pitch: Option<i64>,
    pub bass: RoleTuning,
    pub rhythm: RoleTuning,
    pub mtime: f64,
    pub size: u64,
    pub work_key: String,
}

fn role_tuning(manifest_arrs: &[Value], p: &Perspective) -> RoleTuning {
    let mut raw = None;
    'outer: for exact in [true, false] {
        for e in manifest_arrs {
            let name = py_str(e.get("name"), "").to_lowercase();
            let Some(t) = e
                .get("tuning")
                .filter(|t| t.as_array().is_some_and(|a| !a.is_empty()))
            else {
                continue;
            };
            if (exact && name == p.role) || (!exact && name.contains(p.role)) {
                raw = Some(t.clone());
                break 'outer;
            }
        }
    }
    let Some(offsets) = raw.as_ref().and_then(|r| tunings::normalize_offsets(r, p)) else {
        return RoleTuning::default();
    };
    RoleTuning {
        name: tunings::perspective_tuning_name(&offsets, p),
        sort_key: offsets.iter().sum(),
        offsets: tunings::offsets_str(&offsets),
        key: tunings::perspective_tuning_key(&offsets, p),
        low_pitch: tunings::perspective_low_pitch(&offsets, p),
    }
}

fn norm_token(s: &str, fold_the: bool) -> String {
    let mut t: String = s
        .nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect();
    t = t.to_lowercase();
    if fold_the
        && let Some(rest) = t.strip_prefix("the")
        && rest.starts_with(char::is_whitespace)
    {
        t = rest.trim_start().to_string();
    }
    let folded: String = t
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    if !folded.is_empty() {
        return folded;
    }
    s.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn opt_digit_int(v: Option<&Value>) -> Option<i64> {
    let s = match v? {
        Value::Null => return None,
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// Metadata for one pack, from its manifest alone.
pub fn extract(filename: &str, manifest: &Value, mtime: f64, size: u64) -> Song {
    let marr: Vec<Value> = manifest
        .get("arrangements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut arrs: Vec<(i64, String)> = marr
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let name = match e.get("name") {
                Some(n) => py_str(Some(n), ""),
                None => match e.get("id") {
                    Some(id) => py_str(Some(id), ""),
                    None => format!("Arr{i}"),
                },
            };
            let pri = match name.as_str() {
                "Lead" => 0,
                "Combo" => 1,
                "Rhythm" => 2,
                "Bass" => 3,
                _ => 99,
            };
            (pri, name)
        })
        .collect();
    arrs.sort_by_key(|a| a.0);
    let names: Vec<String> = arrs.into_iter().map(|a| a.1).collect();
    let smart = chart::smart_names(&names);
    let arrangements = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            json!({
                "index": i, "name": n, "notes": 0, "smart_name": smart[i],
            })
        })
        .collect();

    let song_offsets: Vec<i64> = {
        let pick = |pred: &dyn Fn(&str) -> bool| {
            marr.iter().find_map(|e| {
                let name = py_str(e.get("name"), "").to_lowercase();
                let t = e.get("tuning")?.as_array().filter(|a| !a.is_empty())?;
                pred(&name).then(|| tunings::offsets_from_value(&Value::Array(t.clone())))?
            })
        };
        pick(&|n| ["lead", "rhythm", "combo"].contains(&n))
            .or_else(|| pick(&|_| true))
            .unwrap_or_else(|| vec![0; 6])
    };
    let lead = tunings::LEAD;
    let low = tunings::normalize_offsets(&json!(song_offsets), &lead)
        .and_then(|o| tunings::perspective_low_pitch(&o, &lead));
    let (_, inst) = chart::manifest_stems(manifest);
    let title = py_str(manifest.get("title"), "");
    let artist = py_str(manifest.get("artist"), "");
    Song {
        filename: filename.to_string(),
        work_key: format!(
            "{}|{}",
            norm_token(&artist, true),
            norm_token(&title, false)
        ),
        title,
        artist,
        album: py_str(manifest.get("album"), ""),
        year: match manifest.get("year") {
            None | Some(Value::Null) => String::new(),
            Some(Value::Number(n)) if n.as_f64() == Some(0.0) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(v) => py_str(Some(v), ""),
        },
        genre: manifest
            .get("genres")
            .and_then(Value::as_array)
            .and_then(|g| g.first())
            .map(|g| py_str(Some(g), ""))
            .unwrap_or_default(),
        track: opt_digit_int(manifest.get("track")),
        disc: opt_digit_int(manifest.get("disc")),
        duration: as_f64(manifest.get("duration"), 0.0),
        arrangements,
        has_lyrics: crate::util::truthy(manifest.get("lyrics")),
        stem_ids: inst.iter().map(|s| s.id.clone()).collect(),
        tuning_name: tunings::tuning_name(&song_offsets),
        tuning_offsets: tunings::offsets_str(&song_offsets),
        tuning_sort_key: song_offsets.iter().sum(),
        tuning_low_pitch: low,
        bass: role_tuning(&marr, &tunings::BASS),
        rhythm: role_tuning(&marr, &tunings::RHYTHM),
        mtime,
        size,
    }
}

// ── Index + scanning ─────────────────────────────────────────────────────────

#[derive(Default)]
pub struct Index {
    songs: RwLock<Vec<Song>>,
    status: RwLock<Value>,
}

impl Index {
    pub fn new() -> Self {
        let idx = Self::default();
        *idx.status.write().unwrap() = json!({"running": false, "stage": "idle", "total": 0, "done": 0,
            "current": "", "error": null, "is_first_scan": true, "added": 0, "removed": 0});
        idx
    }

    pub fn status(&self) -> Value {
        self.status.read().unwrap().clone()
    }

    pub fn all(&self) -> Vec<Song> {
        self.songs.read().unwrap().clone()
    }

    pub fn get(&self, filename: &str) -> Option<Song> {
        self.songs
            .read()
            .unwrap()
            .iter()
            .find(|s| s.filename == filename)
            .cloned()
    }

    /// Rescan the library folder. Unchanged packs reuse their cached entry.
    pub fn scan(&self, root: &Path) {
        let first = self.songs.read().unwrap().is_empty();
        let mut found = Vec::new();
        walk(root, root, &mut found, 0);
        let old: HashMap<String, Song> = self
            .songs
            .read()
            .unwrap()
            .iter()
            .map(|s| (s.filename.clone(), s.clone()))
            .collect();
        let mut fresh = Vec::new();
        let mut added = 0;
        for (key, path) in found {
            let Some((mtime, size)) = pack::stamp(&path) else {
                continue;
            };
            if let Some(s) = old.get(&key).filter(|s| s.mtime == mtime && s.size == size) {
                fresh.push(s.clone());
                continue;
            }
            let Some(manifest) = pack::read_manifest(&path) else {
                tracing::warn!("scan: unreadable manifest in {key}");
                continue;
            };
            let song = extract(&key, &manifest, mtime, size);
            if song.title.is_empty() {
                continue;
            }
            if !old.contains_key(&key) {
                added += 1;
            }
            fresh.push(song);
        }
        let removed = old
            .keys()
            .filter(|k| !fresh.iter().any(|s| &s.filename == *k))
            .count();
        if added > 0 || removed > 0 {
            tracing::info!(
                "scan: {} songs ({added} added, {removed} removed)",
                fresh.len()
            );
        }
        *self.songs.write().unwrap() = fresh;
        *self.status.write().unwrap() = json!({"running": false, "stage": "complete", "total": 0, "done": 0,
            "current": "", "error": null, "is_first_scan": first, "added": added, "removed": removed});
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>, depth: usize) {
    if depth > 12 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        tracing::warn!("scan: can't read {} (check permissions)", dir.display());
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        let Ok(ft) = std::fs::metadata(&path) else {
            continue;
        };
        if pack::is_pack(&path) {
            if let Some(key) = pack::rel_key(root, &path.canonicalize().unwrap_or(path.clone())) {
                out.push((key, path));
            }
        } else if ft.is_dir() {
            walk(root, &path, out, depth + 1);
        }
    }
}

// ── Queries ──────────────────────────────────────────────────────────────────

#[derive(Default, Debug)]
pub struct Filters {
    pub q: String,
    pub format: String,
    pub artist: String,
    pub album: String,
    pub arr_has: Vec<String>,
    pub arr_lacks: Vec<String>,
    pub stems_has: Vec<String>,
    pub stems_lacks: Vec<String>,
    pub has_lyrics: Option<bool>,
    pub tunings: Vec<String>,
    pub genre: Vec<String>,
    pub smart: bool,
    pub perspective: Option<Perspective>,
}

pub fn csv(v: Option<&String>) -> Vec<String> {
    v.map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(String::from)
            .collect()
    })
    .unwrap_or_default()
}

impl Filters {
    pub fn from_query(q: &HashMap<String, String>) -> Self {
        Self {
            q: q.get("q").cloned().unwrap_or_default(),
            format: q.get("format").cloned().unwrap_or_default(),
            artist: q.get("artist").cloned().unwrap_or_default(),
            album: q.get("album").cloned().unwrap_or_default(),
            arr_has: csv(q.get("arrangements_has")),
            arr_lacks: csv(q.get("arrangements_lacks")),
            stems_has: csv(q.get("stems_has")),
            stems_lacks: csv(q.get("stems_lacks")),
            has_lyrics: q.get("has_lyrics").and_then(|v| match v.as_str() {
                "1" | "true" => Some(true),
                "0" | "false" => Some(false),
                _ => None,
            }),
            tunings: csv(q.get("tunings")),
            genre: csv(q.get("genre")),
            smart: q.get("naming_mode").is_some_and(|m| m == "smart"),
            perspective: q.get("instrument").map(|i| tunings::perspective(i)),
        }
    }
}

/// The tuning a song shows for a perspective: its own role tuning when it has
/// one, else the song-level tuning (marked inferred).
pub fn effective_tuning(s: &Song, p: &Perspective) -> (String, String, i64, bool) {
    let role = match p.id {
        "bass" => Some(&s.bass),
        "guitar-rhythm" => Some(&s.rhythm),
        _ => None,
    };
    match role {
        Some(r) if !r.name.is_empty() => (r.name.clone(), r.offsets.clone(), r.sort_key, false),
        Some(_) => (
            s.tuning_name.clone(),
            s.tuning_offsets.clone(),
            s.tuning_sort_key,
            true,
        ),
        None => (
            s.tuning_name.clone(),
            s.tuning_offsets.clone(),
            s.tuning_sort_key,
            false,
        ),
    }
}

fn group_key(name: &str, offsets: &str) -> String {
    if name == "Custom Tuning" {
        offsets.to_string()
    } else {
        name.to_string()
    }
}

fn arr_matches(s: &Song, kind: &str, smart_mode: bool) -> bool {
    s.arrangements.iter().any(|a| {
        let name = a["name"].as_str().unwrap_or("");
        let smart = a["smart_name"].as_str();
        let starts = |v: &str| {
            v == kind
                || v.to_lowercase()
                    .starts_with(&format!("alt. {}", kind.to_lowercase()))
                || v.to_lowercase()
                    .starts_with(&format!("bonus {}", kind.to_lowercase()))
        };
        if smart_mode {
            match smart {
                Some(sn) => starts(sn),
                None => starts(name),
            }
        } else {
            name == kind
        }
    })
}

impl Filters {
    fn matches(&self, s: &Song) -> bool {
        if !self.q.is_empty() {
            let q = nocase(&self.q);
            if ![&s.title, &s.artist, &s.album]
                .iter()
                .any(|f| nocase(f).contains(&q))
            {
                return false;
            }
        }
        if !self.artist.is_empty() && nocase(&s.artist) != nocase(&self.artist) {
            return false;
        }
        if !self.album.is_empty() && nocase(&s.album) != nocase(&self.album) {
            return false;
        }
        if !self.genre.is_empty() && !self.genre.iter().any(|g| nocase(g) == nocase(&s.genre)) {
            return false;
        }
        if !self.format.is_empty() && self.format != "sloppak" {
            return false;
        }
        let allowed = ["Lead", "Rhythm", "Bass", "Combo"];
        for k in self
            .arr_has
            .iter()
            .filter(|k| allowed.contains(&k.as_str()))
        {
            let k = if self.smart && k == "Combo" {
                "Lead"
            } else {
                k.as_str()
            };
            if !arr_matches(s, k, self.smart) {
                return false;
            }
        }
        for k in self
            .arr_lacks
            .iter()
            .filter(|k| allowed.contains(&k.as_str()))
        {
            if arr_matches(s, k, self.smart) {
                return false;
            }
        }
        if self.stems_has.iter().any(|x| !s.stem_ids.contains(x)) {
            return false;
        }
        if self.stems_lacks.iter().any(|x| s.stem_ids.contains(x)) {
            return false;
        }
        if let Some(l) = self.has_lyrics
            && s.has_lyrics != l
        {
            return false;
        }
        if !self.tunings.is_empty() {
            let p = self.perspective.unwrap_or(tunings::LEAD);
            let (name, offsets, _, _) = effective_tuning(s, &p);
            let gk = group_key(&name, &offsets);
            if !self.tunings.iter().any(|t| nocase(t) == nocase(&gk)) {
                return false;
            }
        }
        true
    }
}

fn cmp_nocase(a: &str, b: &str) -> Ordering {
    nocase(a).cmp(&nocase(b))
}

fn year_int(y: &str) -> i64 {
    let digits: String = y
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse().unwrap_or(0)
}

pub fn sort_songs(songs: &mut [Song], sort: &str, dir: &str, p: &Perspective) {
    let desc = dir == "desc";
    songs.sort_by(|a, b| {
        let o = match sort {
            "artist" => cmp_nocase(&a.artist, &b.artist).then(cmp_nocase(&a.title, &b.title)),
            "artist-desc" => cmp_nocase(&b.artist, &a.artist).then(cmp_nocase(&a.title, &b.title)),
            "title" => {
                let o = cmp_nocase(&a.title, &b.title);
                if desc { o.reverse() } else { o }
            }
            "title-desc" => cmp_nocase(&b.title, &a.title),
            "recent" => b.mtime.total_cmp(&a.mtime),
            "year" | "year-desc" => (a.year.is_empty()).cmp(&b.year.is_empty()).then_with(|| {
                let o = year_int(&a.year).cmp(&year_int(&b.year));
                if sort == "year-desc" { o.reverse() } else { o }
            }),
            "tuning" => {
                let (an, _, ak, _) = effective_tuning(a, p);
                let (bn, _, bk, _) = effective_tuning(b, p);
                an.is_empty()
                    .cmp(&bn.is_empty())
                    .then(ak.abs().cmp(&bk.abs()))
                    .then(ak.cmp(&bk))
                    .then(cmp_nocase(&an, &bn))
            }
            "track" => a
                .track
                .is_none()
                .cmp(&b.track.is_none())
                .then(a.disc.unwrap_or(1).cmp(&b.disc.unwrap_or(1)))
                .then(a.track.cmp(&b.track))
                .then(cmp_nocase(&a.title, &b.title)),
            // Practice stats and difficulty live in the visitor's browser, so
            // these sorts have nothing to order by server-side.
            "mastery" | "mastery-desc" | "difficulty" | "difficulty-desc" => Ordering::Equal,
            _ => {
                let o = cmp_nocase(&a.artist, &b.artist);
                if desc { o.reverse() } else { o }
            }
        };
        o.then_with(|| a.filename.cmp(&b.filename))
    });
}

fn base_row(s: &Song) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("filename".into(), json!(s.filename));
    o.insert("title".into(), json!(s.title));
    o.insert("artist".into(), json!(s.artist));
    o.insert("album".into(), json!(s.album));
    o.insert("year".into(), json!(s.year));
    o.insert("duration".into(), num(s.duration));
    o.insert("tuning".into(), json!(s.tuning_name));
    o.insert("arrangements".into(), json!(s.arrangements));
    o.insert("has_lyrics".into(), json!(s.has_lyrics));
    o.insert("format".into(), json!("sloppak"));
    o.insert("stem_count".into(), json!(s.stem_ids.len()));
    o.insert("stem_ids".into(), json!(s.stem_ids));
    o.insert("tuning_name".into(), json!(s.tuning_name));
    o.insert("bass_tuning_name".into(), json!(s.bass.name));
    o.insert("has_estd".into(), json!(false));
    o.insert("favorite".into(), json!(false));
    o.insert("user_difficulty".into(), Value::Null);
    o
}

pub fn page_row(s: &Song, p: Option<&Perspective>) -> Map<String, Value> {
    let mut o = base_row(s);
    o.insert("mtime".into(), num(s.mtime));
    o.insert("tuning_offsets".into(), json!(s.tuning_offsets));
    o.insert("bass_tuning_offsets".into(), json!(s.bass.offsets));
    o.insert("rhythm_tuning_name".into(), json!(s.rhythm.name));
    o.insert("rhythm_tuning_offsets".into(), json!(s.rhythm.offsets));
    o.insert("tags".into(), json!([]));
    o.insert("unmatched".into(), json!(false));
    if let Some(p) = p.filter(|p| !p.prefix.is_empty()) {
        let (_, _, _, inferred) = effective_tuning(s, p);
        o.insert("tuning_perspective".into(), json!(p.id));
        o.insert("tuning_inferred".into(), json!(inferred));
    }
    o
}

fn low(v: Option<i64>) -> Value {
    v.map_or(json!(""), |x| json!(x))
}

/// `GET /api/song/{filename}` metadata.
pub fn song_info(s: &Song) -> Map<String, Value> {
    let mut o = Map::new();
    for (k, v) in [
        ("title", json!(s.title)),
        ("artist", json!(s.artist)),
        ("album", json!(s.album)),
        ("year", json!(s.year)),
        ("duration", num(s.duration)),
        ("tuning", json!(s.tuning_name)),
        ("tuning_name", json!(s.tuning_name)),
        ("tuning_offsets", json!(s.tuning_offsets)),
        ("tuning_sort_key", json!(s.tuning_sort_key)),
        ("tuning_low_pitch", low(s.tuning_low_pitch)),
        ("arrangements", json!(s.arrangements)),
        ("has_lyrics", json!(s.has_lyrics)),
        ("format", json!("sloppak")),
        ("stem_count", json!(s.stem_ids.len())),
        ("stem_ids", json!(s.stem_ids)),
    ] {
        o.insert(k.into(), v);
    }
    for (prefix, r) in [("bass_", &s.bass), ("rhythm_", &s.rhythm)] {
        o.insert(format!("{prefix}tuning_name"), json!(r.name));
        o.insert(format!("{prefix}tuning_offsets"), json!(r.offsets));
        o.insert(format!("{prefix}tuning_sort_key"), json!(r.sort_key));
        o.insert(format!("{prefix}tuning_key"), json!(r.key));
        o.insert(format!("{prefix}tuning_low_pitch"), low(r.low_pitch));
    }
    o
}

/// Collapse a work's charts to one row (the first by filename) when grouping.
fn grouped(songs: Vec<Song>, all: &[Song]) -> Vec<(Song, usize)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for s in all {
        *counts.entry(s.work_key.as_str()).or_default() += 1;
    }
    let mut reps: BTreeMap<String, String> = BTreeMap::new();
    for s in all {
        reps.entry(s.work_key.clone())
            .and_modify(|f| {
                if s.filename < *f {
                    f.clone_from(&s.filename);
                }
            })
            .or_insert_with(|| s.filename.clone());
    }
    let mut seen = std::collections::HashSet::new();
    songs
        .into_iter()
        .filter(|s| seen.insert(s.work_key.clone()))
        .map(|s| {
            let n = counts.get(s.work_key.as_str()).copied().unwrap_or(1);
            (s, n)
        })
        .collect()
}

pub fn query_page(
    all: &[Song],
    f: &Filters,
    sort: &str,
    dir: &str,
    page: usize,
    size: usize,
    group: bool,
) -> Value {
    let p = f.perspective.unwrap_or(tunings::LEAD);
    let mut hits: Vec<Song> = all.iter().filter(|s| f.matches(s)).cloned().collect();
    sort_songs(&mut hits, sort, dir, &p);
    let rows: Vec<(Song, Option<usize>)> = if group {
        grouped(hits, all)
            .into_iter()
            .map(|(s, n)| (s, Some(n)))
            .collect()
    } else {
        hits.into_iter().map(|s| (s, None)).collect()
    };
    let total = rows.len();
    let songs: Vec<Value> = rows
        .into_iter()
        .skip(page * size)
        .take(size)
        .map(|(s, n)| {
            let mut o = page_row(&s, f.perspective.as_ref());
            if let Some(n) = n {
                o.insert("work_key".into(), json!(s.work_key));
                o.insert("chart_count".into(), json!(n));
                o.insert("is_split".into(), json!(false));
            }
            Value::Object(o)
        })
        .collect();
    json!({"songs": songs, "total": total, "page": page, "size": size, "next_cursor": null})
}

fn letter_bucket(s: &str) -> String {
    match s.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase().to_string(),
        _ => "#".into(),
    }
}

pub fn query_stats(
    all: &[Song],
    f: &Filters,
    sort: &str,
    want_sort_letters: bool,
    group: bool,
) -> Value {
    let mut hits: Vec<Song> = all.iter().filter(|s| f.matches(s)).cloned().collect();
    if group {
        hits = grouped(hits, all).into_iter().map(|(s, _)| s).collect();
    }
    let mut artists: BTreeMap<String, String> = BTreeMap::new();
    for s in &hits {
        artists
            .entry(nocase(&s.artist))
            .or_insert_with(|| s.artist.clone());
    }
    let mut letters: BTreeMap<String, usize> = BTreeMap::new();
    for a in artists.values() {
        *letters.entry(letter_bucket(a)).or_default() += 1;
    }
    let mut out =
        json!({"total_songs": hits.len(), "total_artists": artists.len(), "letters": letters});
    if want_sort_letters {
        let mut sl: BTreeMap<String, usize> = BTreeMap::new();
        for s in &hits {
            let col = if sort == "title" || sort == "title-desc" {
                &s.title
            } else {
                &s.artist
            };
            *sl.entry(letter_bucket(col)).or_default() += 1;
        }
        out["sort_letters"] = json!(sl);
    }
    out
}

pub fn query_albums(all: &[Song], f: &Filters, page: usize, size: usize) -> Value {
    let mut groups: BTreeMap<(String, String), (String, String, usize, String)> = BTreeMap::new();
    let mut hits: Vec<&Song> = all
        .iter()
        .filter(|s| f.matches(s) && !s.album.is_empty())
        .collect();
    hits.sort_by(|a, b| a.filename.cmp(&b.filename));
    for s in hits {
        groups
            .entry((nocase(&s.artist), nocase(&s.album)))
            .and_modify(|g| g.2 += 1)
            .or_insert_with(|| (s.artist.clone(), s.album.clone(), 1, s.filename.clone()));
    }
    let total = groups.len();
    let albums: Vec<Value> = groups
        .into_values()
        .skip(page * size)
        .take(size)
        .map(|(artist, album, n, cover)| {
            json!({
                "artist": if artist.is_empty() { "Unknown Artist".to_string() } else { artist },
                "album": album, "count": n, "cover": cover,
            })
        })
        .collect();
    json!({"albums": albums, "total": total, "page": page, "size": size})
}

/// (lowercase key, display name, albums of (lowercase key, name, songs)).
type ArtistGroup = (String, String, Vec<(String, String, Vec<Value>)>);

pub fn query_artists(all: &[Song], f: &Filters, letter: &str, page: usize, size: usize) -> Value {
    let hits: Vec<&Song> = all
        .iter()
        .filter(|s| f.matches(s))
        .filter(|s| match letter {
            "" => true,
            "#" => !s.artist.starts_with(|c: char| c.is_ascii_alphabetic()),
            l => s
                .artist
                .chars()
                .next()
                .is_some_and(|c| c.to_uppercase().to_string() == l.to_uppercase()),
        })
        .collect();
    let mut names: Vec<String> = hits.iter().map(|s| nocase(&s.artist)).collect();
    names.sort();
    names.dedup();
    let total = names.len();
    let page_names: Vec<String> = names.into_iter().skip(page * size).take(size).collect();
    let mut rows: Vec<&Song> = hits
        .into_iter()
        .filter(|s| page_names.contains(&nocase(&s.artist)))
        .collect();
    rows.sort_by(|a, b| {
        cmp_nocase(&a.artist, &b.artist)
            .then(cmp_nocase(&a.album, &b.album))
            .then(cmp_nocase(&a.title, &b.title))
            .then(a.filename.cmp(&b.filename))
    });
    let mut artists: Vec<ArtistGroup> = Vec::new();
    for s in rows {
        let artist = if s.artist.is_empty() {
            "Unknown Artist".to_string()
        } else {
            s.artist.clone()
        };
        let album = if s.album.is_empty() {
            "Unknown Album".to_string()
        } else {
            s.album.clone()
        };
        let akey = artist.to_lowercase();
        if artists.last().is_none_or(|a| a.0 != akey) {
            artists.push((akey, artist, Vec::new()));
        }
        let albums = &mut artists.last_mut().unwrap().2;
        let bkey = album.to_lowercase();
        if albums.last().is_none_or(|b| b.0 != bkey) {
            albums.push((bkey, album, Vec::new()));
        }
        albums
            .last_mut()
            .unwrap()
            .2
            .push(Value::Object(base_row(s)));
    }
    let out: Vec<Value> = artists.into_iter().map(|(_, name, albums)| {
        let song_count: usize = albums.iter().map(|a| a.2.len()).sum();
        json!({
            "name": name, "album_count": albums.len(), "song_count": song_count,
            "albums": albums.into_iter().map(|(_, n, songs)| json!({"name": n, "songs": songs})).collect::<Vec<_>>(),
        })
    }).collect();
    json!({"artists": out, "total_artists": total, "page": page, "size": size})
}

pub fn genres(all: &[Song]) -> Value {
    let mut g: BTreeMap<String, String> = BTreeMap::new();
    for s in all.iter().filter(|s| !s.genre.is_empty()) {
        g.entry(nocase(&s.genre)).or_insert_with(|| s.genre.clone());
    }
    json!({"genres": g.into_values().collect::<Vec<_>>()})
}

pub fn tuning_names(all: &[Song], p: &Perspective) -> Value {
    struct G {
        name: String,
        key: String,
        sort: i64,
        count: usize,
        offs: String,
        inferred: usize,
    }
    let mut groups: Vec<G> = Vec::new();
    for s in all.iter().filter(|s| !s.title.is_empty()) {
        let (name, offs, sk, inferred) = effective_tuning(s, p);
        if name.is_empty() {
            continue;
        }
        let gk = group_key(&name, &offs);
        if let Some(g) = groups.iter_mut().find(|g| nocase(&g.key) == nocase(&gk)) {
            g.count += 1;
            g.sort = g.sort.min(sk);
            if offs < g.offs {
                g.offs = offs;
            }
            g.inferred += usize::from(inferred);
        } else {
            groups.push(G {
                name,
                key: gk,
                sort: sk,
                count: 1,
                offs,
                inferred: usize::from(inferred),
            });
        }
    }
    groups.sort_by(|a, b| {
        a.sort
            .abs()
            .cmp(&b.sort.abs())
            .then(a.sort.cmp(&b.sort))
            .then(cmp_nocase(&a.name, &b.name))
    });
    json!({
        "instrument": p.id,
        "tunings": groups.into_iter().map(|g| json!({
            "name": g.name, "key": g.key, "offsets": g.offs, "sort_key": g.sort,
            "count": g.count, "inferred_count": g.inferred,
        })).collect::<Vec<_>>(),
    })
}

pub fn work_charts(all: &[Song], work_key: &str) -> Value {
    let mut members: Vec<&Song> = all.iter().filter(|s| s.work_key == work_key).collect();
    members.sort_by(|a, b| a.filename.cmp(&b.filename));
    let keeper = members.first().map(|s| s.filename.clone());
    json!({
        "work_key": work_key,
        "preferred": null,
        "keeper": keeper,
        "charts": members.iter().map(|s| Value::Object(page_row(s, None))).collect::<Vec<_>>(),
    })
}

pub fn providers() -> Value {
    json!({"providers": [{
        "capabilities": ["art.read", "favorite.write", "library.read", "metadata.write", "song.play"],
        "default": true, "id": "local", "kind": "local", "label": "My Library", "owner_plugin_id": null,
    }]})
}
