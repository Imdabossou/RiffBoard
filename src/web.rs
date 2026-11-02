//! HTTP + WebSocket routes.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

use crate::chart::{self, Loaded};
use crate::config::Config;
use crate::library::{self, Filters, Index};
use crate::pack::{self, UnpackCache};
use crate::store::{self, Store};
use crate::tunings;
use crate::util::{self, num, round_to};
use crate::util_url::quote_component;

// ── State ────────────────────────────────────────────────────────────────────

/// (library key, mtime, size, parsed charts) — a small most-recent-first cache.
type ChartEntry = (String, f64, u64, Arc<Loaded>);

pub struct AppState {
    pub cfg: Config,
    pub index: Index,
    pub cache: UnpackCache,
    pub store: Store,
    pub admin_token: String,
    pub plugins: Value,
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
const MIN_ELAPSED_FRACTION: f64 = 0.95;
const ELAPSED_GRACE_S: f64 = 3.0;
const MAX_JUDGED_SLACK: f64 = 1.05;
const MIN_JUDGED_FRACTION: f64 = 0.5;
const RUN_TTL_EXTRA_S: u64 = 900;
const MAX_RUNS_PER_IP: usize = 4;
const MAX_RUNS_TOTAL: usize = 5000;
const CHART_CACHE: usize = 12;

impl AppState {
    pub fn new(cfg: Config, store: Store, admin_token: String, plugins: Value) -> Self {
        let cache = UnpackCache::new(cfg.config_dir.join("sloppak_cache"), cfg.cache_bytes);
        Self {
            cfg,
            index: Index::new(),
            cache,
            store,
            admin_token,
            plugins,
            charts: Mutex::default(),
            runs: Mutex::default(),
            start_limiter: RateLimiter::new(40, 600),
            submit_limiter: RateLimiter::new(20, 3600),
        }
    }

    pub fn rescan(&self) {
        self.index.scan(&self.cfg.dlc_dir);
    }

    fn operator_config(&self) -> Value {
        std::fs::read_to_string(self.cfg.config_dir.join("config.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
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

    /// Parse (and cache) a pack's charts.
    fn load(&self, key: &str, path: &FsPath) -> Option<Arc<Loaded>> {
        let (mtime, size) = pack::stamp(path)?;
        {
            let c = self.charts.lock().unwrap();
            if let Some(hit) = c.iter().find(|e| e.0 == key && e.1 == mtime && e.2 == size) {
                return Some(hit.3.clone());
            }
        }
        let dir = self.cache.source_dir(key, path)?;
        let manifest = pack::read_manifest(&dir)?;
        let loaded = Arc::new(chart::load(&dir, manifest));
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
            "/api/scan-status",
            get(|State(s): S| async move { Json(s.index.status()) }),
        )
        .route(
            "/api/startup-status",
            get(|| async { Json(startup_status()) }),
        )
        .route("/api/startup-status/stream", get(startup_stream))
        .route("/api/settings", get(settings))
        .route("/api/tunings", get(tunings_route))
        .route("/api/library", get(library_page))
        .route("/api/library/stats", get(library_stats))
        .route("/api/library/albums", get(library_albums))
        .route("/api/library/artists", get(library_artists))
        .route(
            "/api/library/genres",
            get(|State(s): S| async move { Json(library::genres(&s.index.all())) }),
        )
        .route("/api/library/tuning-names", get(tuning_names))
        .route(
            "/api/library/providers",
            get(|| async { Json(library::providers()) }),
        )
        .route(
            "/api/library/providers/{id}/songs/{*rest}",
            get(provider_art),
        )
        .route("/api/work/{*rest}", get(work_charts))
        .route("/api/song/{*rest}", get(song))
        .route("/api/sloppak/{*rest}", get(sloppak_file))
        .route(
            "/api/plugins",
            get(|State(s): S| async move { Json(s.plugins.clone()) }),
        )
        .route("/api/plugins/{id}/{*rest}", get(plugin_file))
        .route("/ws/highway/{*filename}", get(highway_ws))
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
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::guard::middleware,
        ))
        .with_state(state)
}

// ── Static / meta ────────────────────────────────────────────────────────────

async fn serve_file(path: PathBuf, req: Request) -> Response {
    match ServeFile::new(path).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "read failed"),
    }
}

async fn index(State(s): S, req: Request) -> Response {
    no_cache(serve_file(s.cfg.static_dir.join("v3/index.html"), req).await)
}

async fn favicon(State(s): S, req: Request) -> Response {
    serve_file(s.cfg.static_dir.join("v3/brand/favicon-32.png"), req).await
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
    match ServeDir::new(&s.cfg.static_dir).oneshot(req).await {
        Ok(r) => no_cache(r.map(Body::new)),
        Err(_) => err(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn version(State(s): S) -> Json<Value> {
    Json(
        json!({"version": s.cfg.version, "source_url": s.cfg.source_url, "license_url": s.cfg.license_url}),
    )
}

fn startup_status() -> Value {
    json!({"running": false, "phase": "complete", "message": "Startup complete",
           "current_plugin": "", "loaded": 0, "total": 0, "error": null})
}

async fn startup_stream() -> Response {
    let body = format!("data: {}\n\n", startup_status());
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

async fn settings(State(s): S, Query(q): Query<HashMap<String, String>>) -> Response {
    match crate::settings::compute(&s.operator_config(), q.get("state"), q.get("patch")) {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, &e),
    }
}

async fn tunings_route(State(s): S) -> Json<Value> {
    let cfg = s.operator_config();
    let rp = cfg
        .get("reference_pitch")
        .and_then(Value::as_f64)
        .filter(|r| (430.0..=450.0).contains(r))
        .unwrap_or(tunings::DEFAULT_REFERENCE_PITCH);
    Json(tunings::tunings_payload(rp))
}

// ── Library ──────────────────────────────────────────────────────────────────

fn qnum(q: &HashMap<String, String>, k: &str, default: usize) -> usize {
    q.get(k)
        .and_then(|v| v.parse::<i64>().ok())
        .map_or(default, |v| v.max(0) as usize)
}

fn provider_ok(q: &HashMap<String, String>) -> Result<(), Box<Response>> {
    match q.get("provider").map(String::as_str) {
        None | Some("local") => Ok(()),
        Some(p) => Err(Box::new(
            (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": format!("Unknown library provider: {p}")})),
            )
                .into_response(),
        )),
    }
}

async fn library_page(State(s): S, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = provider_ok(&q) {
        return *r;
    }
    let f = Filters::from_query(&q);
    let size = qnum(&q, "size", 24).min(100);
    let group = q.get("group").is_some_and(|g| g != "0" && !g.is_empty());
    let sort = q.get("sort").map_or("artist", String::as_str);
    let dir = q.get("dir").map_or("asc", String::as_str);
    Json(library::query_page(
        &s.index.all(),
        &f,
        sort,
        dir,
        qnum(&q, "page", 0),
        size,
        group,
    ))
    .into_response()
}

async fn library_stats(State(s): S, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = provider_ok(&q) {
        return *r;
    }
    let f = Filters::from_query(&q);
    let want = q.get("sort_letters").is_some_and(|v| v != "0");
    let group = q.get("group").is_some_and(|g| g != "0" && !g.is_empty());
    let sort = q.get("sort").map_or("artist", String::as_str);
    Json(library::query_stats(&s.index.all(), &f, sort, want, group)).into_response()
}

async fn library_albums(State(s): S, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = provider_ok(&q) {
        return *r;
    }
    let f = Filters::from_query(&q);
    Json(library::query_albums(
        &s.index.all(),
        &f,
        qnum(&q, "page", 0),
        qnum(&q, "size", 120).min(500),
    ))
    .into_response()
}

async fn library_artists(State(s): S, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = provider_ok(&q) {
        return *r;
    }
    let f = Filters::from_query(&q);
    let letter = q.get("letter").cloned().unwrap_or_default();
    Json(library::query_artists(
        &s.index.all(),
        &f,
        &letter,
        qnum(&q, "page", 0),
        qnum(&q, "size", 50).min(200),
    ))
    .into_response()
}

async fn tuning_names(State(s): S, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = provider_ok(&q) {
        return *r;
    }
    let p = tunings::perspective(q.get("instrument").map_or("guitar-lead", String::as_str));
    Json(library::tuning_names(&s.index.all(), &p)).into_response()
}

async fn work_charts(State(s): S, Path(rest): Path<String>) -> Response {
    match rest.strip_suffix("/charts") {
        Some(key) => Json(library::work_charts(&s.index.all(), key)).into_response(),
        None => err(StatusCode::NOT_FOUND, "not found"),
    }
}

// ── Songs ────────────────────────────────────────────────────────────────────

async fn provider_art(
    State(s): S,
    Path((id, rest)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if id != "local" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("Unknown library provider: {id}")})),
        )
            .into_response();
    }
    match rest.strip_suffix("/art") {
        Some(f) => art(&s, f, &headers),
        None => err(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn song(
    State(s): S,
    Path(rest): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
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
            let Some((mtime, size)) = pack::stamp(&path) else {
                return err(StatusCode::NOT_FOUND, "File not found");
            };
            let Some(manifest) = pack::read_manifest(&path) else {
                return err(StatusCode::NOT_FOUND, "File not found");
            };
            library::extract(&key, &manifest, mtime, size)
        }
    };
    let mut out = library::song_info(&meta);
    if q.get("stems").is_some_and(|v| v != "0") {
        let (stems, full) = match s.load(&key, &path) {
            Some(l) => {
                let url = |rel: &str| {
                    format!(
                        "/api/sloppak/{}/file/{}",
                        quote_component(&key),
                        crate::util_url::quote_path(rel)
                    )
                };
                let stems: Vec<Value> = l
                    .stems
                    .iter()
                    .map(|st| {
                        let mut o = serde_json::Map::new();
                        o.insert("id".into(), json!(st.id));
                        o.insert("url".into(), json!(url(&st.file)));
                        o.insert("default".into(), json!(st.default));
                        o.extend(st.extra.clone());
                        Value::Object(o)
                    })
                    .collect();
                (stems, l.full_mix.as_deref().map(url))
            }
            None => (vec![], None),
        };
        out.insert("stems".into(), json!(stems));
        out.insert("full_mix_url".into(), json!(full));
    }
    Json(Value::Object(out)).into_response()
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

async fn sloppak_file(State(s): S, Path(rest): Path<String>, req: Request) -> Response {
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

// ── Plugins ──────────────────────────────────────────────────────────────────

async fn plugin_file(
    State(s): S,
    Path((id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    let known = s
        .plugins
        .as_array()
        .is_some_and(|ps| ps.iter().any(|p| p["id"] == id.as_str()));
    let Some(root) = known
        .then(|| pack::resolve_in(&s.cfg.plugins_dir, &id))
        .flatten()
    else {
        return err(StatusCode::NOT_FOUND, "not found");
    };
    let rel = match rest.as_str() {
        "screen.js" | "screen.html" | "settings.html" => rest.clone(),
        r if r.starts_with("assets/") || r.starts_with("src/") => r.to_string(),
        r if id == "tuner"
            && ["utils/", "workers/", "visualization/", "viz-assets/"]
                .iter()
                .any(|d| r.starts_with(d)) =>
        {
            r.to_string()
        }
        _ => return err(StatusCode::NOT_FOUND, "not found"),
    };
    match pack::resolve_in(&root, &rel).filter(|p| p.is_file()) {
        Some(p) => no_cache(serve_file(p, req).await),
        None => err(StatusCode::NOT_FOUND, "not found"),
    }
}

// ── Highway WebSocket ────────────────────────────────────────────────────────

async fn highway_ws(
    State(s): S,
    Path(filename): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    let arrangement = q
        .get("arrangement")
        .and_then(|a| a.parse::<i64>().ok())
        .unwrap_or(-1);
    let naming = q
        .get("naming_mode")
        .cloned()
        .unwrap_or_else(|| "legacy".into());
    ws.max_message_size(64 * 1024)
        .on_upgrade(move |sock| stream_chart(s, sock, filename, arrangement, naming))
}

async fn send(sock: &mut WebSocket, v: &Value) -> bool {
    sock.send(Message::Text(v.to_string().into())).await.is_ok()
}

async fn stream_chart(
    s: Arc<AppState>,
    mut sock: WebSocket,
    filename: String,
    arrangement: i64,
    naming: String,
) {
    let Some((path, key)) = s.resolve(&filename) else {
        let msg = if pack::resolve_in(&s.cfg.dlc_dir, &filename).is_some() {
            "forbidden"
        } else {
            "File not found"
        };
        let _ = send(&mut sock, &json!({"error": msg})).await;
        let _ = sock.send(Message::Close(None)).await;
        return;
    };
    if !send(
        &mut sock,
        &json!({"type": "loading", "stage": "Extracting..."}),
    )
    .await
    {
        return;
    }
    let st = s.clone();
    let k = key.clone();
    let mut task = tokio::task::spawn_blocking(move || st.load(&k, &path));
    let loaded = loop {
        tokio::select! {
            res = &mut task => break res.ok().flatten(),
            () = tokio::time::sleep(Duration::from_secs(3)) => {
                if !send(&mut sock, &json!({"type": "loading", "stage": "Loading..."})).await { return; }
            }
        }
    };
    let Some(song) = loaded else {
        let _ = send(&mut sock, &json!({"error": "Could not read this song"})).await;
        let _ = sock.send(Message::Close(None)).await;
        return;
    };
    if song.arrangements.is_empty() {
        let _ = send(&mut sock, &json!({"error": "No arrangements found"})).await;
        let _ = sock.send(Message::Close(None)).await;
        return;
    }
    let sel = chart::select(&song, arrangement, &naming, &s.operator_config());
    for m in chart::highway_messages(&song, &sel, &key, &naming) {
        if !send(&mut sock, &m).await {
            return;
        }
    }
    // Stay open (the client keeps the socket for the session); ignore input.
    while let Some(Ok(msg)) = sock.recv().await {
        if matches!(msg, Message::Close(_)) {
            break;
        }
    }
}

// ── Leaderboard ──────────────────────────────────────────────────────────────

/// The server-side checks on a finished run. Returns (score, accuracy).
fn check_result(
    elapsed: f64,
    judgeable: usize,
    last_note: f64,
    hits: i64,
    misses: i64,
) -> Result<(i64, f64), &'static str> {
    if elapsed + ELAPSED_GRACE_S < last_note * MIN_ELAPSED_FRACTION {
        return Err("that run finished too quickly to count");
    }
    let judged = (hits + misses) as f64;
    if judged > judgeable as f64 * MAX_JUDGED_SLACK + 2.0 {
        return Err("more notes judged than the chart has");
    }
    if judged < judgeable as f64 * MIN_JUDGED_FRACTION {
        return Err("only full runs count for the leaderboard");
    }
    let accuracy = hits as f64 / judged.max(1.0);
    // Same formula as the client: round(hits * 100 * accuracy), half up.
    Ok((
        (hits as f64 * 100.0 * accuracy + 0.5).floor() as i64,
        accuracy,
    ))
}

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

/// (judgeable notes, last note time) of one arrangement.
fn facts(s: &AppState, key: &str, path: &FsPath, arrangement: i64) -> Option<(usize, f64)> {
    let song = s.load(key, path)?;
    let a = song.arrangements.get(usize::try_from(arrangement).ok()?)?;
    Some((chart::judgeable(a), chart::last_event_time(a)?))
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
        let arr_name = song.arrangements.iter().find(|a| a["index"] == arr)
            .and_then(|a| a["name"].as_str()).unwrap_or("").to_string();
        Some(json!({
            "filename": fn_, "arrangement": arr, "runs": runs, "top_score": top, "top_name": top_name,
            "title": song.title, "artist": song.artist, "arrangement_name": arr_name,
            "art_url": format!("/api/song/{}/art", crate::util_url::quote_path(&fn_)),
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
