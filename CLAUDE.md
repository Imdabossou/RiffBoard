# RiffBoard — agent guide

RiffBoard is a public, play-only fork of fee[dB]ack: visitors browse a song library, play along on a note highway with in-browser note detection, and post scores to per-song leaderboards. It runs on the open internet, so **every change is judged first by what it lets an anonymous visitor do to the server.**

Upstream's long developer guide (plugin contracts, renderer API, WebSocket protocol) is kept at [docs/upstream-developer-guide.md](docs/upstream-developer-guide.md). Where it disagrees with this file, this file wins.

## Map

```
server.py                    FastAPI wiring only (middleware, routers, startup scan)
main.py                      uvicorn entry point
lib/public_guard.py          THE allowlist middleware + security headers + client_ip()
lib/admin_token.py           loopback admin token (written to CONFIG_DIR/admin.token)
lib/leaderboard.py           scores SQLite store (CONFIG_DIR/leaderboard.db)
lib/routers/scores.py        POST /api/runs, POST /api/scores, boards, /api/admin/*
lib/routers/public_settings.py  stateless GET /api/settings?state=&patch=
lib/routers/*.py             read-only library/song/art/media/tunings/version/ws_highway
lib/sloppak.py, song.py ...  pack reading (upstream)
static/riffboard/            RiffBoard frontend: local-api.js, leaderboard.js, riffboard.css
static/v3/                   upstream v3 shell, trimmed (shell.js, songs.js, dashboard.js...)
static/js/, static/app.js    upstream player engine
plugins/                     bundled: highway_3d, drum/keys_highway_3d, tuner, input_setup, note_detect
tools/riffboard_admin.py     operator CLI (rescan, moderate scores)
tests/                       pytest (server) + tests/js (node --test)
```

## Rules that matter

1. **The allowlist is the security boundary.** A route is reachable only if `lib/public_guard.py` lists its method + path. Adding a route means adding the narrowest possible entry there and a test in `tests/test_riffboard_public.py` (both an allow case and the deny cases around it). Never add a wildcard covering `/api/*`.
2. **Visitors write exactly two things:** run tickets and scores. Anything per-user (settings, favourites, loops, stats, tuner config) lives in the browser via `static/riffboard/local-api.js`, which answers those upstream endpoints from localStorage. Don't add server-side per-user state.
3. **The library is read-only.** Nothing writes into `DLC_DIR`. No seeding, no metadata write-back, no uploads.
4. **Server settings come from the environment / `CONFIG_DIR/config.json`**, never from HTTP. `/api/settings` returns only `PLAYER_KEYS`.
5. **Admin routes** (`/api/admin/*`) require loopback, no proxy forwarding headers, and the token. Keep it that way.
6. **No third-party requests from the page** (fonts, analytics, CDNs). The only exception is the note detector's opt-in CREPE model loader.
7. **Untrusted text is escaped** (`esc()` in every frontend module). Names are normalised server-side by `leaderboard.clean_name`.
8. **Light + dark themes.** Colours are CSS variables (`static/riffboard/riffboard.css`, `tailwind.config.js`). Use `fb-*` Tailwind tokens, never raw hex, in new UI. Run `bash scripts/build-tailwind.sh` after adding classes and commit `static/tailwind.min.css`. The player (`#player`) and `.rb-on-dark` surfaces stay dark.
9. **Keep the diff from upstream small** where possible: hide upstream UI with CSS (`riffboard.css`, "Public build" section) instead of deleting deep inside `songs.js`, and answer removed endpoints in `local-api.js` instead of editing every caller.

## Leaderboard flow

`song:loading` → first `song:play` → `POST /api/runs {filename, arrangement}` returns a one-use `run_id` bound to the visitor's IP. `note:hit`/`note:miss` events (from note_detect) are tallied in `static/riffboard/leaderboard.js`; seeks > 1 s, loop restarts, playback rate ≠ 1, mastery < 100 % or a part switch mark the run as practice. On `song:ended` the result modal offers name entry; `POST /api/scores` is accepted only if ≥ 95 % of the chart's last-note time has elapsed since the ticket, judged notes ≤ notes + chords (+5 %) and ≥ 50 %, and the name isn't banned. The server computes score = round(hits × 100 × accuracy).

## Testing

```bash
.venv/bin/python -m pytest -q          # ~1150 tests, a few seconds
node --test tests/js/*.test.js
node --test plugins/note_detect/test/*.test.js
```

Run the app locally with `PYTHONPATH=lib:. DLC_DIR=<folder of .feedpak> CONFIG_DIR=<tmp> .venv/bin/python main.py`.

## Repo conventions

- Branches: `main` (released) and `development` (work). Changes land on `development` first.
- Licence: AGPL-3.0-only (inherited from upstream). `plugins/note_detect` is MIT.
- Don't put deployment details (hosts, IPs, ports, paths of a real deployment) in this repo.
