#!/usr/bin/env python3
"""Record reference responses from the original Python server.

For every line of urls.txt ("GET <path>" or "WS <path>") against the fixture
library (tests/fixtures/make_fixtures.py), write the JSON response (or the
full list of WebSocket messages) into golden.json. The Rust server's test
suite replays the same URLs and compares.

Needs the original Python server: run from a checkout of tag v0.1.0
(pip install -r requirements.txt there), e.g.:
    python tests/golden/capture.py FIXTURE_DIR tests/golden/golden.json
"""

import importlib
import json
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path[:0] = [str(ROOT), str(ROOT / "lib")]


def main(fixture_dir: str, out: str):
    cfg = tempfile.mkdtemp(prefix="rb-golden-")
    os.environ.update(DLC_DIR=fixture_dir, CONFIG_DIR=cfg, FEEDBACK_SYNC_STARTUP="1",
                      FEEDBACK_SKIP_STARTUP_TASKS="1")
    server = importlib.import_module("server")
    import scan
    scan.background_scan(force=True)
    from fastapi.testclient import TestClient

    results = {}
    with TestClient(server.app) as c:
        for line in (HERE / "urls.txt").read_text().splitlines():
            if not line.strip():
                continue
            kind, url = line.split(" ", 1)
            if kind == "GET":
                r = c.get(url)
                try:
                    body = r.json()
                except ValueError:
                    body = None
                results[line] = {"status": r.status_code, "body": body}
            elif kind == "WS":
                msgs = []
                try:
                    with c.websocket_connect(url) as ws:
                        while True:
                            m = ws.receive_json()
                            msgs.append(m)
                            if m.get("type") == "ready" or "error" in m:
                                break
                except Exception as e:  # server closed after an error frame
                    msgs.append({"_closed": type(e).__name__})
                results[line] = {"messages": msgs}
    Path(out).write_text(json.dumps(results, indent=1, sort_keys=True))
    print(f"{len(results)} responses -> {out}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
