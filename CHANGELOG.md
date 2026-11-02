# Changelog

All notable changes to RiffBoard are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Upstream fee[dB]ack's history up to
the fork is in [docs/upstream-changelog.md](docs/upstream-changelog.md).

## [Unreleased]

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
