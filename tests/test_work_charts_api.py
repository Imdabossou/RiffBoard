"""Tests for the multi-chart work API (P5b): GET /api/work/{work_key}/charts +
PUT/DELETE …/preferred + POST /api/chart/{filename}/split|unsplit. HTTP surface
over the P5a grouping engine (what the P5d Charts drawer consumes)."""

import importlib
import sys

import pytest
from fastapi.testclient import TestClient


@pytest.fixture()
def server(tmp_path, monkeypatch, isolate_logging):
    monkeypatch.setenv("CONFIG_DIR", str(tmp_path))
    monkeypatch.setenv("FEEDBACK_SKIP_STARTUP_TASKS", "1")
    sys.modules.pop("server", None)
    srv = importlib.import_module("server")
    try:
        yield srv
    finally:
        conn = getattr(getattr(srv, "meta_db", None), "conn", None)
        if conn is not None:
            getattr(__import__("sys").modules.get("server"), "_join_background_db_threads", lambda: None)()
            conn.close()
        sys.modules.pop("server", None)


@pytest.fixture()
def client(server):
    return TestClient(server.app)


def _put(server, fn, title, artist, arrangements=1):
    arr = [{"name": "Lead", "index": i} for i in range(arrangements)]
    server.meta_db.put(fn, 0, 0, {"title": title, "artist": artist, "arrangements": arr})


def _wk(server, fn):
    return server.meta_db.work_key_for(fn)


def _rep(body):
    return next(c for c in body["charts"] if c["is_representative"])


# ── grouped grid row carries work_key (so the drawer can address it) ─────────

def test_grouped_row_exposes_work_key(client, server):
    _put(server, "a.archive", "Song", "Artist")
    _put(server, "b.archive", "Song", "Artist")
    row = client.get("/api/library", params={"group": 1}).json()["songs"][0]
    assert row["work_key"] == _wk(server, "a.archive")
    assert row["chart_count"] == 2


# ── GET charts ───────────────────────────────────────────────────────────────

def test_get_work_charts(client, server):
    _put(server, "a.archive", "Song", "Artist", arrangements=2)
    _put(server, "b.archive", "Song", "Artist", arrangements=1)
    body = client.get(f"/api/work/{_wk(server, 'a.archive')}/charts").json()
    assert body["count"] == 2
    assert {c["filename"] for c in body["charts"]} == {"a.archive", "b.archive"}
    assert _rep(body)["filename"] == "a.archive"          # auto-pick = most arrangements
    assert body["preferred_source"] == "auto"
    assert body["preferred_filename"] is None


def test_charts_include_best_accuracy(client, server):
    _put(server, "a.archive", "Song", "Artist")
    _put(server, "b.archive", "Song", "Artist")
    server.meta_db.record_session("a.archive", 0, score=900, accuracy=0.9)
    charts = {c["filename"]: c for c in
              client.get(f"/api/work/{_wk(server, 'a.archive')}/charts").json()["charts"]}
    assert charts["a.archive"]["best_accuracy"] == 0.9
    assert charts["b.archive"]["best_accuracy"] is None


# ── PUT / DELETE preferred ───────────────────────────────────────────────────







# ── split / unsplit ──────────────────────────────────────────────────────────



# ── GET chart work membership (P5d — tree/ungrouped opener resolve) ──────────







