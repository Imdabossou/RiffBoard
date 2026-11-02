//! Small helpers shared across modules: Python-compatible rounding and JSON
//! coercions that mirror the original server's permissive parsing.

use serde_json::Value;

/// Python's `round(x, n)`: correctly rounded to `n` decimals, ties to even on
/// the exact binary value. Rust's `{:.n}` formatting rounds the same way.
pub fn round_to(x: f64, n: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.n$}").parse().unwrap_or(x)
}

/// JSON number for a float that should serialise like Python (`40.0`, not `40`).
pub fn num(x: f64) -> Value {
    serde_json::Number::from_f64(if x.is_finite() { x } else { 0.0 })
        .map(Value::Number)
        .unwrap_or(Value::from(0))
}

/// `float(v)` with a default: numbers, numeric strings and bools as Python does.
pub fn as_f64(v: Option<&Value>, default: f64) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(default),
        _ => default,
    }
}

/// `int(v)` with a default (truncating floats like Python's `int()`).
pub fn as_i64(v: Option<&Value>, default: i64) -> i64 {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| {
                n.as_f64()
                    .filter(|f| f.is_finite())
                    .map(|f| f.trunc() as i64)
            })
            .unwrap_or(default),
        Some(Value::Bool(b)) => i64::from(*b),
        Some(Value::String(s)) => s.trim().parse::<i64>().unwrap_or(default),
        _ => default,
    }
}

/// `_wire_int_optional`: null/blank/invalid -> default, else int (via float).
pub fn opt_i64(v: Option<&Value>, default: i64) -> i64 {
    match v {
        None | Some(Value::Null) => default,
        Some(Value::String(s)) if s.trim().is_empty() => default,
        Some(Value::String(s)) => s
            .trim()
            .parse::<i64>()
            .ok()
            .or_else(|| {
                s.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|f| f.is_finite())
                    .map(|f| f.trunc() as i64)
            })
            .unwrap_or(default),
        other => as_i64(other, default),
    }
}

/// Python truthiness for `bool(d.get(k, False))`.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

pub fn truthy_value(v: &Value) -> bool {
    truthy(Some(v))
}

/// A finite non-bool JSON number.
pub fn finite_num(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64().filter(|f| f.is_finite()),
        _ => None,
    }
}

/// `str(v)` for scalars (used for manifest titles etc.).
pub fn py_str(v: Option<&Value>, default: &str) -> String {
    match v {
        None | Some(Value::Null) => default.to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => {
            if *b {
                "True".into()
            } else {
                "False".into()
            }
        }
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

/// Lowercase-insensitive compare key (SQLite NOCASE folds ASCII only).
pub fn nocase(s: &str) -> String {
    s.to_ascii_lowercase()
}

pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random source unavailable");
    // URL-safe base64 without padding.
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes * 4 / 3 + 2);
    for chunk in buf.chunks(3) {
        let n = match chunk.len() {
            3 => (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]),
            2 => (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8),
            _ => u32::from(chunk[0]) << 16,
        };
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_matches_python() {
        assert_eq!(round_to(2.675, 2), 2.67); // binary value is below .675
        assert_eq!(round_to(0.0005, 3), 0.001); // 0.0005 is slightly above in binary
        assert_eq!(round_to(1.2345, 3), 1.234);
        assert_eq!(round_to(12.5, 0), 12.0);
    }

    #[test]
    fn tokens_are_url_safe() {
        let t = random_token(18);
        assert_eq!(t.len(), 24);
        assert!(
            t.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }
}
