"""The operator token for /api/admin/* (rescan, score moderation).

A fresh random token is written to ``<CONFIG_DIR>/admin.token`` (mode 0600) at
startup. tools/riffboard_admin.py reads it from the same file, so only someone
with a shell in the container (or on the host, via the config volume) can use
the admin routes. public_guard additionally requires a loopback peer with no
proxy forwarding headers.
"""

from __future__ import annotations

import hmac
import os
import secrets
from pathlib import Path

_token: str | None = None


def init(config_dir: Path) -> Path:
    global _token
    _token = secrets.token_urlsafe(32)
    path = Path(config_dir) / "admin.token"
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(_token + "\n")
    return path


def check(candidate: str | None) -> bool:
    if not _token or not candidate:
        return False
    return hmac.compare_digest(_token.encode(), candidate.strip().encode())
