//! HTTP routes.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::services::{ServeDir, ServeFile};

use riffboard_core::chart::Chart;

use crate::chart;
use crate::config::Config;
use crate::library::{self, Index};
use crate::pack::{self, UnpackCache};
use crate::store::{self, Store};
use crate::util::{self, num, quote_path, round_to};

// ── State ────────────────────────────────────────────────────────────────────

/// (library key, mtime, size, its arrangements) — a small most-recent-first cache.
type ChartEntry = (String, f64, u64, Arc<Vec<Chart>>);

pub struct AppState {
    pub cfg: Config,
    pub index: Index,
    pub cache: UnpackCache,
    pub store: Store,
    pub admin_token: String,
    charts: Mutex<VecDeque<ChartEntry>>,
    runs: Mutex<HashMap<String, Run>>,
    start_limiter: RateLimiter,
    submit_limiter: RateLimiter,
}

struct Run {
    ip: String,
    filename: String,
    arrangement: i64,
    started: Instant,
    expires: Instant,
}

pub struct RateLimiter {
    limit: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(limit: usize, window_secs: u64) -> Self {
        Self {
            limit,
            window: Duration::from_secs(window_secs),
            hits: Mutex::default(),
        }
    }
    pub fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().unwrap();
        if hits.len() > 20_000 {
            hits.retain(|_, q| {
                q.back()
                    .is_some_and(|t| now.duration_since(*t) < self.window)
            });
        }
        let q = hits.entry(key.to_string()).or_default();
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) > self.window)
        {
            q.pop_front();
        }
        if q.len() >= self.limit {
            return false;
        }
        q.push_back(now);
        true
    }
}

// Leaderboard tunables (see README "Score checks").
// Leaderboard rules are shared with the game (crates/core/src/rules.rs).
use riffboard_core::rules::{MIN_ELAPSED_FRACTION, check_result};
const RUN_TTL_EXTRA_S: u64 = 900;
const MAX_RUNS_PER_IP: usize = 4;
const MAX_RUNS_TOTAL: usize = 5000;
const CHART_CACHE: usize = 12;

impl AppState {
    pub fn new(cfg: Config, store: Store, admin_token: String) -> Self {
        // Unpacked zip packs live here; the old name's folder is just a stale
        // copy of the same cache.
        let _ = std::fs::remove_dir_all(cfg.config_dir.join("sloppak_cache"));
        let cache = UnpackCache::new(cfg.config_dir.join("pack_cache"), cfg.cache_bytes);
        Self {
            cfg,
            index: Index::new(),
            cache,
            store,
            admin_token,
            charts: Mutex::default(),
            runs: Mutex::default(),
            start_limiter: RateLimiter::new(40, 600),
            submit_limiter: RateLimiter::new(20, 3600),
        }
    }

    pub fn rescan(&self) {
        self.index.scan(&self.cfg.dlc_dir);
    }

    /// (canonical path, library key) for a request filename, or None.
    fn resolve(&self, filename: &str) -> Option<(PathBuf, String)> {
        let p = pack::resolve_in(&self.cfg.dlc_dir, filename)?;
        if !pack::is_pack(&p) {
            return None;
        }
        let key = pack::rel_key(&self.cfg.dlc_dir, &p)?;
        Some((p, key))
    }

    /// Parse (and cache) a pack's arrangements.
    fn load(&self, key: &str, path: &FsPath) -> Option<Arc<Vec<Chart>>> {
        let (mtime, size) = pack::stamp(path)?;
        {
            let c = self.charts.lock().unwrap();
            if let Some(hit) = c.iter().find(|e| e.0 == key && e.1 == mtime && e.2 == size) {
                return Some(hit.3.clone());
            }
        }
        let dir = self.cache.source_dir(key, path)?;
        let manifest = pack::read_manifest(&dir)?;
        let base = format!("/api/pack/{}/file/", quote_path(key));
        let loaded = Arc::new(chart::load(&dir, &manifest, |rel| {
            format!("{base}{}", quote_path(rel))
        }));
        let mut c = self.charts.lock().unwrap();
        c.retain(|e| e.0 != key);
        c.push_front((key.to_string(), mtime, size, loaded.clone()));
        c.truncate(CHART_CACHE);
        Some(loaded)
    }

    fn ip(&self, headers: &HeaderMap, peer: SocketAddr) -> String {
        crate::guard::client_ip(headers, Some(peer), self.cfg.client_ip_header.as_deref())
    }
}

type S = State<Arc<AppState>>;

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"error": msg}))).into_response()
}

fn no_cache(mut r: Response) -> Response {
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    r
}

// ── Router ───────────────────────────────────────────────────────────────────

/// Largest request body any route accepts, in bytes.
pub const BODY_LIMIT: usize = 16 * 1024;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/favicon.ico", get(favicon))
        .route(
            "/robots.txt",
            get(|| async { "User-agent: *\nDisallow: /api/\nDisallow: /ws/\n" }),
        )
        .route("/static/{*path}", get(static_file))
        .route("/api/version", get(version))
        .route(
            "/api/site",
            get(|State(s): S| async move {
                no_cache(Json(crate::site::public_json(&s.cfg.config_dir)).into_response())
            }),
        )
        .route("/site.css", get(site_css))
        .route("/site/{name}", get(site_asset))
        .route("/api/library", get(library_page))
        .route("/api/song/{*rest}", get(song))
        .route("/api/chart/{*filename}", get(chart_json))
        .route("/api/pack/{*rest}", get(pack_file))
        .route("/api/runs", post(start_run))
        .route("/api/scores", post(submit_score))
        .route("/api/scores/{*filename}", get(get_board))
        .route("/api/leaderboards", get(leaderboards))
        .route("/api/admin/rescan", post(admin_rescan))
        .route("/api/admin/scores/recent", get(admin_recent))
        .route("/api/admin/scores/delete", post(admin_delete))
        .route("/api/admin/names/ban", post(admin_ban))
        .route("/api/admin/names/unban", post(admin_unban))
        .route("/api/admin/scores/wipe", post(admin_wipe))
        // Every request body is a few small JSON fields; cap them well below axum's 2 MB default.
        .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::guard::middleware,
        ))
        // JSON (charts, the library) compressed on the fly; the static files
        // ship precompressed and audio is compressed already.
        .layer(
            CompressionLayer::new()
                .gzip(true)
                .compress_when(DefaultPredicate::new().and(NotForContentType::const_new("audio/"))),
        )
        .with_state(state)
}

// ── Static / meta ────────────────────────────────────────────────────────────

/// Serves a file, preferring a precompressed `<file>.br` / `<file>.gz` sibling
/// when the client accepts it (the container image ships them for the
/// frontend; without them this is a plain file response).
async fn serve_file(path: PathBuf, req: Request) -> Response {
    match ServeFile::new(path)
        .precompressed_br()
        .precompressed_gzip()
        .oneshot(req)
        .await
    {
        Ok(r) => r.map(Body::new),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "read failed"),
    }
}

/// The game page's Content-Security-Policy: everything from this origin only;
/// `wasm-unsafe-eval` lets the page compile the game's WebAssembly, nothing
/// else. Inline style attributes are allowed (no inline scripts).
const GAME_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self' blob:; \
    connect-src 'self'; worker-src 'self'; object-src 'none'; base-uri 'none'; \
    form-action 'none'; frame-ancestors 'none'";

async fn index(State(s): S, req: Request) -> Response {
    let mut r = no_cache(serve_file(s.cfg.static_dir.join("game/index.html"), req).await);
    r.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(GAME_CSP),
    );
    r
}

async fn favicon(State(s): S, req: Request) -> Response {
    serve_file(s.cfg.static_dir.join("brand/favicon-32.png"), req).await
}

async fn static_file(State(s): S, req: Request) -> Response {
    // ServeDir resolves under the static root and refuses traversal itself.
    let uri = req.uri().clone();
    let stripped = uri
        .path()
        .strip_prefix("/static")
        .unwrap_or("/")
        .to_string();
    let mut req = req;
    *req.uri_mut() = match uri.query() {
        Some(q) => format!("{stripped}?{q}"),
        None => stripped,
    }
    .parse()
    .unwrap_or_default();
    match ServeDir::new(&s.cfg.static_dir)
        .precompressed_br()
        .precompressed_gzip()
        .oneshot(req)
        .await
    {
        Ok(r) => no_cache(r.map(Body::new)),
        Err(_) => err(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn site_css(State(s): S) -> Response {
    no_cache(
        (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            crate::site::css(&s.cfg.config_dir),
        )
            .into_response(),
    )
}

async fn site_asset(State(s): S, Path(name): Path<String>, req: Request) -> Response {
    if !crate::site::is_asset_name(&name) {
        return err(StatusCode::NOT_FOUND, "not found");
    }
    let dir = s.cfg.config_dir.join("site");
    match pack::resolve_in(&dir, &name).filter(|p| p.is_file()) {
        Some(p) => no_cache(serve_file(p, req).await),
        None => err(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn version(State(s): S) -> Json<Value> {
    Json(
        json!({"version": s.cfg.version, "source_url": s.cfg.source_url, "license_url": s.cfg.license_url}),
    )
}

// ── Library ──────────────────────────────────────────────────────────────────

async fn library_page(State(s): S) -> Response {
    let songs: Vec<Value> = s.index.all().iter().map(library::Song::row).collect();
    Json(json!({ "songs": songs })).into_response()
}

// ── Songs ────────────────────────────────────────────────────────────────────

async fn song(State(s): S, Path(rest): Path<String>, headers: HeaderMap) -> Response {
    if let Some(f) = rest
        .strip_suffix("/art")
        .filter(|f| pack::is_pack(FsPath::new(f)))
    {
        return art(&s, f, &headers);
    }
    let Some((path, key)) = s.resolve(&rest) else {
        return err(StatusCode::NOT_FOUND, "File not found");
    };
    let meta = match s.index.get(&key) {
        Some(m) => m,
        None => {
            let Some(((mtime, size), manifest)) =
                pack::stamp(&path).zip(pack::read_manifest(&path))
            else {
                return err(StatusCode::NOT_FOUND, "File not found");
            };
            library::extract(&key, &manifest, mtime, size)
        }
    };
    Json(meta.details()).into_response()
}

fn art(s: &AppState, filename: &str, headers: &HeaderMap) -> Response {
    let Some((path, _)) = s.resolve(filename) else {
        return err(StatusCode::NOT_FOUND, "not found");
    };
    let manifest = pack::read_manifest(&path);
    let Some((bytes, mt)) = pack::read_cover(&path, manifest.as_ref()) else {
        return err(StatusCode::NOT_FOUND, "no art");
    };
    let etag = format!("\"{}\"", &util::hex(&Sha256::digest(&bytes))[..32]);
    let inm = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if inm.split(',').any(|t| {
        let t = t.trim().trim_start_matches("W/");
        t == "*" || t == etag
    }) {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, "no-cache".into()),
            ],
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, mt.to_string()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, "no-cache".into()),
        ],
        bytes,
    )
        .into_response()
}

async fn pack_file(State(s): S, Path(rest): Path<String>, req: Request) -> Response {
    let split = pack::PACK_EXTS
        .iter()
        .filter_map(|ext| {
            rest.find(&format!("{ext}/file/"))
                .map(|i| (i + ext.len(), ext.len()))
        })
        .min_by_key(|x| x.0);
    let Some((cut, _)) = split else {
        return err(StatusCode::NOT_FOUND, "not found");
    };
    let (fname, rel) = (&rest[..cut], &rest[cut + "/file/".len()..]);
    let Some((path, key)) = s.resolve(fname) else {
        return err(StatusCode::NOT_FOUND, "not found");
    };
    let state = s.clone();
    let key2 = key.clone();
    let dir = tokio::task::spawn_blocking(move || state.cache.source_dir(&key2, &path))
        .await
        .ok()
        .flatten();
    let Some(dir) = dir else {
        return err(StatusCode::NOT_FOUND, "not found");
    };
    match pack::resolve_in(&dir, rel).filter(|p| p.is_file() && !p.ends_with(".complete")) {
        Some(target) => serve_file(target, req).await,
        None => err(StatusCode::NOT_FOUND, "not found"),
    }
}

// ── Charts ───────────────────────────────────────────────────────────────────

/// One arrangement as the game plays it. Without `?arrangement=` it is the
/// one with the most notes.
async fn chart_json(
    State(s): S,
    Path(filename): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some((path, key)) = s.resolve(&filename) else {
        return err(StatusCode::NOT_FOUND, "File not found");
    };
    let st = s.clone();
    let charts = tokio::task::spawn_blocking(move || st.load(&key, &path))
        .await
        .ok()
        .flatten();
    let Some(charts) = charts.filter(|c| !c.is_empty()) else {
        return err(StatusCode::NOT_FOUND, "This song has no playable parts");
    };
    let pick = match q.get("arrangement").and_then(|a| a.parse::<usize>().ok()) {
        Some(i) => charts.get(i),
        None => charts.iter().max_by_key(|c| c.judgeable()),
    };
    let Some(c) = pick else {
        return err(StatusCode::NOT_FOUND, "No such part");
    };
    match serde_json::to_string(c) {
        Ok(body) => no_cache(([(header::CONTENT_TYPE, "application/json")], body).into_response()),
        Err(_) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not send the chart",
        ),
    }
}

// ── Leaderboard ──────────────────────────────────────────────────────────────

/// The server-side checks on a finished run. Returns (score, accuracy).
fn count(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite() && f.fract() == 0.0)
                .map(|f| f as i64)
        }),
        _ => None,
    }
    .filter(|x| (0..=1_000_000).contains(x))
}

/// (judgeable notes, last note time) of one arrangement; None if it has no notes.
fn facts(s: &AppState, key: &str, path: &FsPath, arrangement: i64) -> Option<(usize, f64)> {
    let charts = s.load(key, path)?;
    let c = charts.get(usize::try_from(arrangement).ok()?)?;
    (c.judgeable() > 0).then(|| (c.judgeable(), c.last_event_time()))
}

async fn start_run(
    State(s): S,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let ip = s.ip(&headers, peer);
    if !s.start_limiter.allow(&ip) {
        return err(StatusCode::TOO_MANY_REQUESTS, "slow down");
    }
    let data: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let arrangement = if data.get("arrangement").is_none() {
        Some(0)
    } else {
        count(data.get("arrangement"))
    };
    let resolved = data
        .get("filename")
        .and_then(Value::as_str)
        .and_then(|f| s.resolve(f));
    let (Some((path, key)), Some(arrangement)) = (resolved, arrangement) else {
        return err(StatusCode::BAD_REQUEST, "unknown song");
    };
    let st = s.clone();
    let k = key.clone();
    let f = tokio::task::spawn_blocking(move || facts(&st, &k, &path, arrangement))
        .await
        .ok()
        .flatten();
    let Some((_, last)) = f else {
        return err(StatusCode::BAD_REQUEST, "this arrangement can't be scored");
    };
    let now = Instant::now();
    let rid = util::random_token(18);
    {
        let mut runs = s.runs.lock().unwrap();
        runs.retain(|_, r| r.expires > now);
        let mut mine: Vec<(Instant, String)> = runs
            .iter()
            .filter(|(_, r)| r.ip == ip)
            .map(|(k, r)| (r.started, k.clone()))
            .collect();
        mine.sort();
        while mine.len() >= MAX_RUNS_PER_IP {
            runs.remove(&mine.remove(0).1);
        }
        if runs.len() >= MAX_RUNS_TOTAL {
            return err(StatusCode::SERVICE_UNAVAILABLE, "busy, try again shortly");
        }
        runs.insert(
            rid.clone(),
            Run {
                ip,
                filename: key,
                arrangement,
                started: now,
                expires: now + Duration::from_secs_f64(last) + Duration::from_secs(RUN_TTL_EXTRA_S),
            },
        );
    }
    Json(json!({"run_id": rid, "min_seconds": num(round_to(last * MIN_ELAPSED_FRACTION, 1))}))
        .into_response()
}

async fn submit_score(
    State(s): S,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let ip = s.ip(&headers, peer);
    if !s.submit_limiter.allow(&ip) {
        return err(StatusCode::TOO_MANY_REQUESTS, "slow down");
    }
    let data: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let Some(rid) = data.get("run_id").and_then(Value::as_str) else {
        return err(StatusCode::BAD_REQUEST, "missing run");
    };
    let run = {
        let mut runs = s.runs.lock().unwrap();
        let now = Instant::now();
        runs.retain(|_, r| r.expires > now);
        runs.remove(rid)
    };
    let Some(run) = run.filter(|r| r.ip == ip) else {
        return err(
            StatusCode::CONFLICT,
            "this run has expired - play it again to post a score",
        );
    };
    let Some(name) = data
        .get("name")
        .and_then(Value::as_str)
        .and_then(store::clean_name)
    else {
        return err(
            StatusCode::BAD_REQUEST,
            &format!(
                "names are 1-{} letters, numbers, spaces or . ' _ -",
                store::NAME_MAX
            ),
        );
    };
    if s.store.is_banned(&name) {
        return err(StatusCode::FORBIDDEN, "that name can't be used");
    }
    let (hits, misses) = (count(data.get("hits")), count(data.get("misses")));
    let streak = if data.get("best_streak").is_none() {
        Some(0)
    } else {
        count(data.get("best_streak"))
    };
    let (Some(hits), Some(misses), Some(streak)) = (hits, misses, streak) else {
        return err(StatusCode::BAD_REQUEST, "invalid result");
    };
    if streak > hits {
        return err(StatusCode::BAD_REQUEST, "invalid result");
    }
    let Some((path, key)) = s.resolve(&run.filename) else {
        return err(StatusCode::BAD_REQUEST, "unknown song");
    };
    let st = s.clone();
    let k = key.clone();
    let arr = run.arrangement;
    let f = tokio::task::spawn_blocking(move || facts(&st, &k, &path, arr))
        .await
        .ok()
        .flatten();
    let Some((judgeable, last)) = f else {
        return err(StatusCode::BAD_REQUEST, "this arrangement can't be scored");
    };
    let (score, accuracy) = match check_result(
        run.started.elapsed().as_secs_f64(),
        judgeable,
        last,
        hits,
        misses,
    ) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::UNPROCESSABLE_ENTITY, e),
    };
    let Ok(id) = s
        .store
        .add(&key, arr, &name, score, accuracy, hits, misses, streak, &ip)
    else {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not save the score",
        );
    };
    tracing::info!("score {name} on {key}[{arr}]: {score}");
    Json(json!({
        "id": id, "score": score, "accuracy": round_to(accuracy, 4),
        "rank": s.store.rank_of(&key, arr, id), "board": s.store.top(&key, arr, 10),
    }))
    .into_response()
}

async fn get_board(
    State(s): S,
    Path(filename): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some((_, key)) = s.resolve(&filename) else {
        return err(StatusCode::NOT_FOUND, "unknown song");
    };
    let arr = q
        .get("arrangement")
        .and_then(|a| a.parse().ok())
        .unwrap_or(0);
    let limit = q.get("limit").and_then(|a| a.parse().ok()).unwrap_or(10);
    Json(json!({"filename": key, "arrangement": arr, "scores": s.store.top(&key, arr, limit)}))
        .into_response()
}

async fn leaderboards(State(s): S) -> Json<Value> {
    let songs = s.index.all();
    let mut boards: Vec<Value> = s.store.summary().into_iter().filter_map(|(fn_, arr, runs, top, top_name)| {
        let song = songs.iter().find(|x| x.filename == fn_)?;
        let arr_name = usize::try_from(arr).ok().and_then(|i| song.arrangements.get(i)).cloned().unwrap_or_default();
        Some(json!({
            "filename": fn_, "arrangement": arr, "runs": runs, "top_score": top, "top_name": top_name,
            "title": song.title, "artist": song.artist, "arrangement_name": arr_name,
            "art_url": format!("/api/song/{}/art", quote_path(&fn_)),
        }))
    }).collect();
    boards.sort_by(|a, b| {
        let k = |v: &Value| {
            (
                v["artist"].as_str().unwrap_or("").to_lowercase(),
                v["title"].as_str().unwrap_or("").to_lowercase(),
                v["arrangement"].as_i64(),
            )
        };
        k(a).cmp(&k(b))
    });
    Json(json!({"boards": boards}))
}

// ── Operator (loopback + token, enforced in guard) ──────────────────────────

async fn admin_rescan(State(s): S) -> Json<Value> {
    let st = s.clone();
    tokio::task::spawn_blocking(move || st.rescan());
    Json(json!({"started": true}))
}

async fn admin_recent(State(s): S, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    let limit = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    Json(json!({"scores": s.store.recent(limit)}))
}

async fn admin_delete(State(s): S, Json(d): Json<Value>) -> Response {
    match count(d.get("id")) {
        Some(id) => Json(json!({"deleted": s.store.delete(id)})).into_response(),
        None => err(StatusCode::BAD_REQUEST, "id required"),
    }
}

async fn admin_ban(State(s): S, Json(d): Json<Value>) -> Response {
    match d
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.trim().is_empty())
    {
        Some(n) => {
            let purge = d.get("purge").is_none_or(util::truthy_value);
            Json(json!({"removed_scores": s.store.ban(n, purge)})).into_response()
        }
        None => err(StatusCode::BAD_REQUEST, "name required"),
    }
}

async fn admin_unban(State(s): S, Json(d): Json<Value>) -> Response {
    match d
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.trim().is_empty())
    {
        Some(n) => Json(json!({"unbanned": s.store.unban(n)})).into_response(),
        None => err(StatusCode::BAD_REQUEST, "name required"),
    }
}

async fn admin_wipe(State(s): S, Json(d): Json<Value>) -> Response {
    match d
        .get("filename")
        .and_then(Value::as_str)
        .filter(|f| !f.is_empty())
    {
        Some(f) => {
            Json(json!({"removed": s.store.wipe(f, count(d.get("arrangement")))})).into_response()
        }
        None => err(StatusCode::BAD_REQUEST, "filename required"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_checks() {
        // 25 judgeable notes, last at 30 s.
        assert_eq!(check_result(30.0, 25, 30.0, 20, 5).unwrap().0, 1600);
        assert!(check_result(5.0, 25, 30.0, 20, 5).is_err(), "too fast");
        assert!(check_result(26.0, 25, 30.0, 20, 5).is_ok(), "within grace");
        assert!(
            check_result(30.0, 25, 30.0, 100, 0).is_err(),
            "more than the chart"
        );
        assert!(check_result(30.0, 25, 30.0, 5, 2).is_err(), "partial run");
        assert_eq!(
            check_result(30.0, 8, 30.0, 3, 5).unwrap().0,
            113,
            "rounds half up like JS"
        );
    }

    #[test]
    fn rate_limiter() {
        let r = RateLimiter::new(2, 60);
        assert!(r.allow("a") && r.allow("a"));
        assert!(!r.allow("a"));
        assert!(r.allow("b"));
    }
}
