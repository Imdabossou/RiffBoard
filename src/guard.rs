//! Public-site request guard: an ALLOWLIST of what visitors may reach.
//!
//! A request is served only when its method and path match an entry in
//! `is_allowed`; everything else gets a plain 404 before any handler runs.
//! The guard also sets the security headers and owns the "who is the client"
//! question used for rate limiting. Keep the list narrow: never add a rule
//! that matches `/api/*` as a whole.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

use crate::web::AppState;

const PACK_SUFFIXES: [&str; 2] = [".feedpak", ".sloppak"];

fn has_dotdot(path: &str) -> bool {
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    path.split('/').any(|seg| seg == "..")
        || decoded.split(['/', '\\']).any(|seg| seg == "..")
        || decoded.contains('\0')
}

pub fn is_allowed(method: &Method, path: &str) -> bool {
    if has_dotdot(path) || path.contains('\0') {
        return false;
    }
    if method == Method::POST {
        return path == "/api/runs" || path == "/api/scores";
    }
    if method != Method::GET && method != Method::HEAD {
        return false;
    }
    match path {
        "/" | "/favicon.ico" | "/robots.txt" | "/api/version" | "/api/library"
        | "/api/leaderboards" | "/api/site" | "/site.css" => return true,
        _ => {}
    }
    if let Some(rest) = path.strip_prefix("/site/") {
        return crate::site::is_asset_name(rest);
    }
    let nonempty = |prefix: &str| path.strip_prefix(prefix).is_some_and(|r| !r.is_empty());
    if nonempty("/static/")
        || nonempty("/api/chart/")
        || nonempty("/api/song/")
        || nonempty("/api/scores/")
    {
        return true;
    }
    if let Some(rest) = path.strip_prefix("/api/pack/") {
        return PACK_SUFFIXES
            .iter()
            .any(|s| rest.contains(&format!("{s}/file/")));
    }
    false
}

/// The visitor's IP: from the operator-named proxy header when configured
/// (e.g. CF-Connecting-IP), else the socket peer.
pub fn client_ip(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    header_name: Option<&str>,
) -> String {
    if let Some(h) = header_name
        && let Some(v) = headers.get(h).and_then(|v| v.to_str().ok())
        && let Ok(ip) = v.split(',').next().unwrap_or("").trim().parse::<IpAddr>()
    {
        return ip.to_string();
    }
    peer.map_or_else(|| "unknown".into(), |p| p.ip().to_string())
}

/// Admin requests: loopback peer, no proxy forwarding headers, right token.
pub fn is_admin(headers: &HeaderMap, peer: Option<SocketAddr>, token: &str) -> bool {
    if !peer.is_some_and(|p| p.ip().is_loopback()) {
        return false;
    }
    for h in [
        "x-forwarded-for",
        "cf-connecting-ip",
        "forwarded",
        "x-real-ip",
    ] {
        if headers.contains_key(h) {
            return false;
        }
    }
    let given = headers
        .get("x-riffboard-admin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    !token.is_empty()
        && given.len() == token.len()
        && bool::from(given.as_bytes().ct_eq(token.as_bytes()))
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"not found"}"#,
    )
        .into_response()
}

pub async fn middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    let path = req.uri().path().to_string();
    let ok = if path.starts_with("/api/admin/") {
        is_admin(req.headers(), peer, &state.admin_token)
    } else {
        is_allowed(req.method(), &path)
    };
    let mut resp = if ok { next.run(req).await } else { not_found() };
    let h = resp.headers_mut();
    for (k, v) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "strict-origin-when-cross-origin"),
        ("x-frame-options", "DENY"),
        (
            "permissions-policy",
            "microphone=(self), camera=(), geolocation=(), payment=(), usb=(), midi=(self)",
        ),
        ("cross-origin-opener-policy", "same-origin"),
    ] {
        h.entry(k).or_insert(HeaderValue::from_static(v));
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        let g = Method::GET;
        let p = Method::POST;
        for (m, path) in [
            (&g, "/"),
            (&g, "/static/game/app.js"),
            (&g, "/static/game/pkg/riffboard_game_bg.wasm"),
            (&g, "/api/library"),
            (&g, "/api/song/A/b.feedpak"),
            (&g, "/api/song/a/b.feedpak/art"),
            (&g, "/api/pack/a.feedpak/file/stems/full.ogg"),
            (&g, "/api/scores/a.feedpak"),
            (&g, "/api/leaderboards"),
            (&p, "/api/runs"),
            (&p, "/api/scores"),
            (&g, "/api/chart/a/b.feedpak"),
            (&g, "/api/site"),
            (&g, "/site.css"),
            (&g, "/site/logo.png"),
        ] {
            assert!(is_allowed(m, path), "{m} {path}");
        }
        for (m, path) in [
            (&p, "/api/settings"),
            (&p, "/api/rescan"),
            (&p, "/api/songs/upload"),
            (&Method::DELETE, "/api/song/a"),
            (&p, "/api/plugins/x/update"),
            (&g, "/api/settings/export"),
            (&g, "/api/diagnostics/preview"),
            (&g, "/docs"),
            (&Method::PUT, "/api/scores"),
            (&g, "/api/admin/scores/recent"),
            (&g, "/api/song/../../etc/passwd"),
            (&g, "/static/../server.py"),
            (&g, "/api/plugins/x/routes.py"),
            (&g, "/api/plugins/tuner/screen.js"),
            (&g, "/api/plugins"),
            (&g, "/api/settings"),
            (&g, "/api/tunings"),
            (&g, "/api/library/artists"),
            (&g, "/api/library/providers/remote:x/songs/a/art"),
            (&g, "/api/work/a%7Cb/charts"),
            (&g, "/api/startup-status"),
            (&g, "/api/scan-status"),
            (&g, "/static/"),
            (&g, "/api/chart/"),
            (&g, "/api/pack/a.zip/file/x"),
            (&p, "/api/chart/a.feedpak"),
            // The old frontend's routes are gone.
            (&g, "/ws/highway/a/b.feedpak"),
            (&g, "/api/sloppak/a.feedpak/file/stems/full.ogg"),
            (&g, "/api/song/%2E%2E%2Fetc%2Fpasswd"),
            (&g, "/site/site.json"),
            (&g, "/site/logo.svg"),
            (&g, "/site/../config.json"),
            (&p, "/site.css"),
            (&g, "/static/%2e%2e/server.py"),
        ] {
            assert!(!is_allowed(m, path), "{m} {path}");
        }
    }

    #[test]
    fn admin_needs_loopback_token_and_no_proxy_headers() {
        let lo: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let far: SocketAddr = "192.0.2.1:5000".parse().unwrap();
        let mut h = HeaderMap::new();
        h.insert("x-riffboard-admin", HeaderValue::from_static("tok"));
        assert!(is_admin(&h, Some(lo), "tok"));
        assert!(!is_admin(&h, Some(lo), "other"));
        assert!(!is_admin(&h, Some(far), "tok"));
        h.insert("cf-connecting-ip", HeaderValue::from_static("1.2.3.4"));
        assert!(!is_admin(&h, Some(lo), "tok"));
    }

    #[test]
    fn client_ip_header_is_opt_in() {
        let peer: SocketAddr = "10.0.0.2:1".parse().unwrap();
        let mut h = HeaderMap::new();
        h.insert("cf-connecting-ip", HeaderValue::from_static("198.51.100.7"));
        assert_eq!(client_ip(&h, Some(peer), None), "10.0.0.2");
        assert_eq!(
            client_ip(&h, Some(peer), Some("cf-connecting-ip")),
            "198.51.100.7"
        );
        h.insert("cf-connecting-ip", HeaderValue::from_static("junk"));
        assert_eq!(
            client_ip(&h, Some(peer), Some("cf-connecting-ip")),
            "10.0.0.2"
        );
    }
}
