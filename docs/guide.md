# Hosting RiffBoard

## Getting started

You'll need Docker, plus your songs as `.feedpak` files ([format spec](https://github.com/got-feedback/feedpak-spec)). Subfolders are fine; `Artist/Song.feedpak` is a tidy layout. Drop new files into the library folder and they appear within five minutes.

Visitors need HTTPS for microphone access in most browsers, so put RiffBoard behind a reverse proxy or tunnel with TLS when it goes public.

## Configuration

All server settings come from the environment. None can be changed over HTTP.

| Variable | Default | What it does |
|---|---|---|
| `DLC_DIR` | `/dlc` | The song library folder (read-only is fine). |
| `CONFIG_DIR` | `/config` | Where scores, caches and the admin token live. Back this up. |
| `RIFFBOARD_CLIENT_IP_HEADER` | unset | Behind a proxy, the header holding the visitor's IP (for example `CF-Connecting-IP` or `X-Forwarded-For`), so rate limits apply per visitor. Only set it if your proxy always sets this header. |
| `RIFFBOARD_RESCAN_SECONDS` | `300` | How often the library is rescanned (minimum 30). |
| `APP_SOURCE_URL` | this repo | Where the footer's "Source code" link points. If you change the code, point it at your fork (see Licence). |
| `LOG_LEVEL` / `LOG_FORMAT` | `INFO` / text | Logging (`json` for structured logs). |

If the library's files belong to a specific user, run the container as that user (`user: "1000:1000"` in compose) and make sure it can write to the config volume.

## Branding your site

A RiffBoard install can present itself as your band's (or label's) own play-along site. Put a `site.json` and its images in `$CONFIG_DIR/site/`; see [docs/examples/site.json](examples/site.json). You can set the site name, tagline, home-page hero text and image, a logo and favicon, an "About" card with links, and colours for the dark and light themes.

Every value is validated: colours must be `#rrggbb`, links must be http(s), text is shown as plain text, and images must be PNG, JPEG, WebP or ICO files in that folder. Changes apply on the next page load, with no restart needed. Without a `site.json`, RiffBoard uses its plain monochrome theme, in light and dark, with the visitor's system accent colour where the browser offers one, so it suits any band out of the box.

To make your own colour theme, add `dark` and `light` sections to `site.json`, as in the example. You only need the colours you want to change:

| Keys | What they colour |
|---|---|
| `bg`, `sidebar`, `card`, `cardMuted`, `border` | Backgrounds and outlines |
| `primary`, `primaryHi`, `onPrimary` | Buttons and highlights, their hover state, and the text on them |
| `text`, `textDim` | Text |
| `gold` | The top score |

The site uses each visitor's system font. Set `"font": "rubik"` to use the Rubik font that ships with RiffBoard instead.

The game screen keeps its own colours: the string colours tell players which string to play.

## Admin tool

Moderation and rescans use the same binary's `admin` subcommand inside the container. It talks to admin routes that only answer on loopback with a token from the config volume:

```bash
docker compose exec riffboard /app/riffboard admin rescan
docker compose exec riffboard /app/riffboard admin recent 20
docker compose exec riffboard /app/riffboard admin delete 42
docker compose exec riffboard /app/riffboard admin ban "Some Name"
docker compose exec riffboard /app/riffboard admin wipe "Artist/Song.feedpak"
```

## Charting songs

RiffBoard plays charts; it doesn't make them. Use fee[dB]ack's editor or any tool that writes [feedpak](https://github.com/got-feedback/feedpak-spec) files, then copy the `.feedpak` into the library folder.

## Security

Upstream fee[dB]ack exposes about 120 unauthenticated endpoints that change server state: settings, uploads, deletes, plugin updates, metadata scraping. That's fine on a LAN and dangerous anywhere else. RiffBoard's server works the other way round:

- **Allowlist, not blocklist.** Every request passes through [`src/guard.rs`](../src/guard.rs). Only listed method and path pairs are served; anything else is a 404 before any route code runs. The whole public surface is a dozen routes.
- **Read-only library.** The only writes a visitor can make are "start a run" and "post a score". The library folder is never written to. The image runs as a non-root user and works with a read-only root filesystem.
- **Score checks.** Scores are computed by the server from hit/miss counts, with the same rules code the game uses ([`crates/core/src/rules.rs`](../crates/core/src/rules.rs)). A run needs a ticket issued when the song starts, and is only accepted from the same visitor after the chart's length has passed in real time. Hit counts must fit the chart. Both writes are rate-limited per visitor, names are normalised, and the operator can ban names.
- **Locked-down page.** The game page sends a strict Content-Security-Policy: scripts, styles and media only from the site itself, no inline scripts, no framing. There are no analytics, trackers, CDNs or third-party requests of any kind; the font is self-hosted.
- **Small, memory-safe server.** One static Rust binary on a distroless base: no shell, libc or OpenSSL in the image, a few MB of RAM at idle.

Note detection runs in the browser, so a determined cheater could still forge a score. The checks above make that tedious rather than impossible. For a community board that's normally enough, and the admin tool can clean up anything that slips through.
