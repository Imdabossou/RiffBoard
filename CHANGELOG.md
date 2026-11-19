# Changelog

All notable changes to RiffBoard are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Upstream fee[dB]ack's history up to
the fork is in [docs/upstream-changelog.md](docs/upstream-changelog.md).

## [Unreleased]

### Changed

- **New game engine.** The player is now a Rust game compiled to WebAssembly (about 110 KB gzipped), with its own WebGL2 renderer. The highway is drawn in four batched draw calls a frame with nothing allocated per frame, and the song clock comes from the audio output, so playback is smooth: in testing, no dropped frames at 60 Hz even with the CPU slowed 20 times.
- **New note detector**, written in Rust and shared with the server's rules. It checks the microphone for the expected pitch around each note's time: a wrong fret is a miss, and a note still ringing from earlier doesn't count again. A leniency slider sets how strict it is.
- **New, much smaller site.** Song library, song page with its leaderboard, leaderboards, and settings, in plain HTML. The band's branding, About card and links carry over. Settings: note speed, left-handed mode, string order, detection leniency, microphone delay, picture offset, graphics quality and microphone choice.
- The highway is viewed down the neck: the strings run big across the bottom, with upright fret wires that light up across the current fret window, a ring on each string where the next note will land, and fret numbers underneath.
- The fret window follows your hand, worked out from the notes: it stays put while the notes fit and moves only when one falls outside, so a chart whose anchors flip between windows holding the same notes no longer slides the board about.
- A section navigator runs along the top of the game: every section as a bar whose height shows how busy it is, the current one highlighted, and a playhead. In practice, click a section to jump to it.
- Lyrics show in the top-left corner (current line with sung words lit, and the next line). The score moved to the top-right.
- A look of its own: the band's accent colour (RiffBoard's cyan and violet when unbranded) lights the stage: the fret window and its edges, the fret wires, bar lines, the horizon haze, the navigator and the sung lyrics. Notes are glossy and bevelled and glow as they arrive, sustains are shimmering ribbons, a ringing string glows and blurs as it vibrates, and hits throw sparks.
- The band's artwork (the site's hero image, or RiffBoard's own when unbranded) sits blurred and darkened behind the highway, which fades into it. It can be turned off in Settings.
- Held notes stay on the fretboard, lit, until their sustain ends; notes that pass unplayed fade out at the line.
- Score runs may be paused (they still can't skip or slow down). Practice runs can restart and jump to any section from the pause menu.
- The leaderboard rules (score formula and run checks) now live in `crates/core`, used by both the server and the game.

### Removed

- The inherited fee[dB]ack frontend (about 58,000 lines of JavaScript), its plugins (3D highway, note detection, tuner, input setup) and the endpoints only they used: `/api/settings`, `/api/tunings`, `/api/plugins*`, `/api/library/stats|albums|artists|genres|tuning-names|providers*`, `/api/work/*`, `/api/startup-status*`, `/api/scan-status`. This also removes the code behind the open CodeQL alerts in the old highway.
- Not yet carried over from the old player: loops, playback speed, difficulty levels and the tuner.

### Security

- The game page sends a strict Content-Security-Policy (same-origin only, no inline scripts, no framing).

## [0.3.0] - 2026-11-04

### Changed

- The out-of-the-box theme is now plain monochrome, in light and dark, with the system font.
- A band's `site.json` can now set the whole palette and choose the Rubik font, so a theme can look exactly as its band wants.

### Fixed

- The Leaderboards page always said "No scores yet", because its database query failed silently. Scores were saved correctly and now show up.

## [0.2.0] - 2026-11-03

### Changed
- The server is rewritten in Rust (axum + tokio). It answers every request the
  browser makes exactly as the Python server did; tests/golden pins that.
- Container image: one static binary on distroless/static (no shell, libc,
  OpenSSL, Python or pip). 61 MB instead of 293 MB, about 3 MB of RAM at idle,
  no known vulnerabilities at medium severity or above.
- The library index is in memory (manifest-only scan); SQLite is used only for
  leaderboard scores (same database file and schema as 0.1.0).
- The admin CLI is now `riffboard admin <command>`; the container health check
  is `riffboard healthcheck`.

### Added
- Per-site branding from `CONFIG_DIR/site/site.json`: name, tagline, hero
  copy and image, logo, favicon, About card with links, and dark/light
  colours (`/api/site`, `/site.css`, `/site/<image>`).
- Two play modes, picked each time a song starts. **Play for score**: the
  whole song straight through at full speed and difficulty; seeking, looping
  and speed controls are hidden and pausing ends the run. **Practice**: every
  control, never ranked. A practice run can be restarted as a score run.
- Song map: a section strip across the top of the player with the playhead.
  Tap or drag it to jump in practice mode; it only shows progress in score mode.
- Phones, tablets and low-end machines get a lighter 3D highway by default
  (no bloom, animated background or sparks; resolution capped at 1.5x). Any
  setting the visitor changes still wins.
- The automatic resolution scaler also reacts when the GPU, not JavaScript,
  is the bottleneck (frames arriving late), which is the usual case on phones.
- The container image ships brotli/gzip copies of the frontend; the server
  sends them to browsers that accept them (about 4 MB of JavaScript → under 1 MB).
- A second performance pass, measured on a phone-sized screen with the CPU
  slowed 8-12x and a fake microphone feeding note detection:
  - 3D highway: lane strips, lane dividers, fret wires and beat lines are
    batched (instancing / one line buffer per material), about a third fewer
    draw calls; world matrices are updated only for visible objects (the
    pools keep ~900 hidden meshes); overlay text is cached as bitmaps; the
    overlay canvas is left alone (and hidden) while it has nothing on it;
    hidden pool materials are compiled at load instead of mid-song.
  - Note detection: a real-input FFT with precomputed tables (2.2x faster,
    same spectrum), a time budget for the miss-rescue FFTs, and the HUD only
    writes text that changed.
  - The tuner badge no longer runs its own microphone stream and pitch
    tracker while the player is open, and never asks for the microphone on
    page load (only once permission was already granted).
  - The page under the full-screen player is no longer restyled and
    repainted every frame (idle-class churn, tuner badge meter, section
    practice labels, topbar/sidebar painted under the player, backdrop
    blurs over the moving canvas).
  - The automatic resolution scaler stops lowering resolution when that
    doesn't make frames cheaper (it was blurring the picture on CPU-bound
    phones for no gain).
  Result at a 12x slowdown without a microphone: 46 → 59.5 fps, frames
  over 25 ms 524 → 18, and the resolution stays at 100% instead of 25%.

- Player controls that work on a phone: a back button (top left), a settings
  cog (bottom left) that opens the player tools as a labelled row (Visuals,
  Input, Mixer, Lyrics, Panes, Plugins, Sections, More) instead of an
  invisible hover strip, and a mic on/off toggle next to the mode pill, so
  practice works without a microphone.
- The mode chooser says up front when score runs can't work (no microphone,
  or a non-https address).

### Fixed
- Note detection no longer retries an unavailable microphone forever: a
  failure that won't fix itself (insecure page, permission denied, no device)
  stops automatic retries for the session, and it is reported once as a toast
  instead of a blocking alert every few seconds. Tapping the mic toggle tries
  again.
- The "?" tour button is gone (the tour system isn't loaded any more), as are
  the drum-part and editor rows in the player's More panel.
- Leftover "Slopsmith"/"FeedBack" names in visible text, including the
  microphone-needs-HTTPS message.
- Note detection settings no longer offer server-side card auto-save or
  recording (the public build has no backend for them); "Save" on the results
  card downloads in the browser without trying the server first.

### Removed
- The unused Lottie animation library is no longer loaded (~300 KB).
- The Python server and its dependencies, the drum and keys highway plugins
  (guitar and bass charts only), drum tabs and notation streaming.

## [0.1.0] - 2026-11-02

First release: fee[dB]ack (forked at 0.3.0-alpha.1) rebuilt as a public,
play-only site.

### Added
- Leaderboard per song part: run tickets, server-side score checks, best run
  per name, rate limits, name normalisation and bans.
- Allowlist request guard and security headers for every route.
- Browser-side storage for all per-visitor state (settings, favourites, loops,
  practice stats, tuner config).
- New Home, Leaderboards and About pages; end-of-run name entry.
- Light and dark themes (system default, remembered override).
- Operator CLI `tools/riffboard_admin.py` (rescan, recent, delete, ban, wipe).
- Bundled note-detection plugin (v1.32.0, browser-only).
- Self-hosted Rubik font; RiffBoard icon and wordmark.

### Removed
- Career, achievements, XP/progression, shop, minigames, lessons, playlists,
  collections, loops/wanted/profile APIs, audio-effects mappings, diagnostics,
  settings import/export, uploads, song deletion and metadata editing,
  plugin updates, demo mode, MusicBrainz/Cover Art/AcoustID scraping and the
  Guitar Pro/MIDI importers.
- Seeding of upstream starter/diagnostic songs into the library.
- Loose XML+WEM folders (feedpak only), and with them ffmpeg, vgmstream,
  fluidsynth and Node from the image.

### Fixed
- The Songs screen stayed empty on its first visit if the startup scan had
  already finished.
