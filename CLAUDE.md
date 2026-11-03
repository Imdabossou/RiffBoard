# RiffBoard — agent guide

RiffBoard is a public, play-only fork of fee[dB]ack: visitors browse a song library, play along on a note highway with in-browser note detection, and post scores to per-song leaderboards. It runs on the open internet, so **every change is judged first by what it lets an anonymous visitor do to the server.**

Upstream's long developer guide (plugin contracts, renderer API, WebSocket protocol) is kept at [docs/upstream-developer-guide.md](docs/upstream-developer-guide.md). Where it disagrees with this file, this file wins.

## Map

```
src/main.rs        startup, background rescans, `riffboard admin` / `healthcheck` subcommands
src/guard.rs       THE allowlist middleware + security headers + client_ip() + admin check
src/web.rs         every route, the highway WebSocket, runs/scores checks
src/library.rs     in-memory library index (manifest-only scan) + library queries
src/chart.rs       feedpak -> chart model -> highway WebSocket messages (wire format)
src/pack.rs        path containment, manifests, cover art, zip unpack cache
src/tunings.rs     tuning names/presets/perspectives + instrument profiles
src/settings.rs    stateless GET /api/settings?state=&patch=
src/store.rs       leaderboard SQLite (CONFIG_DIR/leaderboard.db)
src/admin.rs       operator CLI (talks to loopback-only /api/admin/*)
static/riffboard/  RiffBoard frontend: local-api.js, modes.js (score/practice + song map), leaderboard.js, site.js, riffboard.css
static/v3/, static/js/, static/app.js   upstream frontend, trimmed
plugins/           bundled: highway_3d, tuner, input_setup, note_detect (+ registry.json = /api/plugins)
tests/server.rs    end-to-end tests against the real binary; replays tests/golden/golden.json
tests/fixtures/    fixture feedpak library (make_fixtures.py rebuilds it)
tests/js/          frontend unit tests (node --test)
```

## Rules that matter

1. **The allowlist is the security boundary.** A route is reachable only if `src/guard.rs::is_allowed` lists its method + path. Adding a route means adding the narrowest possible rule there plus tests (the `allowlist` unit test and `tests/server.rs`), covering both the allow case and the deny cases around it. Never add a rule covering `/api/*`.
2. **Visitors write exactly two things:** run tickets and scores. Anything per-user (settings, favourites, loops, stats, tuner config) lives in the browser via `static/riffboard/local-api.js`, which answers those upstream endpoints from localStorage. Don't add server-side per-user state.
3. **The library is read-only.** Nothing writes into `DLC_DIR`. No seeding, no metadata write-back, no uploads.
4. **Server settings come from the environment / `CONFIG_DIR/config.json`**, never from HTTP. `/api/settings` returns only `PLAYER_KEYS`.
10. **Wire compatibility.** The frontend expects exactly what upstream's Python server sent. `tests/golden/golden.json` pins that; if a response must change, change the frontend with it and re-record deliberately.
5. **Admin routes** (`/api/admin/*`) require loopback, no proxy forwarding headers, and the token. Keep it that way.
6. **No third-party requests from the page** (fonts, analytics, CDNs). The only exception is the note detector's opt-in CREPE model loader.
7. **Untrusted text is escaped** (`esc()` in every frontend module). Names are normalised server-side by `leaderboard.clean_name`.
8. **Light + dark themes.** Colours are CSS variables (`static/riffboard/riffboard.css`, `tailwind.config.js`). Use `fb-*` Tailwind tokens, never raw hex, in new UI. Run `bash scripts/build-tailwind.sh` after adding classes and commit `static/tailwind.min.css`. The player (`#player`) and `.rb-on-dark` surfaces stay dark.
9. **Keep the diff from upstream small** where possible: hide upstream UI with CSS (`riffboard.css`, "Public build" section) instead of deleting deep inside `songs.js`, and answer removed endpoints in `local-api.js` instead of editing every caller.

## Leaderboard flow

Every user-started song goes through the mode chooser in `static/riffboard/modes.js` (it wraps `window.playSong`; `riffboard.play(file, arr, mode)` skips it). **Practice** runs never ask for a ticket. **Score** mode hides the seek/loop/speed/difficulty controls (CSS "Play modes" in `riffboard.css`), swallows seek keys, and treats any pause of `#audio` as the end of the run. Loads started elsewhere (resume, queue, plugins) default to practice.

`song:loading` → first `song:play` → `POST /api/runs {filename, arrangement}` returns a one-use `run_id` bound to the visitor's IP. `note:hit`/`note:miss` events (from note_detect) are tallied in `static/riffboard/leaderboard.js`; seeks > 1 s, loop restarts, playback rate ≠ 1, mastery < 100 % or a part switch mark the run as practice. On `song:ended` the result modal offers name entry; `POST /api/scores` is accepted only if ≥ 95 % of the chart's last-note time has elapsed since the ticket, judged notes ≤ notes + chords (+5 %) and ≥ 50 %, and the name isn't banned. The server computes score = round(hits × 100 × accuracy).

## Testing

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test                                   # unit + end-to-end (spawns the binary)
node --test tests/js/*.test.js
node --test plugins/note_detect/test/*.test.js
```

Run locally: `DLC_DIR=<folder of .feedpak> CONFIG_DIR=<tmp> cargo run`. On the maintainer's mini PC keep builds light: `CARGO_BUILD_JOBS=2`.

## Repo conventions

- Branches: `main` (released) and `development` (work). Changes land on `development` first.
- Licence: AGPL-3.0-only (inherited from upstream). `plugins/note_detect` is MIT.
- Don't put deployment details (hosts, IPs, ports, paths of a real deployment) in this repo.
