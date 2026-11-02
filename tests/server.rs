//! End-to-end tests against the real binary on the fixture library.
//!
//! * golden: every request in tests/golden/golden.json (recorded from the
//!   original Python server, see tests/golden/capture.py) must produce the
//!   same JSON from the Rust server.
//! * public-site guarantees: the allowlist, the leaderboard checks and the
//!   operator routes.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

struct Server {
    child: Child,
    base: String,
    config: tempdir::Dir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

mod tempdir {
    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "riffboard-test-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

async fn start(tag: &str) -> Server {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = tempdir::Dir::new(tag);
    let child = Command::new(env!("CARGO_BIN_EXE_riffboard"))
        .env("DLC_DIR", root().join("tests/fixtures/library"))
        .env("CONFIG_DIR", &config.0)
        .env("STATIC_DIR", root().join("static"))
        .env("PLUGINS_DIR", root().join("plugins"))
        .env("HOST", "127.0.0.1")
        .env("PORT", port.to_string())
        .env("LOG_LEVEL", "warn")
        .stdout(Stdio::null())
        .spawn()
        .expect("start server");
    let base = format!("http://127.0.0.1:{port}");
    for _ in 0..100 {
        if reqwest::get(format!("{base}/api/version")).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Server {
        child,
        base,
        config,
    }
}

/// Numbers compare as f64 (Python wrote `40.0`, Rust may write `40`).
fn norm(v: &Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().map(|f| (f * 1e9).round() / 1e9)),
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| !["mtime", "next_cursor", "_closed"].contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), norm(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

async fn ws_messages(base: &str, path: &str) -> Vec<Value> {
    let url = format!("{}{path}", base.replace("http", "ws"));
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect");
    let mut out = Vec::new();
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
        let Message::Text(t) = msg else { continue };
        let v: Value = serde_json::from_str(&t).unwrap();
        let done = v.get("type") == Some(&json!("ready")) || v.get("error").is_some();
        out.push(v);
        if done {
            break;
        }
    }
    let _ = ws.close(None).await;
    out
}

#[tokio::test]
async fn matches_the_python_server() {
    let srv = start("golden").await;
    let golden: serde_json::Map<String, Value> = serde_json::from_str(
        &std::fs::read_to_string(root().join("tests/golden/golden.json")).unwrap(),
    )
    .unwrap();
    // Not comparable: the capture skipped plugin loading / runs at another
    // version / caught the scanner mid-flight.
    let skip = [
        "GET /api/plugins",
        "GET /api/version",
        "GET /api/scan-status",
    ];
    let mut failures = Vec::new();
    for (key, expected) in &golden {
        if skip.contains(&key.as_str()) {
            continue;
        }
        let (kind, path) = key.split_once(' ').unwrap();
        let got = if kind == "GET" {
            let r = reqwest::get(format!("{}{path}", srv.base)).await.unwrap();
            let status = r.status().as_u16();
            let body: Value = r.json().await.unwrap_or(Value::Null);
            json!({"status": status, "body": body})
        } else {
            json!({"messages": ws_messages(&srv.base, path).await})
        };
        if norm(&got) != norm(expected) {
            failures.push(key.clone());
        }
    }
    assert!(
        failures.is_empty(),
        "responses differ from the reference for: {failures:#?}"
    );
}

async fn post(base: &str, path: &str, body: Value) -> (u16, Value) {
    let r = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let s = r.status().as_u16();
    (s, r.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn only_listed_routes_are_reachable() {
    let srv = start("guard").await;
    let c = reqwest::Client::new();
    for (method, path) in [
        ("POST", "/api/settings"),
        ("POST", "/api/rescan"),
        ("POST", "/api/songs/upload"),
        ("DELETE", "/api/song/x.feedpak"),
        ("GET", "/docs"),
        ("GET", "/api/settings/export"),
        ("GET", "/api/admin/scores/recent"),
        ("GET", "/static/%2e%2e/Cargo.toml"),
        ("GET", "/api/song/..%2F..%2Fetc%2Fpasswd"),
        ("GET", "/api/plugins/tuner/routes.py"),
    ] {
        let r = c
            .request(method.parse().unwrap(), format!("{}{path}", srv.base))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 404, "{method} {path}");
    }
    let r = c
        .get(format!("{}/api/version", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    assert_eq!(r.headers()["x-frame-options"], "DENY");
    assert!(
        r.headers()["permissions-policy"]
            .to_str()
            .unwrap()
            .contains("microphone=(self)")
    );
    // Static files and plugins are served.
    for path in [
        "/",
        "/static/v3/index.html",
        "/api/plugins/tuner/screen.js",
        "/api/plugins/note_detect/settings.html",
    ] {
        assert_eq!(
            c.get(format!("{}{path}", srv.base))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            200,
            "{path}"
        );
    }
    // Audio supports range requests (seeking).
    let r = c
        .get(format!(
            "{}/api/sloppak/Fixture%20Band%2FRich%20Song.feedpak/file/stems/full.ogg",
            srv.base
        ))
        .header("Range", "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 206);
    assert_eq!(&r.bytes().await.unwrap()[..], b"OggS");
}

#[tokio::test]
async fn leaderboard_checks() {
    let srv = start("scores").await;
    let b = &srv.base;
    let song = "Fixture Band/Rich Song.feedpak";
    let (s, run) = post(b, "/api/runs", json!({"filename": song, "arrangement": 0})).await;
    assert_eq!(s, 200);
    let rid = run["run_id"].as_str().unwrap().to_string();
    assert!(run["min_seconds"].as_f64().unwrap() > 20.0);
    // Too fast: rejected, and the ticket is spent.
    let body = json!({"run_id": rid, "name": "Riffer", "hits": 50, "misses": 8, "best_streak": 10});
    assert_eq!(post(b, "/api/scores", body.clone()).await.0, 422);
    assert_eq!(post(b, "/api/scores", body).await.0, 409);
    // Bad input.
    assert_eq!(
        post(b, "/api/runs", json!({"filename": "nope.feedpak"}))
            .await
            .0,
        400
    );
    assert_eq!(
        post(b, "/api/runs", json!({"filename": song, "arrangement": 9}))
            .await
            .0,
        400
    );
    assert_eq!(
        post(b, "/api/runs", json!({"filename": "../Cargo.toml"}))
            .await
            .0,
        400
    );
    for name in ["", "<script>", "😀", "xxxxxxxxxxxxxxxxxxxxx"] {
        let (_, r) = post(b, "/api/runs", json!({"filename": song})).await;
        let s = post(
            b,
            "/api/scores",
            json!({"run_id": r["run_id"], "name": name, "hits": 1, "misses": 0}),
        )
        .await
        .0;
        assert_eq!(s, 400, "{name:?}");
    }
    let (_, board) = reqwest::get(format!(
        "{b}/api/scores/{}",
        song.replace(' ', "%20").replace('/', "%2F")
    ))
    .await
    .map(|r| (0, r))
    .unwrap();
    assert_eq!(board.json::<Value>().await.unwrap()["scores"], json!([]));
}

#[tokio::test]
async fn admin_routes_need_the_token() {
    let srv = start("admin").await;
    let token = std::fs::read_to_string(srv.config.0.join("admin.token")).unwrap();
    let c = reqwest::Client::new();
    let url = format!("{}/api/admin/scores/recent", srv.base);
    assert_eq!(c.get(&url).send().await.unwrap().status().as_u16(), 404);
    assert_eq!(
        c.get(&url)
            .header("x-riffboard-admin", "nope")
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        404
    );
    let ok = c
        .get(&url)
        .header("x-riffboard-admin", token.trim())
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    let behind_proxy = c
        .get(&url)
        .header("x-riffboard-admin", token.trim())
        .header("x-forwarded-for", "203.0.113.5")
        .send()
        .await
        .unwrap();
    assert_eq!(behind_proxy.status().as_u16(), 404);
    // The CLI talks to the same routes.
    let port = srv.base.rsplit(':').next().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_riffboard"))
        .args(["admin", "recent"])
        .env("CONFIG_DIR", &srv.config.0)
        .env("PORT", port)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("No scores yet"));
}

#[tokio::test]
async fn websocket_stays_open_after_ready() {
    let srv = start("ws").await;
    let url = format!(
        "{}/ws/highway/Fixture%20Band/Plain%20Bass.feedpak",
        srv.base.replace("http", "ws")
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut saw_ready = false;
    while let Some(Ok(Message::Text(t))) = ws.next().await {
        if t.contains("\"ready\"") {
            saw_ready = true;
            break;
        }
    }
    assert!(saw_ready);
    ws.send(Message::Text(r#"{"action":"change_arrangement"}"#.into()))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(300), ws.next())
            .await
            .is_err(),
        "socket should idle, not close"
    );
}
