//! `GET /api/settings`: player settings computed, never stored, server-side.
//!
//! Server settings come from the environment and the operator's config file
//! only. Player settings live in each visitor's browser
//! (static/riffboard/local-api.js): the browser sends its saved `state` and an
//! optional `patch` as query parameters and gets back the normalised settings
//! plus the new state (`_state`) to keep. Nothing is written here.

use serde_json::{Map, Value, json};

use crate::tunings::{self, PROFILE_IDS, PROFILE_PATHWAYS};
use crate::util::num;

const PLAYER_KEYS: [&str; 13] = [
    "default_arrangement",
    "master_difficulty",
    "av_offset_ms",
    "countdown_before_song",
    "miss_penalty",
    "fail_behavior",
    "instrument_profiles",
    "active_instrument_profile",
    "instrument",
    "string_count",
    "tuning",
    "reference_pitch",
    "pathway",
];
const PROFILE_KEYS: [&str; 7] = [
    "instrument",
    "string_count",
    "tuning",
    "reference_pitch",
    "pathway",
    "instrument_profiles",
    "active_instrument_profile",
];
const MAX_PARAM: usize = 8192;

fn number(v: &Value, lo: f64, hi: f64, name: &str) -> Result<f64, String> {
    let err = || {
        format!(
            "{name} must be a number between {} and {}",
            fmt(lo),
            fmt(hi)
        )
    };
    let f = match v {
        Value::Bool(_) => return Err(err()),
        Value::Number(n) => n.as_f64().ok_or_else(err)?,
        Value::String(s) => s.trim().parse::<f64>().map_err(|_| err())?,
        _ => return Err(err()),
    };
    if !f.is_finite() {
        return Err(err());
    }
    Ok(f.clamp(lo, hi))
}

fn fmt(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{x:.0}")
    } else {
        x.to_string()
    }
}

fn present<'a>(d: &'a Map<String, Value>, k: &str) -> Option<&'a Value> {
    d.get(k).filter(|v| !v.is_null())
}

/// (flat updates, instrument-profile patch).
type Validated = (Map<String, Value>, Option<Map<String, Value>>);

/// Validate a change (player keys only). Returns (flat updates, profile patch).
fn validate_patch(d: &Map<String, Value>) -> Result<Validated, String> {
    let mut u = Map::new();
    if let Some(v) = present(d, "default_arrangement") {
        let s = v
            .as_str()
            .filter(|s| s.chars().count() <= 64)
            .ok_or("default_arrangement must be a short string")?;
        u.insert("default_arrangement".into(), json!(s));
    }
    if let Some(v) = present(d, "master_difficulty") {
        u.insert(
            "master_difficulty".into(),
            json!(number(v, 0.0, 100.0, "master_difficulty")?.trunc() as i64),
        );
    }
    if let Some(v) = present(d, "av_offset_ms") {
        u.insert(
            "av_offset_ms".into(),
            num(number(v, -1000.0, 1000.0, "av_offset_ms")?),
        );
    }
    if let Some(v) = present(d, "countdown_before_song") {
        let b = v
            .as_bool()
            .ok_or("countdown_before_song must be a boolean")?;
        u.insert("countdown_before_song".into(), json!(b));
    }
    if let Some(v) = present(d, "miss_penalty") {
        if !v
            .as_str()
            .is_some_and(|s| ["none", "low", "medium", "high"].contains(&s))
        {
            return Err("miss_penalty must be one of none, low, medium, high".into());
        }
        u.insert("miss_penalty".into(), v.clone());
    }
    if let Some(v) = present(d, "fail_behavior") {
        if !v
            .as_str()
            .is_some_and(|s| ["continue", "restart", "stop"].contains(&s))
        {
            return Err("fail_behavior must be continue, restart or stop".into());
        }
        u.insert("fail_behavior".into(), v.clone());
    }
    if let Some(v) = present(d, "reference_pitch") {
        u.insert(
            "reference_pitch".into(),
            num(number(v, 430.0, 450.0, "reference_pitch")?),
        );
    }
    if let Some(v) = present(d, "instrument") {
        if !v.as_str().is_some_and(|s| s == "guitar" || s == "bass") {
            return Err("instrument must be 'guitar' or 'bass'".into());
        }
        u.insert("instrument".into(), v.clone());
    }
    if let Some(v) = present(d, "string_count") {
        let n = match v {
            Value::Number(n) if n.as_i64().is_some() => n.as_i64(),
            Value::Number(n) => n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64),
            _ => None,
        }
        .filter(|n| (4..=8).contains(n))
        .ok_or("string_count must be an integer between 4 and 8")?;
        u.insert("string_count".into(), json!(n));
    }
    if let Some(v) = present(d, "tuning") {
        match v {
            Value::String(s) if s.chars().count() > 64 => return Err("tuning name too long".into()),
            Value::String(_) => {}
            Value::Array(a) => {
                let ok = a.len() <= 8
                    && a.iter().all(|o| {
                        !o.is_boolean() && o.as_i64().is_some_and(|x| (-12..=12).contains(&x))
                    });
                if !ok {
                    return Err("tuning offsets must be ≤8 integers between -12 and 12".into());
                }
            }
            _ => return Err("tuning must be a name or a list of semitone offsets".into()),
        }
        u.insert("tuning".into(), v.clone());
    }
    if let Some(v) = present(d, "pathway") {
        if !v.as_str().is_some_and(|s| PROFILE_PATHWAYS.contains(&s)) {
            return Err("unknown pathway".into());
        }
        u.insert("pathway".into(), v.clone());
    }
    let mut profile_patch = None;
    if let Some(v) = present(d, "instrument_profiles") {
        let o = v
            .as_object()
            .ok_or("instrument_profiles must be an object")?;
        let mut pp = Map::new();
        for (id, raw) in o {
            if !PROFILE_IDS.contains(&id.as_str()) {
                return Err(format!("unknown instrument profile: {id}"));
            }
            pp.insert(
                id.clone(),
                Value::Object(tunings::normalize_profile(id, Some(raw))?),
            );
        }
        profile_patch = Some(pp);
    }
    if let Some(v) = present(d, "active_instrument_profile") {
        if !v.as_str().is_some_and(|s| PROFILE_IDS.contains(&s)) {
            return Err("unknown instrument profile".into());
        }
        u.insert("active_instrument_profile".into(), v.clone());
    }
    Ok((u, profile_patch))
}

fn apply(
    state: &Map<String, Value>,
    patch: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let (updates, profile_patch) = validate_patch(patch)?;
    let mut cfg = state.clone();
    cfg.extend(updates.clone());
    if let Some(pp) = profile_patch {
        let mut existing =
            tunings::normalize_profiles(cfg.get("instrument_profiles")).unwrap_or_default();
        existing.extend(pp);
        cfg.insert("instrument_profiles".into(), Value::Object(existing));
    }
    if cfg.contains_key("instrument_profiles")
        || PROFILE_KEYS.iter().any(|k| updates.contains_key(*k))
    {
        cfg = tunings::apply_flat_patch(&cfg, &updates)?;
        cfg = tunings::settings_with_profiles(&cfg);
    }
    cfg.retain(|k, _| PLAYER_KEYS.contains(&k.as_str()));
    Ok(cfg)
}

fn defaults(config: &Value) -> Map<String, Value> {
    let mut out = json!({
        "countdown_before_song": true, "miss_penalty": "none", "fail_behavior": "continue",
        "master_difficulty": 100, "av_offset_ms": 0, "default_arrangement": "",
    })
    .as_object()
    .cloned()
    .unwrap();
    if let Some(extra) = config.get("player_defaults").and_then(Value::as_object) {
        for (k, v) in extra {
            if PLAYER_KEYS.contains(&k.as_str()) {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

fn parse(raw: Option<&String>, name: &str) -> Result<Map<String, Value>, String> {
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return Ok(Map::new());
    };
    if raw.len() > MAX_PARAM {
        return Err(format!("{name} is too large"));
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(o)) => Ok(o),
        Ok(_) => Err(format!("{name} must be an object")),
        Err(_) => Err(format!("{name} must be JSON")),
    }
}

/// Returns Ok(settings) or Err(message) for a 400.
pub fn compute(
    config: &Value,
    state: Option<&String>,
    patch: Option<&String>,
) -> Result<Value, String> {
    let st = parse(state, "state")?;
    let player_state: Map<String, Value> = st
        .into_iter()
        .filter(|(k, _)| PLAYER_KEYS.contains(&k.as_str()))
        .collect();
    // A saved state is re-validated as a patch over nothing, so a tampered
    // localStorage value can't smuggle in anything invalid.
    let stored = apply(&Map::new(), &player_state).unwrap_or_default();
    let new_state = match patch.filter(|p| !p.is_empty()) {
        Some(_) => apply(&stored, &parse(patch, "patch")?)?,
        None => stored,
    };
    let mut merged = defaults(config);
    merged.extend(new_state.clone());
    let mut out = tunings::settings_with_profiles(&merged);
    out.insert("_state".into(), Value::Object(new_state));
    Ok(Value::Object(out))
}
