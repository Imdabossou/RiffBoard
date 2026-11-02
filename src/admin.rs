//! Operator CLI: `riffboard admin <command>`. Talks to the running server's
//! loopback-only /api/admin/ routes with the token from CONFIG_DIR.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

pub const USAGE: &str = "admin commands:
  rescan                          pick up new/removed songs now (default: every 5 min)
  recent [N]                      newest scores with their ids (default 30)
  delete ID [ID ...]              delete scores
  ban NAME [--keep-scores]        block a name (and delete its scores)
  unban NAME                      allow a name again
  wipe FILENAME [ARRANGEMENT]     clear a song's board (or one part's)";

fn token() -> Result<String, String> {
    let dir = std::env::var("CONFIG_DIR").unwrap_or_else(|_| "/config".into());
    std::fs::read_to_string(format!("{dir}/admin.token")).map(|t| t.trim().to_string())
        .map_err(|e| format!("can't read {dir}/admin.token ({e}). Is the server running with the same CONFIG_DIR?"))
}

fn call(method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".into());
    let tok = token()?;
    let payload = body.map(Value::to_string).unwrap_or_default();
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}"))
        .map_err(|e| format!("can't reach the server on 127.0.0.1:{port} ({e})"))?;
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nx-riffboard-admin: {tok}\r\n\
               Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len())
        .map_err(|e| e.to_string())?;
    let mut raw = String::new();
    s.read_to_string(&mut raw).map_err(|e| e.to_string())?;
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status != "200" {
        return Err(format!("{method} {path} failed: HTTP {status} {body}"));
    }
    serde_json::from_str(body).map_err(|e| format!("bad response: {e}"))
}

/// Container health probe: GET /api/version on the local port.
pub fn healthcheck() -> i32 {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".into());
    let ok = (|| -> std::io::Result<bool> {
        let mut s = TcpStream::connect(format!("127.0.0.1:{port}"))?;
        s.set_read_timeout(Some(Duration::from_secs(4)))?;
        write!(
            s,
            "GET /api/version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )?;
        let mut buf = [0u8; 16];
        let n = s.read(&mut buf)?;
        Ok(buf[..n].starts_with(b"HTTP/1.1 200"))
    })();
    i32::from(!ok.unwrap_or(false))
}

fn fmt_time(secs: f64) -> String {
    // UTC, minute precision, without pulling in a date crate.
    let s = secs as i64;
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let (h, m) = (rem / 3600, (rem % 3600) / 60);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}Z")
}

pub fn run(args: &[String]) -> i32 {
    let res = (|| -> Result<(), String> {
        let cmd = args.first().map(String::as_str).ok_or(USAGE)?;
        match cmd {
            "rescan" => {
                call("POST", "/api/admin/rescan", Some(&json!({"force": true})))?;
                println!("Rescan started.");
            }
            "recent" => {
                let n: i64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(30);
                let r = call("GET", &format!("/api/admin/scores/recent?limit={n}"), None)?;
                let rows = r["scores"].as_array().cloned().unwrap_or_default();
                if rows.is_empty() {
                    println!("No scores yet.");
                }
                for r in rows {
                    println!(
                        "{:>6}  {}  {:>8}  {:5.1}%  {:<20}  {} [{}]  ip#{}",
                        r["id"],
                        fmt_time(r["created_at"].as_f64().unwrap_or(0.0)),
                        r["score"],
                        r["accuracy"].as_f64().unwrap_or(0.0) * 100.0,
                        r["name"].as_str().unwrap_or(""),
                        r["filename"].as_str().unwrap_or(""),
                        r["arrangement"],
                        &r["ip_hash"].as_str().unwrap_or("")
                            [..8.min(r["ip_hash"].as_str().unwrap_or("").len())]
                    );
                }
            }
            "delete" if args.len() > 1 => {
                for id in &args[1..] {
                    let n: i64 = id.parse().map_err(|_| format!("not an id: {id}"))?;
                    let r = call("POST", "/api/admin/scores/delete", Some(&json!({"id": n})))?;
                    println!(
                        "{n}: {}",
                        if r["deleted"] == true {
                            "deleted"
                        } else {
                            "not found"
                        }
                    );
                }
            }
            "ban" if args.len() > 1 => {
                let keep = args.iter().any(|a| a == "--keep-scores");
                let r = call(
                    "POST",
                    "/api/admin/names/ban",
                    Some(&json!({"name": args[1], "purge": !keep})),
                )?;
                println!(
                    "Banned {:?}; removed {} score(s).",
                    args[1], r["removed_scores"]
                );
            }
            "unban" if args.len() > 1 => {
                let r = call(
                    "POST",
                    "/api/admin/names/unban",
                    Some(&json!({"name": args[1]})),
                )?;
                println!(
                    "{}",
                    if r["unbanned"] == true {
                        "Unbanned."
                    } else {
                        "That name wasn't banned."
                    }
                );
            }
            "wipe" if args.len() > 1 => {
                let mut body = json!({"filename": args[1]});
                if let Some(a) = args.get(2).and_then(|a| a.parse::<i64>().ok()) {
                    body["arrangement"] = json!(a);
                }
                let r = call("POST", "/api/admin/scores/wipe", Some(&body))?;
                println!("Removed {} score(s).", r["removed"]);
            }
            _ => return Err(USAGE.into()),
        }
        Ok(())
    })();
    match res {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
