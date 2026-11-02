//! Per-deployment branding: `<CONFIG_DIR>/site/site.json` plus images next to
//! it. Lets one RiffBoard install present itself as, say, a band's play-along
//! site (name, tagline, hero copy, colours, logo, about text and links) without
//! any of that living in the code repository.
//!
//! Everything is validated: colours must be `#rrggbb`, links http(s), text is
//! plain text (the browser inserts it with textContent / escaping), images are
//! raster files inside the site folder (no SVG: it could carry script).

use std::path::Path;

use serde_json::{Map, Value, json};

const COLOR_KEYS: [&str; 11] = [
    "bg",
    "sidebar",
    "card",
    "cardMuted",
    "primary",
    "primaryHi",
    "accent",
    "text",
    "textDim",
    "border",
    "gold",
];
const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "ico"];

pub fn is_asset_name(name: &str) -> bool {
    let ok_chars = !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    ok_chars && IMAGE_EXTS.contains(&ext.as_str())
}

fn text(v: Option<&Value>, max: usize) -> Option<String> {
    let s = v?.as_str()?.trim();
    (!s.is_empty()).then(|| s.chars().take(max).collect())
}

fn hex_rgb(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(format!("{} {} {}", p(0)?, p(2)?, p(4)?))
}

fn http_url(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    (s.len() <= 300 && (s.starts_with("https://") || s.starts_with("http://")))
        .then(|| s.to_string())
}

fn image(dir: &Path, v: Option<&Value>) -> Option<String> {
    let name = v?.as_str()?.trim();
    (is_asset_name(name) && dir.join(name).is_file()).then(|| format!("/site/{name}"))
}

fn load(config_dir: &Path) -> (std::path::PathBuf, Value) {
    let dir = config_dir.join("site");
    let raw = std::fs::read_to_string(dir.join("site.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    (dir, raw)
}

/// `GET /api/site`: the validated branding (empty object when unbranded).
pub fn public_json(config_dir: &Path) -> Value {
    let (dir, raw) = load(config_dir);
    let mut o = Map::new();
    for (k, max) in [
        ("name", 40),
        ("tagline", 120),
        ("hero_title", 80),
        ("hero_text", 400),
        ("about_title", 80),
    ] {
        if let Some(t) = text(raw.get(k), max) {
            o.insert(k.into(), json!(t));
        }
    }
    for k in ["logo", "hero_image", "favicon"] {
        if let Some(u) = image(&dir, raw.get(k)) {
            o.insert(k.into(), json!(u));
        }
    }
    if let Some(paras) = raw.get("about").and_then(Value::as_array) {
        let p: Vec<String> = paras
            .iter()
            .filter_map(|x| text(Some(x), 1000))
            .take(10)
            .collect();
        if !p.is_empty() {
            o.insert("about".into(), json!(p));
        }
    }
    if let Some(links) = raw.get("links").and_then(Value::as_array) {
        let l: Vec<Value> = links
            .iter()
            .filter_map(|x| {
                Some(json!({"label": text(x.get("label"), 40)?, "url": http_url(x.get("url"))?}))
            })
            .take(12)
            .collect();
        if !l.is_empty() {
            o.insert("links".into(), json!(l));
        }
    }
    Value::Object(o)
}

fn vars(theme: Option<&Value>) -> String {
    let Some(t) = theme.and_then(Value::as_object) else {
        return String::new();
    };
    COLOR_KEYS
        .iter()
        .filter_map(|k| Some(format!("    --fb-{k}: {};\n", hex_rgb(t.get(*k)?)?)))
        .collect()
}

/// `GET /site.css`: colour overrides for dark and light themes.
pub fn css(config_dir: &Path) -> String {
    let (_, raw) = load(config_dir);
    let dark = vars(raw.get("dark"));
    let light = vars(raw.get("light"));
    let mut out = String::from("/* Site branding (CONFIG_DIR/site/site.json). */\n");
    if !dark.is_empty() {
        out += &format!(":root,\n#player,\n.rb-on-dark {{\n{dark}}}\n");
    }
    if !light.is_empty() {
        out += &format!(
            "@media (prefers-color-scheme: light) {{\n  :root:not([data-theme=\"dark\"]) {{\n{light}  }}\n}}\n:root[data-theme=\"light\"] {{\n{light}}}\n"
        );
        // Keep the player and dark surfaces on the dark palette in light mode.
        if !dark.is_empty() {
            out += &format!(
                "@media (prefers-color-scheme: light) {{\n  :root:not([data-theme=\"dark\"]) #player,\n  :root:not([data-theme=\"dark\"]) .rb-on-dark {{\n{dark}  }}\n}}\n:root[data-theme=\"light\"] #player,\n:root[data-theme=\"light\"] .rb-on-dark {{\n{dark}}}\n"
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        assert_eq!(hex_rgb(&json!("#ff6fb5")).as_deref(), Some("255 111 181"));
        assert!(hex_rgb(&json!("red; } body { x")).is_none());
        assert!(hex_rgb(&json!("#fff")).is_none());
        assert!(is_asset_name("hero.jpg"));
        for bad in ["logo.svg", "../x.png", ".hidden.png", "a/b.png", "x.html"] {
            assert!(!is_asset_name(bad), "{bad}");
        }
        assert!(http_url(Some(&json!("javascript:alert(1)"))).is_none());
    }
}
