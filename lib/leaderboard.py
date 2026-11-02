"""Leaderboard store: public highscores per song and arrangement.

One SQLite file (``<CONFIG_DIR>/leaderboard.db``), separate from the library
cache so it can be backed up on its own and the library DB can be rebuilt
freely. Visitors are anonymous: a run records the display name they typed and
a salted hash of their IP (for moderation and per-visitor de-duplication, never
shown). The salt lives next to the DB and never leaves the server.

Ranking keeps each name's best run per (song, arrangement), so one player
cannot fill a board by replaying.
"""

from __future__ import annotations

import hashlib
import os
import re
import secrets
import sqlite3
import threading
import time
import unicodedata
from pathlib import Path

NAME_MAX = 20
_NAME_OK = re.compile(r"^[\w .'\-]+$", re.UNICODE)

_SCHEMA = """
CREATE TABLE IF NOT EXISTS scores (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    filename    TEXT    NOT NULL,
    arrangement INTEGER NOT NULL,
    name        TEXT    NOT NULL,
    name_key    TEXT    NOT NULL,
    score       INTEGER NOT NULL,
    accuracy    REAL    NOT NULL,
    hits        INTEGER NOT NULL,
    misses      INTEGER NOT NULL,
    best_streak INTEGER NOT NULL,
    created_at  REAL    NOT NULL,
    ip_hash     TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS scores_board ON scores (filename, arrangement, score DESC);
CREATE INDEX IF NOT EXISTS scores_name ON scores (name_key);
CREATE TABLE IF NOT EXISTS banned_names (
    name_key   TEXT PRIMARY KEY,
    created_at REAL NOT NULL
);
"""


def clean_name(raw) -> str | None:
    """Normalise a typed display name, or None if it isn't acceptable."""
    if not isinstance(raw, str):
        return None
    name = unicodedata.normalize("NFKC", raw)
    # Drop control/format characters (bidi overrides, zero-width joiners...).
    name = "".join(ch for ch in name if unicodedata.category(ch)[0] != "C")
    name = re.sub(r"\s+", " ", name).strip()
    if not (1 <= len(name) <= NAME_MAX) or not _NAME_OK.match(name):
        return None
    return name


def name_key(name: str) -> str:
    """Case/spacing-insensitive identity of a name (for bans and de-dup)."""
    return re.sub(r"[\s_.\-']+", "", name.casefold())


class Leaderboard:
    def __init__(self, config_dir: Path):
        config_dir = Path(config_dir)
        config_dir.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self.conn = sqlite3.connect(str(config_dir / "leaderboard.db"), check_same_thread=False)
        self.conn.execute("PRAGMA journal_mode=WAL")
        self.conn.executescript(_SCHEMA)
        self.conn.commit()
        salt_file = config_dir / "leaderboard.salt"
        if not salt_file.exists():
            fd = os.open(salt_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w") as f:
                f.write(secrets.token_hex(32))
        self._salt = salt_file.read_text().strip()

    def close(self) -> None:
        with self._lock:
            self.conn.close()

    def ip_hash(self, ip: str) -> str:
        return hashlib.sha256(f"{self._salt}:{ip}".encode()).hexdigest()[:32]

    # ── writes ────────────────────────────────────────────────────────────
    def is_banned(self, name: str) -> bool:
        with self._lock:
            row = self.conn.execute(
                "SELECT 1 FROM banned_names WHERE name_key = ?", (name_key(name),)
            ).fetchone()
        return row is not None

    def add(self, *, filename: str, arrangement: int, name: str, score: int,
            accuracy: float, hits: int, misses: int, best_streak: int, ip: str) -> int:
        with self._lock:
            cur = self.conn.execute(
                "INSERT INTO scores (filename, arrangement, name, name_key, score, accuracy,"
                " hits, misses, best_streak, created_at, ip_hash)"
                " VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                (filename, arrangement, name, name_key(name), score, accuracy, hits,
                 misses, best_streak, time.time(), self.ip_hash(ip)),
            )
            self.conn.commit()
            return int(cur.lastrowid)

    def delete(self, score_id: int) -> bool:
        with self._lock:
            n = self.conn.execute("DELETE FROM scores WHERE id = ?", (score_id,)).rowcount
            self.conn.commit()
        return n > 0

    def ban(self, name: str, purge: bool = True) -> int:
        key = name_key(name)
        with self._lock:
            self.conn.execute(
                "INSERT OR IGNORE INTO banned_names (name_key, created_at) VALUES (?, ?)",
                (key, time.time()),
            )
            removed = 0
            if purge:
                removed = self.conn.execute(
                    "DELETE FROM scores WHERE name_key = ?", (key,)
                ).rowcount
            self.conn.commit()
        return removed

    def unban(self, name: str) -> bool:
        with self._lock:
            n = self.conn.execute(
                "DELETE FROM banned_names WHERE name_key = ?", (name_key(name),)
            ).rowcount
            self.conn.commit()
        return n > 0

    def wipe_song(self, filename: str, arrangement: int | None = None) -> int:
        with self._lock:
            if arrangement is None:
                n = self.conn.execute("DELETE FROM scores WHERE filename = ?", (filename,)).rowcount
            else:
                n = self.conn.execute(
                    "DELETE FROM scores WHERE filename = ? AND arrangement = ?",
                    (filename, arrangement),
                ).rowcount
            self.conn.commit()
        return n

    # ── reads ─────────────────────────────────────────────────────────────
    def top(self, filename: str, arrangement: int, limit: int = 10) -> list[dict]:
        """Best run per name, ranked by score (ties: earlier run wins)."""
        limit = max(1, min(int(limit), 100))
        with self._lock:
            rows = self.conn.execute(
                """
                SELECT id, name, score, accuracy, hits, misses, best_streak, created_at
                FROM (
                    SELECT *, ROW_NUMBER() OVER (
                        PARTITION BY name_key ORDER BY score DESC, created_at ASC
                    ) AS rn
                    FROM scores WHERE filename = ? AND arrangement = ?
                ) WHERE rn = 1
                ORDER BY score DESC, created_at ASC
                LIMIT ?
                """,
                (filename, arrangement, limit),
            ).fetchall()
        return [
            {"rank": i + 1, "id": r[0], "name": r[1], "score": r[2],
             "accuracy": round(r[3], 4), "hits": r[4], "misses": r[5],
             "best_streak": r[6], "created_at": r[7]}
            for i, r in enumerate(rows)
        ]

    def rank_of(self, filename: str, arrangement: int, score_id: int) -> int | None:
        """1-based board position the given run would hold (its name's best)."""
        board = self.top(filename, arrangement, limit=100)
        for row in board:
            if row["id"] == score_id:
                return row["rank"]
        return None

    def summary(self) -> list[dict]:
        """Per (song, arrangement): run count + current best, for the overview."""
        with self._lock:
            rows = self.conn.execute(
                """
                SELECT filename, arrangement, COUNT(*), MAX(score)
                FROM scores GROUP BY filename, arrangement
                """
            ).fetchall()
            best = {}
            for fn, arr, _n, top in rows:
                r = self.conn.execute(
                    "SELECT name FROM scores WHERE filename = ? AND arrangement = ? AND score = ?"
                    " ORDER BY created_at ASC LIMIT 1",
                    (fn, arr, top),
                ).fetchone()
                best[(fn, arr)] = r[0] if r else ""
        return [
            {"filename": fn, "arrangement": arr, "runs": n, "top_score": top,
             "top_name": best[(fn, arr)]}
            for fn, arr, n, top in rows
        ]

    def recent(self, limit: int = 50) -> list[dict]:
        limit = max(1, min(int(limit), 500))
        with self._lock:
            rows = self.conn.execute(
                "SELECT id, filename, arrangement, name, score, accuracy, created_at, ip_hash"
                " FROM scores ORDER BY created_at DESC LIMIT ?",
                (limit,),
            ).fetchall()
        return [
            {"id": r[0], "filename": r[1], "arrangement": r[2], "name": r[3],
             "score": r[4], "accuracy": round(r[5], 4), "created_at": r[6], "ip_hash": r[7]}
            for r in rows
        ]
