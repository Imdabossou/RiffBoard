"""RiffBoard's public-site guarantees: the route allowlist, the leaderboard's
score checks, and the operator-only admin routes."""

from __future__ import annotations

import importlib
import json
import sys

import pytest
import yaml
from fastapi.testclient import TestClient

import public_guard


def _arr(n_notes, n_chords=0, step=1.0):
    notes = [{"t": i * step, "s": 0, "f": 3} for i in range(n_notes)]
    chords = [{"t": (n_notes + i) * step, "id": 0,
               "notes": [{"t": (n_notes + i) * step, "s": 0, "f": 3},
                         {"t": (n_notes + i) * step, "s": 1, "f": 5}]}
              for i in range(n_chords)]
    return {"notes": notes, "chords": chords, "anchors": [], "handshapes": [],
            "templates": [{"name": "X", "frets": [3, 5, -1, -1, -1, -1],
                           "fingers": [1, 3, -1, -1, -1, -1]}],
            "beats": [{"time": 0.0, "measure": 1}],
            "sections": [{"name": "intro", "number": 1, "time": 0.0}]}


def _write_pak(dlc, name="song", n_notes=20, n_chords=5):
    pak = dlc / f"{name}.feedpak"
    (pak / "arrangements").mkdir(parents=True)
    (pak / "arrangements" / "lead.json").write_text(json.dumps(_arr(n_notes, n_chords)))
    manifest = {"title": name.title(), "artist": "Tester", "album": "", "year": 2026,
                "duration": 30.0,
                "arrangements": [{"id": "lead", "name": "Lead", "file": "arrangements/lead.json"}],
                "stems": []}
    (pak / "manifest.yaml").write_text(yaml.safe_dump(manifest, sort_keys=False))
    return pak


@pytest.fixture()
def rb(tmp_path, monkeypatch):
    cfg, dlc = tmp_path / "config", tmp_path / "dlc"
    cfg.mkdir()
    dlc.mkdir()
    _write_pak(dlc)
    monkeypatch.setenv("CONFIG_DIR", str(cfg))
    monkeypatch.setenv("DLC_DIR", str(dlc))
    monkeypatch.setenv("FEEDBACK_SYNC_STARTUP", "1")
    monkeypatch.delenv("RIFFBOARD_CLIENT_IP_HEADER", raising=False)
    sys.modules.pop("server", None)
    server = importlib.import_module("server")
    monkeypatch.setattr(server, "load_plugins", lambda *a, **kw: None)
    monkeypatch.setattr(server, "startup_scan", lambda: None)
    import appstate
    monkeypatch.setattr(appstate, "sloppak_cache_dir", tmp_path / "cache")
    import routers.scores as scores
    scores._runs.clear()
    scores._chart_cache.clear()
    monkeypatch.setattr(scores, "_start_limiter", scores._RateLimiter(1000, 60))
    monkeypatch.setattr(scores, "_submit_limiter", scores._RateLimiter(1000, 60))
    # Freeze the clock the run tickets use so "time passed" is explicit.
    clock = {"t": 1000.0}
    monkeypatch.setattr(scores.time, "monotonic", lambda: clock["t"])
    with TestClient(server.app) as client:
        client.clock = clock
        client.server = server
        client.cfg = cfg
        yield client
    server.meta_db.conn.close()
    server.leaderboard.close()


# ── Allowlist ────────────────────────────────────────────────────────────────
@pytest.mark.parametrize("method,path", [
    ("GET", "/"),
    ("GET", "/static/app.js"),
    ("GET", "/api/library"),
    ("GET", "/api/song/Artist%2Fsong.feedpak"),
    ("GET", "/api/song/a/b.feedpak/art"),
    ("GET", "/api/sloppak/a.feedpak/file/stems/full.ogg"),
    ("GET", "/api/scores/a.feedpak"),
    ("GET", "/api/leaderboards"),
    ("POST", "/api/runs"),
    ("POST", "/api/scores"),
])
def test_allowlist_permits_the_public_surface(method, path):
    assert public_guard.is_allowed(method, path)


@pytest.mark.parametrize("method,path", [
    ("POST", "/api/settings"),
    ("POST", "/api/rescan"),
    ("POST", "/api/songs/upload"),
    ("DELETE", "/api/song/a.feedpak"),
    ("POST", "/api/plugins/x/update"),
    ("POST", "/api/plugins/x/enabled"),
    ("GET", "/api/settings/export"),
    ("GET", "/api/diagnostics/preview"),
    ("GET", "/api/enrichment/search"),
    ("GET", "/api/audio-local-path"),
    ("GET", "/docs"),
    ("GET", "/openapi.json"),
    ("PUT", "/api/scores"),
    ("GET", "/api/admin/scores/recent"),
    ("GET", "/api/song/../../etc/passwd"),
    ("GET", "/static/../server.py"),
])
def test_allowlist_denies_everything_else(method, path):
    assert not public_guard.is_allowed(method, path)


def test_guard_404s_unlisted_routes_and_sets_headers(rb):
    assert rb.post("/api/rescan").status_code == 404
    assert rb.get("/docs").status_code == 404
    r = rb.get("/api/version")
    assert r.status_code == 200
    assert r.headers["x-content-type-options"] == "nosniff"
    assert r.headers["x-frame-options"] == "DENY"
    assert "microphone=(self)" in r.headers["permissions-policy"]


def test_settings_state_round_trip_is_stateless(rb):
    import json as _j
    r = rb.get("/api/settings", params={"patch": _j.dumps({"av_offset_ms": 25, "dlc_dir": "/x"})}).json()
    assert r["av_offset_ms"] == 25 and "dlc_dir" not in r["_state"]
    state = r["_state"]
    # The server kept nothing: a plain GET is back to defaults...
    assert rb.get("/api/settings").json()["av_offset_ms"] == 0
    # ...and the browser's saved state restores the change.
    assert rb.get("/api/settings", params={"state": _j.dumps(state)}).json()["av_offset_ms"] == 25
    bass = rb.get("/api/settings", params={"state": _j.dumps(state),
                                           "patch": _j.dumps({"active_instrument_profile": "bass"})}).json()
    assert bass["instrument"] == "bass" and bass["_state"]["av_offset_ms"] == 25
    assert rb.get("/api/settings", params={"patch": _j.dumps({"master_difficulty": "x"})}).status_code == 400
    # A tampered saved state is dropped, not trusted.
    assert rb.get("/api/settings", params={"state": _j.dumps({"master_difficulty": "x"})}).status_code == 200


def test_settings_are_read_only_defaults(rb):
    s = rb.get("/api/settings").json()
    assert "dlc_dir" not in s
    assert s["master_difficulty"] == 100
    assert rb.post("/api/settings", json={"dlc_dir": "/"}).status_code == 404


def test_operator_player_defaults_only_pass_player_keys(rb):
    (rb.cfg / "config.json").write_text(json.dumps({
        "dlc_dir": "/secret/path",
        "player_defaults": {"av_offset_ms": 40, "dlc_dir": "/nope", "acoustid_api_key": "k"},
    }))
    s = rb.get("/api/settings").json()
    assert s["av_offset_ms"] == 40
    assert "dlc_dir" not in s and "acoustid_api_key" not in s


# ── Leaderboard ──────────────────────────────────────────────────────────────
def _start(rb, fn="song.feedpak", arr=0):
    r = rb.post("/api/runs", json={"filename": fn, "arrangement": arr})
    assert r.status_code == 200, r.text
    return r.json()["run_id"]


def _submit(rb, rid, **kw):
    body = {"run_id": rid, "name": "Riffer", "hits": 20, "misses": 5, "best_streak": 12}
    body.update(kw)
    return rb.post("/api/scores", json=body)


def test_full_run_posts_and_ranks(rb):
    rid = _start(rb)
    rb.clock["t"] += 30
    r = _submit(rb, rid)
    assert r.status_code == 200, r.text
    d = r.json()
    assert d["rank"] == 1
    assert d["score"] == round(20 * 100 * (20 / 25))
    board = rb.get("/api/scores/song.feedpak").json()["scores"]
    assert [row["name"] for row in board] == ["Riffer"]
    rb.get("/api/song/song.feedpak")  # index the song (the scan is stubbed out)
    boards = rb.get("/api/leaderboards").json()["boards"]
    assert boards and boards[0]["title"] == "Song" and boards[0]["top_name"] == "Riffer"


def test_server_computes_score_ignoring_client_numbers(rb):
    rid = _start(rb)
    rb.clock["t"] += 30
    r = _submit(rb, rid, score=999999999, accuracy=1.0)
    assert r.json()["score"] == 1600


def test_too_fast_is_rejected_and_burns_ticket(rb):
    rid = _start(rb)
    rb.clock["t"] += 5
    assert _submit(rb, rid).status_code == 422
    rb.clock["t"] += 60
    assert _submit(rb, rid).status_code == 409


def test_impossible_counts_rejected(rb):
    # chart has 20 notes + 5 chords = 25 judgements
    for hits, misses in [(100, 0), (5, 2)]:
        rid = _start(rb)
        rb.clock["t"] += 30
        assert _submit(rb, rid, hits=hits, misses=misses, best_streak=0).status_code == 422
    rid = _start(rb)
    rb.clock["t"] += 30
    assert _submit(rb, rid, best_streak=21).status_code == 400  # streak > hits


def test_ticket_is_bound_to_the_visitor(rb, monkeypatch):
    rid = _start(rb)
    rb.clock["t"] += 30
    monkeypatch.setattr(public_guard, "client_ip", lambda scope: "203.0.113.9")
    assert _submit(rb, rid).status_code == 409


@pytest.mark.parametrize("name", ["", " ", "x" * 21, "<script>", "\u202e", "😀"])
def test_bad_names_rejected(rb, name):
    rid = _start(rb)
    rb.clock["t"] += 30
    assert _submit(rb, rid, name=name).status_code == 400


def test_best_run_per_name_and_ban(rb):
    for hits in (10, 20, 15):
        rid = _start(rb)
        rb.clock["t"] += 30
        assert _submit(rb, rid, hits=hits, misses=25 - hits, best_streak=0).status_code == 200
    board = rb.get("/api/scores/song.feedpak").json()["scores"]
    assert len(board) == 1 and board[0]["hits"] == 20
    rb.server.leaderboard.ban("  RIFFER ")
    assert rb.get("/api/scores/song.feedpak").json()["scores"] == []
    rid = _start(rb)
    rb.clock["t"] += 30
    assert _submit(rb, rid, name="riffer").status_code == 403


def test_unknown_song_and_arrangement(rb):
    assert rb.post("/api/runs", json={"filename": "nope.feedpak"}).status_code == 400
    assert rb.post("/api/runs", json={"filename": "song.feedpak", "arrangement": 3}).status_code == 400
    assert rb.post("/api/runs", json={"filename": "../etc/passwd"}).status_code == 400


def test_rate_limit(rb, monkeypatch):
    import routers.scores as scores
    monkeypatch.setattr(scores, "_start_limiter", scores._RateLimiter(2, 60))
    _start(rb)
    _start(rb)
    assert rb.post("/api/runs", json={"filename": "song.feedpak"}).status_code == 429


def test_client_ip_header_is_opt_in(monkeypatch):
    scope = {"client": ("10.0.0.2", 1), "headers": [(b"cf-connecting-ip", b"198.51.100.7")]}
    assert public_guard.client_ip(scope) == "10.0.0.2"
    monkeypatch.setenv("RIFFBOARD_CLIENT_IP_HEADER", "CF-Connecting-IP")
    assert public_guard.client_ip(scope) == "198.51.100.7"
    scope["headers"] = [(b"cf-connecting-ip", b"not-an-ip")]
    assert public_guard.client_ip(scope) == "10.0.0.2"


# ── Admin ────────────────────────────────────────────────────────────────────
def _admin_scope(token, client="127.0.0.1", extra=()):
    return {"client": (client, 5), "headers": [(b"x-riffboard-admin", token.encode()), *extra]}


def test_admin_needs_loopback_token_and_no_proxy_headers(rb):
    token = (rb.cfg / "admin.token").read_text().strip()
    assert public_guard.is_admin_request(_admin_scope(token))
    assert not public_guard.is_admin_request(_admin_scope("wrong"))
    assert not public_guard.is_admin_request(_admin_scope(token, client="192.0.2.1"))
    assert not public_guard.is_admin_request(
        _admin_scope(token, extra=[(b"cf-connecting-ip", b"198.51.100.7")]))
    # TestClient's peer is "testclient" (not loopback): the real route stays shut.
    assert rb.get("/api/admin/scores/recent", headers={"x-riffboard-admin": token}).status_code == 404


def test_invisible_characters_are_stripped_from_names():
    import leaderboard
    assert leaderboard.clean_name("Rif\u202efer\u200b") == "Riffer"
    assert leaderboard.clean_name("  Big   Dog ") == "Big Dog"
