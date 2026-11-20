# RiffBoard — agent guide

RiffBoard is a public, play-only rhythm game for guitar and bass: visitors browse a song library, play along on a note highway with in-browser note detection, and post scores to per-song leaderboards. It runs on the open internet, so **every change is judged first by what it lets an anonymous visitor do to the server.**

It began as a fork of fee[dB]ack. The server was rewritten in Rust (0.2), and in 0.4 the inherited JavaScript player was replaced by a Rust game engine compiled to WebAssembly. Upstream's frontend, plugins and their endpoints are gone.

## Map

```
src/main.rs        startup, background rescans, `riffboard admin` / `healthcheck` subcommands
src/guard.rs       THE allowlist middleware + security headers + client_ip() + admin check
src/web.rs         every route, the game page CSP, the chart route, runs/scores checks
src/library.rs     in-memory library index (manifest-only scan) + library queries
src/chart.rs       feedpak -> riffboard_core::chart::Chart per arrangement (stems, lyrics, timeline)
src/pack.rs        path containment, manifests, cover art, zip unpack cache
src/site.rs        band branding: CONFIG_DIR/site/site.json -> /api/site, /site.css, /site/<img>
src/store.rs       leaderboard SQLite (CONFIG_DIR/leaderboard.db)
src/admin.rs       operator CLI (talks to loopback-only /api/admin/*)
crates/core/       shared game rules (no I/O, native tests): chart.rs (the chart model + its JSON),
                   judge.rs (events + tally), rules.rs (score formula + leaderboard checks, used by
                   the server), detect.rs + fft.rs (mic note detector), pitch.rs
crates/core/examples/detect_bench.rs   detector against a recording or a synthesized take
crates/game/       the browser game (wasm): scene.rs (layout, camera, HUD -> instance lists, pure and
                   tested), gl.rs (WebGL2: 2 programs, 4 instanced draws/frame, glyph atlas),
                   lib.rs (State: judge + detector + camera), web.rs (wasm-bindgen API)
static/game/       the page: index.html, app.js (menus/router), player.js (Web Audio, song clock,
                   mic, frame loop), mic-worklet.js, app.css, theme.js; pkg/ = build output (ignored)
static/brand/      RiffBoard icon and default hero image; static/fonts/ = Rubik (opt-in)
scripts/build-game.sh   builds crates/game into static/game/pkg (Docker runs it too)
tests/server.rs    end-to-end tests against the real binary (library, charts, audio, guard, scores)
tests/fixtures/    fixture feedpak library (make_fixtures.py rebuilds it)
```

## Rules that matter

1. **The allowlist is the security boundary.** A route is reachable only if `src/guard.rs::is_allowed` lists its method + path. Adding a route means adding the narrowest possible rule there plus tests (the `allowlist` unit test and `tests/server.rs`), covering both the allow case and the deny cases around it. Never add a rule covering `/api/*`.
2. **Visitors write exactly two things:** run tickets and scores. Anything per-user (settings, name) lives in the browser (localStorage, `rb-settings` / `rb-theme`). Don't add server-side per-user state.
3. **The library is read-only.** Nothing writes into `DLC_DIR`. No seeding, no metadata write-back, no uploads.
4. **Server settings come from the environment / `CONFIG_DIR/config.json`**, never from HTTP.
5. **Admin routes** (`/api/admin/*`) require loopback, no proxy forwarding headers, and the token. Keep it that way.
6. **No third-party requests from the page** (fonts, analytics, CDNs). The CSP in `web.rs` (`GAME_CSP`) enforces same-origin; keep it strict (no inline scripts; `wasm-unsafe-eval` only).
7. **Untrusted text is escaped** (`esc()` in `app.js`) before it goes into HTML. Names are normalised server-side by `store::clean_name`.
8. **Light + dark themes** for the menus: colours are `--fb-*` "R G B" variables in `app.css`, overridden by `/site.css`; the default is monochrome with the system `AccentColor` when unbranded. The game screen (`#play`) is always dark.
9. **Shared rules live in `crates/core`.** The score formula and run checks are one function used by both the server and the game; never re-implement them in JavaScript.
10. **One chart model.** `riffboard_core::chart::Chart` is what the server builds from a pack (`Chart::from_pack` + `finish`) and what the game reads (`GET /api/chart/<song>?arrangement=N`, serde JSON). Add a field there and both sides have it; don't add a second format.

## Smoothness rules (the reason the engine exists)

- Song time comes from the Web Audio output clock (`getOutputTimestamp`), smoothed per frame in `player.js` `Clock`. Never drive the highway from an `<audio>` element's `currentTime`.
- The frame loop (`Session.loop` -> `Game::frame`) allocates nothing: `scene::build` refills reused `Vec`s (a test checks capacity stays fixed), HUD numbers are formatted on the stack, GPU buffers grow by powers of two. No `format!`/`collect` in per-frame paths.
- Nothing touches the DOM during play. Score, streak, progress, section and mic level are drawn by the game. HTML overlays appear only when paused or finished.
- Mic blocks are recycled between the worklet and the page (transfer back), so steady play makes no garbage.
- Measure with a frame-time probe in Chromium (rAF intervals; with `--use-angle=gl` and `DRI_PRIME=1` on the maintainer's machine for the eGPU) and with CDP CPU throttling (6x/20x). Mid-run screenshots stall a frame: don't count those.

## Leaderboard flow

The song page offers **Play for score** (mic required) or **Practice** (no judging; restart and section jumps from the pause menu). Score mode: `POST /api/runs {filename, arrangement}` before the lead-in returns a one-use `run_id` bound to the visitor's IP; the detector judges every note and chord; at the end `Game::finish` marks unjudged events as misses and the result panel offers name entry; `POST /api/scores {run_id, name, hits, misses, best_streak}` is accepted only if ≥ 95 % of the chart's last-note time has elapsed since the ticket, judged events ≤ notes + chords (+5 %) and ≥ 50 %, and the name isn't banned. Score = round(hits × 100 × accuracy) (`rules::score`).

## Note detector

`crates/core/src/detect.rs`: 4096-point FFT every 512 samples; for each event whose window (`judge::EARLY`/`LATE`) is open it compares the harmonic salience of the expected pitch with the semitones either side (harmonics weighted by how far apart semitones are in bins, so bass notes lean on upper harmonics), requires the input above a tracked noise floor, and a rise over the level just before the window (a ringing note isn't a new hit). Muted/harmonic notes are judged on attack only. `Sensitivity::from_level` maps the leniency slider. Benchmark: `detect_bench` with `synth` (clean take: ~93 % right, 2-6 % wrong-fret at the default) or a decoded recording; check right vs `shift` ±1 (wrong fret), not just the hit rate.

## Testing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p riffboard-game --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace                       # unit + end-to-end (spawns the binary)
bash scripts/build-game.sh                   # needs wasm-bindgen CLI == the pinned version
for f in static/game/*.js; do node --check "$f"; done
```

Run locally: `bash scripts/build-game.sh && DLC_DIR=<folder of .feedpak> CONFIG_DIR=<tmp> cargo run`. On the maintainer's mini PC keep builds light: `CARGO_BUILD_JOBS=2`. The fixture packs' audio is a stub; to play in a browser use a pack with real audio.

## Repo conventions

- Branches: `main` (released) and `development` (work). Changes land on `development` first.
- Licence: AGPL-3.0-only (inherited from upstream). The page footer links the source (`APP_SOURCE_URL`).
- Don't put deployment details (hosts, IPs, ports, paths of a real deployment) in this repo.
