# Changelog

All notable changes to RiffBoard are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Upstream fee[dB]ack's history up to
the fork is in [docs/upstream-changelog.md](docs/upstream-changelog.md).

## [Unreleased]

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

### Removed
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
