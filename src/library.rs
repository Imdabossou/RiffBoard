//! The song library: an in-memory index of the packs in the library folder,
//! rebuilt by a cheap manifest-only scan (unchanged packs are skipped).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::{Value, json};

use crate::chart;
use crate::pack;
use crate::util::{as_f64, nocase, py_str};

#[derive(Clone, Debug)]
pub struct Song {
    pub filename: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    /// Playable arrangement names, by arrangement index.
    pub arrangements: Vec<String>,
    pub mtime: f64,
    pub size: u64,
}

/// Metadata for one pack, from its manifest alone.
pub fn extract(filename: &str, manifest: &Value, mtime: f64, size: u64) -> Song {
    Song {
        filename: filename.to_string(),
        title: py_str(manifest.get("title"), ""),
        artist: py_str(manifest.get("artist"), ""),
        album: py_str(manifest.get("album"), ""),
        duration: as_f64(manifest.get("duration"), 0.0).max(0.0),
        arrangements: chart::playable_entries(manifest)
            .iter()
            .enumerate()
            .map(|(i, e)| chart::entry_name(e, i))
            .collect(),
        mtime,
        size,
    }
}

impl Song {
    /// A library row (`GET /api/library`).
    pub fn row(&self) -> Value {
        json!({"filename": self.filename, "title": self.title, "artist": self.artist,
               "album": self.album, "duration": self.duration})
    }

    /// The song page's details (`GET /api/song/{filename}`).
    pub fn details(&self) -> Value {
        let arrangements: Vec<Value> = self
            .arrangements
            .iter()
            .enumerate()
            .map(|(i, n)| json!({"index": i, "name": n}))
            .collect();
        json!({"filename": self.filename, "title": self.title, "artist": self.artist,
               "album": self.album, "duration": self.duration, "arrangements": arrangements})
    }
}

// ── Index + scanning ─────────────────────────────────────────────────────────

#[derive(Default)]
pub struct Index {
    songs: RwLock<Vec<Song>>,
}

impl Index {
    pub fn new() -> Self {
        Self::default()
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
        fresh.sort_by(|a, b| {
            (nocase(&a.artist), nocase(&a.title), &a.filename).cmp(&(
                nocase(&b.artist),
                nocase(&b.title),
                &b.filename,
            ))
        });
        *self.songs.write().unwrap() = fresh;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_lists_playable_parts() {
        let m = json!({"title": "T", "artist": "A", "duration": "x",
            "arrangements": [{"name": "Bass", "file": "b.json"}, {"type": "drums", "file": "d.json"}]});
        let s = extract("A/t.feedpak", &m, 1.0, 2);
        assert_eq!(s.arrangements, vec!["Bass"]);
        assert_eq!(s.duration, 0.0);
        assert_eq!(
            s.details()["arrangements"],
            json!([{"index": 0, "name": "Bass"}])
        );
    }
}
