"""Leaderboard routes: run tickets, score submission, boards, operator tools.

The browser does the note detection, so the server cannot see a performance;
it can only make cheating tedious and keep the board clean:

* A run starts with ``POST /api/runs``, which hands out a one-use ticket and
  notes the time. A score is accepted only against a live ticket, from the same
  visitor, after at least ~95% of the chart's length has passed in real time.
  A faked score therefore has to sit through the song.
* The submitted hit/miss counts must fit the chart: no more judgements than
  the arrangement has notes + chords, and at least half of them (a full run
  judges every note it passes; an early quit never reaches the end anyway).
* The server computes score and accuracy itself (lib/song_score.py); the
  client's numbers are not trusted.
* Per-visitor rate limits on both writes; names are normalised and can be
  banned by the operator.

Operator routes under /api/admin/ are reachable only from loopback with the
admin token (lib/public_guard.py); tools/riffboard_admin.py is the client.
"""

from __future__ import annotations

import collections
import math
import secrets
import threading
import time
from urllib.parse import quote

from fastapi import APIRouter, Request
from fastapi.responses import JSONResponse

import appstate
import leaderboard as lb_mod
import public_guard
import sloppak as sloppak_mod
import song_score
from dlc_paths import _get_dlc_dir, _resolve_dlc_path

import logging
log = logging.getLogger("riffboard.scores")
router = APIRouter()

# ── Tunables ─────────────────────────────────────────────────────────────────
MIN_ELAPSED_FRACTION = 0.95      # of the chart's last-note time
ELAPSED_GRACE_S = 3.0            # count-in / load jitter
MAX_JUDGED_SLACK = 1.05          # tolerance over notes + chords
MIN_JUDGED_FRACTION = 0.5
RUN_TTL_EXTRA_S = 900            # ticket lifetime beyond the song itself
MAX_RUNS_PER_IP = 4
MAX_RUNS_TOTAL = 5000
RUN_STARTS_PER_WINDOW = (40, 600)    # 40 starts / 10 min per visitor
SUBMITS_PER_WINDOW = (20, 3600)      # 20 scores / hour per visitor


class _RateLimiter:
    def __init__(self, limit: int, window_s: float):
        self.limit, self.window = limit, window_s
        self._hits: dict[str, collections.deque] = {}
        self._lock = threading.Lock()

    def allow(self, key: str) -> bool:
        now = time.monotonic()
        with self._lock:
            dq = self._hits.setdefault(key, collections.deque())
            while dq and now - dq[0] > self.window:
                dq.popleft()
            if len(dq) >= self.limit:
                return False
            dq.append(now)
            if len(self._hits) > 20000:  # bound memory under a flood
                for k in [k for k, v in self._hits.items() if not v][:10000]:
                    self._hits.pop(k, None)
            return True


_start_limiter = _RateLimiter(*RUN_STARTS_PER_WINDOW)
_submit_limiter = _RateLimiter(*SUBMITS_PER_WINDOW)

_runs: dict[str, dict] = {}
_runs_lock = threading.Lock()

# (filename, arrangement, mtime) -> chart facts
_chart_cache: dict[tuple, dict] = {}
_chart_cache_lock = threading.Lock()


def _ip(request: Request) -> str:
    return public_guard.client_ip(request.scope)


def _canonical(filename: str):
    """(dlc, resolved path, library key) for a library song, or None."""
    dlc = _get_dlc_dir()
    if not dlc or not isinstance(filename, str) or not filename:
        return None
    p = _resolve_dlc_path(dlc, filename)
    if p is None or not p.exists() or not sloppak_mod.is_sloppak(p):
        return None
    try:
        key = p.relative_to(dlc.resolve()).as_posix()
    except ValueError:
        return None
    return dlc, p, key


def chart_facts(key: str, arrangement: int, dlc, path) -> dict | None:
    """Judgement count + last-event time for one arrangement, cached by mtime."""
    try:
        mtime = path.stat().st_mtime
    except OSError:
        return None
    ck = (key, arrangement, mtime)
    with _chart_cache_lock:
        if ck in _chart_cache:
            return _chart_cache[ck]
    try:
        loaded = sloppak_mod.load_song(key, dlc, appstate.sloppak_cache_dir)
    except Exception:
        log.warning("leaderboard: could not load %s", key, exc_info=True)
        return None
    arrs = loaded.song.arrangements
    if not (0 <= arrangement < len(arrs)):
        return None
    arr = arrs[arrangement]
    times = [n.time for n in arr.notes] + [c.time for c in arr.chords]
    if not times:
        return None
    facts = {
        "judgeable": len(arr.notes) + len(arr.chords),
        "last_time": max(times),
        "name": arr.name,
    }
    with _chart_cache_lock:
        if len(_chart_cache) > 2000:
            _chart_cache.clear()
        _chart_cache[ck] = facts
    return facts


def _as_count(v) -> int | None:
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        return None
    if isinstance(v, float) and (not math.isfinite(v) or v != int(v)):
        return None
    v = int(v)
    return v if 0 <= v <= 1_000_000 else None


def _prune_runs(now: float) -> None:
    for rid in [r for r, d in _runs.items() if d["expires"] < now]:
        _runs.pop(rid, None)


def _bad(msg: str, status: int = 400):
    return JSONResponse({"error": msg}, status_code=status)


# ── Public routes ────────────────────────────────────────────────────────────
@router.post("/api/runs")
def start_run(data: dict, request: Request):
    ip = _ip(request)
    if not _start_limiter.allow(ip):
        return _bad("slow down", 429)
    arrangement = _as_count(data.get("arrangement", 0))
    found = _canonical(data.get("filename"))
    if found is None or arrangement is None:
        return _bad("unknown song")
    dlc, path, key = found
    facts = chart_facts(key, arrangement, dlc, path)
    if facts is None:
        return _bad("this arrangement can't be scored")
    now = time.monotonic()
    rid = secrets.token_urlsafe(18)
    with _runs_lock:
        _prune_runs(now)
        mine = sorted((d["started"], r) for r, d in _runs.items() if d["ip"] == ip)
        while len(mine) >= MAX_RUNS_PER_IP:
            _runs.pop(mine.pop(0)[1], None)
        if len(_runs) >= MAX_RUNS_TOTAL:
            return _bad("busy, try again shortly", 503)
        _runs[rid] = {
            "ip": ip, "filename": key, "arrangement": arrangement, "started": now,
            "expires": now + facts["last_time"] + RUN_TTL_EXTRA_S,
        }
    return {"run_id": rid, "min_seconds": round(facts["last_time"] * MIN_ELAPSED_FRACTION, 1)}


@router.post("/api/scores")
def submit_score(data: dict, request: Request):
    ip = _ip(request)
    if not _submit_limiter.allow(ip):
        return _bad("slow down", 429)
    rid = data.get("run_id")
    if not isinstance(rid, str):
        return _bad("missing run")
    with _runs_lock:
        _prune_runs(time.monotonic())
        run = _runs.pop(rid, None)
    if run is None or run["ip"] != ip:
        return _bad("this run has expired - play it again to post a score", 409)

    name = lb_mod.clean_name(data.get("name"))
    if name is None:
        return _bad(f"names are 1-{lb_mod.NAME_MAX} letters, numbers, spaces or . ' _ -")
    board = appstate.leaderboard
    if board.is_banned(name):
        return _bad("that name can't be used", 403)

    hits, misses = _as_count(data.get("hits")), _as_count(data.get("misses"))
    streak = _as_count(data.get("best_streak", 0))
    if hits is None or misses is None or streak is None or streak > hits:
        return _bad("invalid result")

    found = _canonical(run["filename"])
    if found is None:
        return _bad("unknown song")
    dlc, path, key = found
    facts = chart_facts(key, run["arrangement"], dlc, path)
    if facts is None:
        return _bad("this arrangement can't be scored")

    elapsed = time.monotonic() - run["started"]
    if elapsed + ELAPSED_GRACE_S < facts["last_time"] * MIN_ELAPSED_FRACTION:
        return _bad("that run finished too quickly to count", 422)
    judged = hits + misses
    if judged > facts["judgeable"] * MAX_JUDGED_SLACK + 2:
        return _bad("more notes judged than the chart has", 422)
    if judged < facts["judgeable"] * MIN_JUDGED_FRACTION:
        return _bad("only full runs count for the leaderboard", 422)

    score = song_score.score(hits, misses)
    accuracy = song_score.accuracy(hits, misses)
    sid = board.add(filename=key, arrangement=run["arrangement"], name=name, score=score,
                    accuracy=accuracy, hits=hits, misses=misses, best_streak=streak, ip=ip)
    log.info("score %s on %s[%d]: %d", name, key, run["arrangement"], score)
    return {
        "id": sid, "score": score, "accuracy": round(accuracy, 4),
        "rank": board.rank_of(key, run["arrangement"], sid),
        "board": board.top(key, run["arrangement"], 10),
    }


@router.get("/api/scores/{filename:path}")
def get_board(filename: str, arrangement: int = 0, limit: int = 10):
    found = _canonical(filename)
    if found is None:
        return _bad("unknown song", 404)
    return {"filename": found[2], "arrangement": arrangement,
            "scores": appstate.leaderboard.top(found[2], arrangement, limit)}


@router.get("/api/leaderboards")
def list_boards():
    """Every board with at least one run, with the song's title/artist."""
    out = []
    conn = appstate.meta_db.conn
    for row in appstate.leaderboard.summary():
        meta = conn.execute(
            "SELECT title, artist, arrangements FROM songs WHERE filename = ?",
            (row["filename"],),
        ).fetchone()
        if not meta:
            continue  # song removed from the library: keep the scores, hide the board
        title, artist, arrs = meta
        arr_name = ""
        try:
            import json
            arr_list = json.loads(arrs) if isinstance(arrs, str) else (arrs or [])
            arr_name = next((a.get("name", "") for a in arr_list
                             if a.get("index") == row["arrangement"]), "")
        except Exception:
            pass
        out.append({**row, "title": title or row["filename"], "artist": artist or "",
                    "arrangement_name": arr_name,
                    "art_url": f"/api/song/{quote(row['filename'])}/art"})
    out.sort(key=lambda r: (r["artist"].casefold(), r["title"].casefold(), r["arrangement"]))
    return {"boards": out}


# ── Operator routes (loopback + admin token; see public_guard) ────────────────
@router.post("/api/admin/rescan")
def admin_rescan(data: dict | None = None):
    force = bool((data or {}).get("force", True))
    started = appstate.kick_scan(force=force)
    return {"started": started}


@router.get("/api/admin/scores/recent")
def admin_recent(limit: int = 50):
    return {"scores": appstate.leaderboard.recent(limit)}


@router.post("/api/admin/scores/delete")
def admin_delete(data: dict):
    sid = _as_count(data.get("id"))
    if sid is None:
        return _bad("id required")
    return {"deleted": appstate.leaderboard.delete(sid)}


@router.post("/api/admin/names/ban")
def admin_ban(data: dict):
    name = data.get("name")
    if not isinstance(name, str) or not name.strip():
        return _bad("name required")
    return {"removed_scores": appstate.leaderboard.ban(name, purge=bool(data.get("purge", True)))}


@router.post("/api/admin/names/unban")
def admin_unban(data: dict):
    name = data.get("name")
    if not isinstance(name, str) or not name.strip():
        return _bad("name required")
    return {"unbanned": appstate.leaderboard.unban(name)}


@router.post("/api/admin/scores/wipe")
def admin_wipe(data: dict):
    fn = data.get("filename")
    if not isinstance(fn, str) or not fn:
        return _bad("filename required")
    arr = data.get("arrangement")
    arr = _as_count(arr) if arr is not None else None
    return {"removed": appstate.leaderboard.wipe_song(fn, arr)}
