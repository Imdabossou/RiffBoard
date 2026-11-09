//! Feedpak access: library-path containment, manifests, cover art, and the
//! unpack cache for zip-form packs.
//!
//! Every path that comes from a request or a manifest goes through
//! `resolve_in` / `resolve_member`, which reject absolute paths, `..`, NUL and
//! backslashes and then check the canonical result stays inside its root, so a
//! crafted request or pack can't read outside the library.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PACK_EXTS: [&str; 2] = [".feedpak", ".sloppak"];

/// Zip-bomb / abuse limits for unpacking a pack.
const MAX_ENTRIES: usize = 4000;
const MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAX_COVER_BYTES: u64 = 20 * 1024 * 1024;

pub fn is_pack(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    PACK_EXTS.iter().any(|e| name.ends_with(e))
}

/// Split a relative path into safe components, or None if it can't be safe.
fn safe_components(rel: &str) -> Option<Vec<String>> {
    if rel.is_empty() || rel.contains('\0') || rel.contains('\\') {
        return None;
    }
    let p = Path::new(rel);
    let mut out = Vec::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => out.push(s.to_str()?.to_string()),
            Component::CurDir => {}
            _ => return None, // RootDir, Prefix, ParentDir
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Resolve `rel` under `root`, requiring the canonical target to stay inside
/// the canonical root (symlinks included). The target must exist.
pub fn resolve_in(root: &Path, rel: &str) -> Option<PathBuf> {
    let parts = safe_components(rel)?;
    let mut p = root.to_path_buf();
    for c in &parts {
        p.push(c);
    }
    let canon_root = root.canonicalize().ok()?;
    let canon = p.canonicalize().ok()?;
    canon.starts_with(&canon_root).then_some(canon)
}

/// Zip member name for a manifest-relative path, or None if unsafe.
fn member_name(rel: &str) -> Option<String> {
    Some(safe_components(rel)?.join("/"))
}

/// Library-relative key for an absolute path inside the library.
pub fn rel_key(root: &Path, path: &Path) -> Option<String> {
    let canon_root = root.canonicalize().ok()?;
    let rel = path.strip_prefix(&canon_root).ok()?;
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub fn parse_jsonc(text: &str) -> Result<Value, serde_json::Error> {
    // String-aware comment stripping (feedpak-spec §8).
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(b.len());
                out.push_str(&text[start..i]);
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            _ => {
                let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&text[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    serde_json::from_str(&out)
}

fn parse_json_file(path: &Path) -> Option<Value> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_JSON_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let is_jsonc = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("jsonc"));
    if is_jsonc {
        parse_jsonc(&text).ok()
    } else {
        serde_json::from_str(&text).ok()
    }
}

/// Read a manifest-referenced JSON file inside an (unpacked) pack directory.
pub fn read_pack_json(source_dir: &Path, rel: &str) -> Option<Value> {
    let p = resolve_in(source_dir, rel)?;
    parse_json_file(&p)
}

fn yaml_value(y: &yaml_rust2::Yaml) -> Value {
    use yaml_rust2::Yaml;
    match y {
        Yaml::Real(r) => r
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::Null, Value::Number),
        Yaml::Integer(i) => Value::from(*i),
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Array(a) => Value::Array(a.iter().map(yaml_value).collect()),
        Yaml::Hash(h) => Value::Object(
            h.iter()
                .filter_map(|(k, v)| {
                    let key = match k {
                        Yaml::String(s) => s.clone(),
                        Yaml::Integer(i) => i.to_string(),
                        Yaml::Real(r) => r.clone(),
                        Yaml::Boolean(b) => {
                            if *b {
                                "True".into()
                            } else {
                                "False".into()
                            }
                        }
                        _ => return None,
                    };
                    Some((key, yaml_value(v)))
                })
                .collect(),
        ),
        Yaml::Alias(_) | Yaml::Null | Yaml::BadValue => Value::Null,
    }
}

/// A manifest is YAML whose top level must be a mapping.
fn yaml_to_json(text: &str) -> Option<Value> {
    let docs = yaml_rust2::YamlLoader::load_from_str(text).ok()?;
    let v = yaml_value(docs.first()?);
    v.is_object().then_some(v)
}

fn open_zip(path: &Path) -> Option<zip::ZipArchive<fs::File>> {
    zip::ZipArchive::new(fs::File::open(path).ok()?).ok()
}

fn read_zip_member(zip: &mut zip::ZipArchive<fs::File>, name: &str, cap: u64) -> Option<Vec<u8>> {
    let f = zip.by_name(name).ok()?;
    if f.size() > cap {
        return None;
    }
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.take(cap + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= cap).then_some(buf)
}

/// The pack's manifest (manifest.yaml / manifest.yml) as JSON, from a zip or a
/// directory, without unpacking anything else.
pub fn read_manifest(pack: &Path) -> Option<Value> {
    if pack.is_dir() {
        for name in ["manifest.yaml", "manifest.yml"] {
            let p = pack.join(name);
            match fs::metadata(&p) {
                Ok(m) if m.is_file() => {
                    if m.len() > MAX_MANIFEST_BYTES {
                        return None;
                    }
                    return yaml_to_json(&fs::read_to_string(&p).ok()?);
                }
                _ => continue,
            }
        }
        return None;
    }
    let mut z = open_zip(pack)?;
    for name in ["manifest.yaml", "manifest.yml"] {
        if let Some(bytes) = read_zip_member(&mut z, name, MAX_MANIFEST_BYTES) {
            return yaml_to_json(&String::from_utf8(bytes).ok()?);
        }
    }
    None
}

fn cover_media_type(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

/// `(bytes, media_type)` of the pack's cover: manifest `cover`, else cover.jpg.
pub fn read_cover(pack: &Path, manifest: Option<&Value>) -> Option<(Vec<u8>, &'static str)> {
    let rel = manifest
        .and_then(|m| m.get("cover"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("cover.jpg")
        .to_string();
    if pack.is_dir() {
        let p = resolve_in(pack, &rel)?;
        let meta = fs::metadata(&p).ok()?;
        if !meta.is_file() || meta.len() > MAX_COVER_BYTES {
            return None;
        }
        return Some((fs::read(&p).ok()?, cover_media_type(&rel)));
    }
    let member = member_name(&rel)?;
    let mut z = open_zip(pack)?;
    let data = read_zip_member(&mut z, &member, MAX_COVER_BYTES)?;
    Some((data, cover_media_type(&member)))
}

/// File stamp used for cache freshness: (mtime, size). For directory-form
/// packs, the newest of the files an edit touches.
pub fn stamp(pack: &Path) -> Option<(f64, u64)> {
    let meta = fs::metadata(pack).ok()?;
    let to_secs = |m: &fs::Metadata| {
        m.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    };
    if !meta.is_dir() {
        return Some((to_secs(&meta), meta.len()));
    }
    let mut newest = 0.0_f64;
    let mut size = 0_u64;
    let mut visit = |p: PathBuf| {
        if let Ok(m) = fs::symlink_metadata(&p)
            && m.is_file()
        {
            newest = newest.max(to_secs(&m));
            size += m.len();
        }
    };
    for name in ["manifest.yaml", "manifest.yml", "drum_tab.json"] {
        visit(pack.join(name));
    }
    if let Ok(rd) = fs::read_dir(pack.join("arrangements")) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                visit(e.path());
            }
        }
    }
    if size == 0 {
        return Some((to_secs(&meta), 0));
    }
    Some((newest, size))
}

// ── Unpack cache (zip packs are extracted once, then served from disk) ───────

pub struct UnpackCache {
    root: PathBuf,
    cap_bytes: u64,
    /// key -> (source dir, mtime, size) of the pack it was made from
    known: Mutex<HashMap<String, (PathBuf, f64, u64)>>,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl UnpackCache {
    pub fn new(root: PathBuf, cap_bytes: u64) -> Self {
        let _ = fs::create_dir_all(&root);
        Self {
            root,
            cap_bytes,
            known: Mutex::default(),
            locks: Mutex::default(),
        }
    }

    fn lock_for(&self, key: &str) -> Arc<Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(key.to_string())
            .or_default()
            .clone()
    }

    /// The directory holding the pack's files: the pack itself for directory
    /// packs, else a freshly-or-previously extracted copy of the zip.
    pub fn source_dir(&self, key: &str, pack: &Path) -> Option<PathBuf> {
        if pack.is_dir() {
            return Some(pack.to_path_buf());
        }
        let (mtime, size) = stamp(pack)?;
        if let Some((dir, m, s)) = self.known.lock().unwrap().get(key)
            && *m == mtime
            && *s == size
            && dir.is_dir()
        {
            touch(dir);
            return Some(dir.clone());
        }
        let lock = self.lock_for(key);
        let _guard = lock.lock().unwrap();
        let digest = {
            let mut h = Sha256::new();
            h.update(key.as_bytes());
            h.update(format!("|{mtime}|{size}").as_bytes());
            crate::util::hex(&h.finalize())[..24].to_string()
        };
        let dir = self.root.join(&digest);
        if !dir.join(".complete").is_file() {
            let tmp = self
                .root
                .join(format!(".{digest}.{}", crate::util::random_token(6)));
            let ok = extract_zip(pack, &tmp);
            if !ok {
                let _ = fs::remove_dir_all(&tmp);
                return None;
            }
            let _ = fs::write(tmp.join(".complete"), b"");
            let _ = fs::remove_dir_all(&dir);
            if fs::rename(&tmp, &dir).is_err() {
                let _ = fs::remove_dir_all(&tmp);
                if !dir.join(".complete").is_file() {
                    return None;
                }
            }
            self.evict(&dir);
        }
        touch(&dir);
        self.known
            .lock()
            .unwrap()
            .insert(key.to_string(), (dir.clone(), mtime, size));
        Some(dir)
    }

    /// Keep the cache under its byte cap, oldest-used first.
    fn evict(&self, keep: &Path) {
        let Ok(rd) = fs::read_dir(&self.root) else {
            return;
        };
        let mut dirs: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
            .flatten()
            .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| {
                let p = e.path();
                let used = fs::metadata(p.join(".complete"))
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (used, dir_size(&p), p)
            })
            .collect();
        let mut total: u64 = dirs.iter().map(|d| d.1).sum();
        dirs.sort_by_key(|d| d.0);
        for (_, size, p) in dirs {
            if total <= self.cap_bytes {
                break;
            }
            if p == keep {
                continue;
            }
            if fs::remove_dir_all(&p).is_ok() {
                total = total.saturating_sub(size);
                self.known.lock().unwrap().retain(|_, v| v.0 != p);
            }
        }
    }
}

fn touch(dir: &Path) {
    let marker = dir.join(".complete");
    if let Ok(f) = fs::OpenOptions::new().write(true).open(&marker) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

fn dir_size(p: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            if let Ok(m) = fs::symlink_metadata(e.path()) {
                if m.is_dir() {
                    total += dir_size(&e.path());
                } else {
                    total += m.len();
                }
            }
        }
    }
    total
}

/// Extract a zip pack into `dest`, refusing unsafe names, symlinks and
/// archives that would expand beyond the limits.
fn extract_zip(pack: &Path, dest: &Path) -> bool {
    let Some(mut z) = open_zip(pack) else {
        return false;
    };
    if z.len() > MAX_ENTRIES || fs::create_dir_all(dest).is_err() {
        return false;
    }
    let mut written: u64 = 0;
    for i in 0..z.len() {
        let Ok(mut f) = z.by_index(i) else {
            return false;
        };
        if f.is_symlink() {
            continue;
        }
        let Some(rel) = f.enclosed_name() else {
            continue;
        };
        let Some(rel_str) = rel.to_str() else {
            continue;
        };
        let Some(parts) = safe_components(rel_str) else {
            continue;
        };
        let mut out = dest.to_path_buf();
        for p in &parts {
            out.push(p);
        }
        if f.is_dir() {
            if fs::create_dir_all(&out).is_err() {
                return false;
            }
            continue;
        }
        if let Some(parent) = out.parent()
            && fs::create_dir_all(parent).is_err()
        {
            return false;
        }
        let remaining = MAX_UNPACKED_BYTES.saturating_sub(written);
        let Ok(mut file) = fs::File::create(&out) else {
            return false;
        };
        match std::io::copy(&mut (&mut f).take(remaining + 1), &mut file) {
            Ok(n) => {
                written += n;
                if written > MAX_UNPACKED_BYTES {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_paths_are_rejected() {
        for bad in ["", "../x", "/etc/passwd", "a/../../b", "a\\b", "a\0b"] {
            assert!(safe_components(bad).is_none(), "{bad:?}");
        }
        assert_eq!(safe_components("./a/b.json").unwrap(), vec!["a", "b.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_in_stays_inside_the_root() {
        let base = std::env::temp_dir().join(format!("rb-resolve-{}", std::process::id()));
        let root = base.join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/ok.txt"), "x").unwrap();
        std::fs::write(base.join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(base.join("secret.txt"), root.join("escape.txt")).unwrap();
        assert!(resolve_in(&root, "sub/ok.txt").is_some());
        for bad in ["../secret.txt", "sub/../../secret.txt", "escape.txt", "/etc/passwd"] {
            assert!(resolve_in(&root, bad).is_none(), "{bad:?}");
        }
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn jsonc_comments_are_stripped_outside_strings() {
        let v = parse_jsonc("{\"a\": \"x//y\", // c\n \"b\": /* z */ 2}").unwrap();
        assert_eq!(v["a"], "x//y");
        assert_eq!(v["b"], 2);
    }
}
