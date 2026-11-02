#!/usr/bin/env python3
"""RiffBoard operator tool: rescan the library and moderate the leaderboard.

Runs next to the server (inside the container: ``docker exec <name> python
tools/riffboard_admin.py ...``). It talks to the server's /api/admin/ routes on
loopback, authenticated with the token the server writes to
``$CONFIG_DIR/admin.token`` at startup. Those routes refuse anything that isn't
loopback, carries proxy headers, or lacks the token, so they are never
reachable from the public site.

Commands:
  rescan                      pick up new/removed packs now (default: every 5 min)
  recent [-n N]               newest scores with ids
  delete ID [ID ...]          delete individual scores
  ban NAME [--keep-scores]    block a name (and remove its scores)
  unban NAME                  allow a name again
  wipe FILENAME [--arrangement N]   clear a song's board (or one part's)
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path


def _token() -> str:
    cfg = Path(os.environ.get("CONFIG_DIR", "/config"))
    try:
        return (cfg / "admin.token").read_text().strip()
    except OSError as exc:
        sys.exit(f"Can't read {cfg / 'admin.token'} ({exc}). Is the server running, "
                 "and is CONFIG_DIR the same as the server's?")


def _call(method: str, path: str, body: dict | None = None) -> dict:
    port = os.environ.get("PORT", "8000")
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"x-riffboard-admin": _token(), "content-type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode(errors="replace")
        sys.exit(f"{method} {path} failed: HTTP {exc.code} {detail}")
    except urllib.error.URLError as exc:
        sys.exit(f"Can't reach the server on 127.0.0.1:{port} ({exc.reason}).")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("rescan", help="scan the library for new or removed packs now")
    p = sub.add_parser("recent", help="list the newest scores")
    p.add_argument("-n", type=int, default=30)
    p = sub.add_parser("delete", help="delete scores by id")
    p.add_argument("ids", type=int, nargs="+")
    p = sub.add_parser("ban", help="ban a display name")
    p.add_argument("name")
    p.add_argument("--keep-scores", action="store_true", help="don't delete its existing scores")
    p = sub.add_parser("unban", help="lift a name ban")
    p.add_argument("name")
    p = sub.add_parser("wipe", help="clear a song's leaderboard")
    p.add_argument("filename", help="library path, e.g. 'Artist/Song.feedpak'")
    p.add_argument("--arrangement", type=int, default=None)
    a = ap.parse_args(argv)

    if a.cmd == "rescan":
        r = _call("POST", "/api/admin/rescan", {"force": True})
        print("Rescan started." if r.get("started") else "A scan is already running; another pass is queued.")
    elif a.cmd == "recent":
        rows = _call("GET", f"/api/admin/scores/recent?limit={a.n}")["scores"]
        if not rows:
            print("No scores yet.")
        for r in rows:
            when = dt.datetime.fromtimestamp(r["created_at"]).strftime("%Y-%m-%d %H:%M")
            print(f"{r['id']:>6}  {when}  {r['score']:>8}  {r['accuracy'] * 100:5.1f}%  "
                  f"{r['name']:<20}  {r['filename']} [{r['arrangement']}]  ip#{r['ip_hash'][:8]}")
    elif a.cmd == "delete":
        for sid in a.ids:
            ok = _call("POST", "/api/admin/scores/delete", {"id": sid}).get("deleted")
            print(f"{sid}: {'deleted' if ok else 'not found'}")
    elif a.cmd == "ban":
        r = _call("POST", "/api/admin/names/ban", {"name": a.name, "purge": not a.keep_scores})
        print(f"Banned {a.name!r}; removed {r.get('removed_scores', 0)} score(s).")
    elif a.cmd == "unban":
        r = _call("POST", "/api/admin/names/unban", {"name": a.name})
        print("Unbanned." if r.get("unbanned") else "That name wasn't banned.")
    elif a.cmd == "wipe":
        body = {"filename": a.filename}
        if a.arrangement is not None:
            body["arrangement"] = a.arrangement
        r = _call("POST", "/api/admin/scores/wipe", body)
        print(f"Removed {r.get('removed', 0)} score(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
