"""RiffBoard server: a public, play-only song library with highscores.

FastAPI app wiring. The heavy lifting lives in lib/ (pack reading, scanning,
the highway WebSocket) and lib/routers/ (HTTP routes). Everything a visitor can
reach is listed in lib/public_guard.py; this file only assembles the pieces.
"""

import asyncio
import json
import logging
import os
import re
import threading
import time
import concurrent.futures
from pathlib import Path
from typing import ClassVar

from logging_setup import configure_logging
from env_compat import getenv_compat

configure_logging()

from fastapi import FastAPI, Request  # noqa: E402
from fastapi.staticfiles import StaticFiles  # noqa: E402
from fastapi.responses import FileResponse, PlainTextResponse, StreamingResponse  # noqa: E402
from asgi_correlation_id import CorrelationIdMiddleware  # noqa: E402

from tunings import DEFAULT_REFERENCE_PITCH, DEFAULT_TUNINGS, apply_reference_pitch  # noqa: E402
from metadata_db import MetadataDB  # noqa: E402
from library_registry import LibraryProviderRegistry, LocalLibraryProvider  # noqa: E402
from dlc_paths import _get_dlc_dir, _resolve_dlc_path  # noqa: E402,F401  (tests patch these)
import sloppak as sloppak_mod  # noqa: E402,F401
from safepath import safe_join  # noqa: E402,F401  (tests patch this)
import admin_token  # noqa: E402
import appstate  # noqa: E402
import leaderboard as leaderboard_mod  # noqa: E402
import public_guard  # noqa: E402
import scan  # noqa: E402
from scan_worker import _extract_meta_for_file  # noqa: E402

from routers import (  # noqa: E402
    art as art_router,
    library as library_router,
    library_extras,
    media as media_router,
    public_settings,
    scores as scores_router,
    song as song_router,
    tunings as tunings_router,
    version,
    ws_highway,
)

log = logging.getLogger("riffboard.server")

app = FastAPI(title="RiffBoard", docs_url=None, redoc_url=None, openapi_url=None)
# Outermost: the allowlist runs before anything else touches the request.
app.add_middleware(CorrelationIdMiddleware, validator=None)
app.add_middleware(public_guard.PublicGuard)

STATIC_DIR = Path(__file__).parent / "static"

# DLC_DIR is the library folder. "" (unset) falls back to config.json's
# dlc_dir; see lib/dlc_paths.py. The folder is only ever READ.
_DLC_DIR_ENV = os.environ.get("DLC_DIR", "").strip()
DLC_DIR = Path(_DLC_DIR_ENV) if _DLC_DIR_ENV else Path("")
CONFIG_DIR = Path(os.environ.get("CONFIG_DIR", str(Path.home() / ".local" / "share" / "riffboard")))
CONFIG_DIR.mkdir(parents=True, exist_ok=True)

AUDIO_CACHE_DIR = CONFIG_DIR / "audio_cache"
SLOPPAK_CACHE_DIR = CONFIG_DIR / "sloppak_cache"


def _env_flag(name: str) -> bool:
    return (getenv_compat(name, "") or "").strip().lower() in {"1", "true", "yes", "on"}


meta_db = MetadataDB(CONFIG_DIR)
leaderboard = leaderboard_mod.Leaderboard(CONFIG_DIR)
admin_token.init(CONFIG_DIR)

library_providers = LibraryProviderRegistry()
_local_library_provider = LocalLibraryProvider(meta_db)
library_providers.register(_local_library_provider)


def register_library_provider(provider: object, *, replace: bool = False,
                              owner_plugin_id: str | None = None) -> object:
    return library_providers.register(provider, replace=replace, owner_plugin_id=owner_plugin_id)


def unregister_library_provider(provider_id: str) -> bool:
    return library_providers.unregister(provider_id)


class TuningProviderRegistry:
    """Registry for plugins that contribute custom tunings to the tuning list."""

    _ID_RE: ClassVar[re.Pattern[str]] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")

    def __init__(self) -> None:
        self._providers: dict[str, callable] = {}
        self._lock = threading.Lock()

    def register(self, provider_id: str, get_tunings: callable) -> None:
        if not self._ID_RE.match(provider_id):
            raise ValueError(f"tuning provider id {provider_id!r} contains invalid characters")
        if not callable(get_tunings):
            raise TypeError("get_tunings must be callable")
        with self._lock:
            self._providers[provider_id] = get_tunings

    def unregister(self, provider_id: str) -> None:
        with self._lock:
            self._providers.pop(provider_id, None)

    def get_merged(self, reference_pitch: float = DEFAULT_REFERENCE_PITCH) -> dict:
        result: dict[str, dict[str, list[float]]] = apply_reference_pitch(DEFAULT_TUNINGS, reference_pitch)
        scale = reference_pitch / DEFAULT_REFERENCE_PITCH
        with self._lock:
            providers = list(self._providers.items())
        for provider_id, get_tunings in providers:
            try:
                extra = get_tunings() or {}
                for instrument, names in extra.items():
                    result.setdefault(instrument, {})
                    for name, freqs in names.items():
                        result[instrument][name] = [round(f * scale, 4) for f in freqs]
            except Exception:
                log.exception("tuning provider %r raised during get_merged()", provider_id)
        return result


tuning_providers = TuningProviderRegistry()


def register_tuning_provider(provider_id: str, get_tunings: callable) -> None:
    tuning_providers.register(provider_id, get_tunings)


def unregister_tuning_provider(provider_id: str) -> None:
    tuning_providers.unregister(provider_id)


def _stat_for_cache(f: Path) -> tuple[float, int]:
    """(mtime, size) for cache freshness. Directory-form packs aggregate the
    files a metadata edit touches, since editing them doesn't bump the dir."""
    if f.is_dir():
        root = f.resolve()

        def _in_folder(p: Path) -> bool:
            try:
                p.resolve().relative_to(root)
            except (OSError, ValueError):
                return False
            return True

        xmls = [p for p in f.glob("*.xml") if _in_folder(p)]
        wems = [p for p in f.glob("*.wem") if _in_folder(p)]
        inner: list[Path] = []
        if xmls and wems:
            inner = xmls + wems + [p for p in f.glob("manifest.json") if _in_folder(p)]
        else:
            man = [p for p in (f / "manifest.yaml", f / "manifest.yml")
                   if p.exists() and _in_folder(p)]
            if man:
                inner = man
                inner += [p for p in f.glob("arrangements/*.json") if _in_folder(p)]
                inner += [p for p in f.glob("drum_tab.json") if _in_folder(p)]
        if inner:
            stats = []
            for p in inner:
                try:
                    stats.append(p.stat())
                except OSError:
                    continue
            if stats:
                return max(s.st_mtime for s in stats), sum(s.st_size for s in stats)
    st = f.stat()
    return st.st_mtime, st.st_size


def _server_root() -> Path:
    return Path(__file__).resolve().parent


def _running_version() -> str:
    env_version = os.environ.get("APP_VERSION", "").strip()
    if env_version:
        return env_version
    version_file = Path(__file__).parent / "VERSION"
    try:
        return version_file.read_text().strip()
    except (OSError, UnicodeDecodeError):
        return "unknown"


appstate.configure(
    meta_db=meta_db,
    leaderboard=leaderboard,
    config_dir=CONFIG_DIR,
    dlc_dir=DLC_DIR,
    dlc_dir_env=_DLC_DIR_ENV,
    static_dir=STATIC_DIR,
    sloppak_cache_dir=SLOPPAK_CACHE_DIR,
    audio_cache_dir=AUDIO_CACHE_DIR,
    library_providers=library_providers,
    local_library_provider=_local_library_provider,
    tuning_providers=tuning_providers,
    running_version=_running_version,
    default_settings=dict,
    kick_scan=scan.kick_scan,
    stat_for_cache=_stat_for_cache,
    scan_status=scan.status,
    server_root=_server_root(),
)


# ── Startup status (SSE) ──────────────────────────────────────────────────────
# The frontend shows a loading state until plugins have loaded; it follows this
# status over a small server-sent-events stream.
_STARTUP_STATUS_INIT = {
    "running": True, "phase": "booting", "message": "Starting RiffBoard...",
    "current_plugin": "", "loaded": 0, "total": 0, "error": None,
}
_startup_status = dict(_STARTUP_STATUS_INIT)
_startup_status_lock = threading.Lock()
_startup_sse_subscribers: set[asyncio.Queue] = set()
_startup_sse_lock = threading.Lock()
_event_loop: asyncio.AbstractEventLoop | None = None
_SSE_POLL_INTERVAL = 2.0
_SSE_KA_INTERVAL = 15.0
_SSE_MAX_SUBSCRIBERS = 200


def _put_latest(q: asyncio.Queue, snapshot: dict) -> None:
    while not q.empty():
        try:
            q.get_nowait()
        except asyncio.QueueEmpty:
            break
    try:
        q.put_nowait(snapshot)
    except asyncio.QueueFull:
        pass


def _set_startup_status(**updates):
    global _startup_status
    with _startup_status_lock:
        _startup_status = {**_startup_status, **updates}
        snapshot = dict(_startup_status)
    loop = _event_loop
    if loop is None or loop.is_closed():
        return
    with _startup_sse_lock:
        for q in _startup_sse_subscribers:
            try:
                loop.call_soon_threadsafe(_put_latest, q, snapshot)
            except RuntimeError:
                break


def _get_startup_status():
    with _startup_status_lock:
        return dict(_startup_status)


# ── Plugins ───────────────────────────────────────────────────────────────────
from plugins import load_plugins, register_plugin_api  # noqa: E402
register_plugin_api(app)


@app.on_event("startup")
async def startup_events():
    configure_logging()
    global _event_loop
    loop = asyncio.get_running_loop()
    _event_loop = loop

    if _env_flag("FEEDBACK_SKIP_STARTUP_TASKS"):
        from plugins import LOADED_PLUGINS, PENDING_PLUGINS, PLUGINS_LOCK
        with PLUGINS_LOCK:
            LOADED_PLUGINS.clear()
            PENDING_PLUGINS.clear()
        _set_startup_status(running=False, phase="complete",
                            message="Startup tasks skipped.", error=None)
        return

    plugin_context = {
        "config_dir": CONFIG_DIR,
        "get_dlc_dir": _get_dlc_dir,
        "extract_meta": lambda p: _extract_meta_for_file(p, _get_dlc_dir),
        "meta_db": meta_db,
        "get_scan_status": lambda: dict(scan.status()),
        "library_providers": library_providers,
        "register_library_provider": register_library_provider,
        "unregister_library_provider": unregister_library_provider,
        "register_tuning_provider": register_tuning_provider,
        "unregister_tuning_provider": unregister_tuning_provider,
        "get_sloppak_cache_dir": lambda: SLOPPAK_CACHE_DIR,
    }
    sync_mode = _env_flag("FEEDBACK_SYNC_STARTUP")

    def _route_setup_on_main(fn):
        # Router mutation isn't thread-safe: run plugin route setup on the loop.
        if sync_mode:
            fn()
            return
        fut: concurrent.futures.Future = concurrent.futures.Future()

        def _do():
            try:
                fn()
                fut.set_result(None)
            except Exception as exc:
                fut.set_exception(exc)

        loop.call_soon_threadsafe(_do)
        fut.result(timeout=60)

    def _on_progress(event: dict):
        _set_startup_status(
            running=True,
            phase=event.get("phase") or "plugins-loading",
            message=event.get("message") or "Loading plugins...",
            current_plugin=event.get("plugin_id") or "",
            loaded=int(event.get("loaded") or 0),
            total=int(event.get("total") or 0),
            **({"error": event["error"]} if event.get("error") else {}),
        )

    def _load_plugins_background():
        try:
            _set_startup_status(running=True, phase="plugins-loading",
                                message="Loading plugins...", error=None)
            load_plugins(app, plugin_context, progress_cb=_on_progress,
                         route_setup_fn=_route_setup_on_main)
            status = _get_startup_status()
            _set_startup_status(running=False, phase="complete", message="Startup complete",
                                current_plugin="", error=status.get("error"))
        except Exception as e:
            _set_startup_status(running=False, phase="error",
                                message="Plugin startup failed", error=str(e))
            log.exception("plugin startup failed")

    if sync_mode:
        _load_plugins_background()
    else:
        threading.Thread(target=_load_plugins_background, daemon=True).start()

    startup_scan()


def startup_scan():
    """Initial library scan + the periodic rescan thread (tests stub this)."""
    scan.kick_scan()
    threading.Thread(target=_periodic_rescan, daemon=True).start()


def _periodic_rescan():
    """Pick up packs dropped into the library folder (every 5 minutes by
    default; RIFFBOARD_RESCAN_SECONDS overrides, minimum 30)."""
    try:
        interval = max(30, int(os.environ.get("RIFFBOARD_RESCAN_SECONDS", "300")))
    except ValueError:
        interval = 300
    while True:
        time.sleep(interval)
        scan.kick_scan()


@app.on_event("shutdown")
def shutdown_events():
    global _event_loop
    _event_loop = None


# ── Status routes ─────────────────────────────────────────────────────────────
app.include_router(version.router)


@app.get("/api/scan-status")
def scan_status():
    st = dict(scan.status())
    # Don't leak server paths in error text.
    if st.get("error"):
        st["error"] = "library scan problem"
    return st


@app.get("/api/startup-status")
def startup_status():
    return _get_startup_status()


@app.get("/api/startup-status/stream")
async def startup_status_stream(request: Request):
    queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=1)
    with _startup_sse_lock:
        if len(_startup_sse_subscribers) >= _SSE_MAX_SUBSCRIBERS:
            # Startup is long over in practice; answer with the snapshot only.
            snap = _get_startup_status()
            return StreamingResponse(iter([f"data: {json.dumps(snap)}\n\n"]),
                                     media_type="text/event-stream")
        _startup_sse_subscribers.add(queue)
    await queue.put(_get_startup_status())

    async def _gen():
        since_ka = 0.0
        try:
            while True:
                try:
                    data = await asyncio.wait_for(queue.get(), timeout=_SSE_POLL_INTERVAL)
                except asyncio.TimeoutError:
                    if await request.is_disconnected():
                        break
                    since_ka += _SSE_POLL_INTERVAL
                    if since_ka >= _SSE_KA_INTERVAL:
                        yield 'data: {"type":"keepalive"}\n\n'
                        since_ka = 0.0
                    continue
                yield f"data: {json.dumps(data)}\n\n"
                if not data.get("running", True):
                    break
                since_ka = 0.0
                if await request.is_disconnected():
                    break
        finally:
            with _startup_sse_lock:
                _startup_sse_subscribers.discard(queue)

    return StreamingResponse(_gen(), media_type="text/event-stream",
                             headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"})


# ── Library + songs (read-only) ───────────────────────────────────────────────
app.include_router(library_router.router)
app.include_router(library_extras.router)
app.include_router(public_settings.router)
app.include_router(tunings_router.router)
app.include_router(scores_router.router)
app.include_router(art_router.router)
# Song info is a catch-all /api/song/{path}; it must mount after /art.
app.include_router(song_router.router)
app.include_router(media_router.router)
app.include_router(ws_highway.router)


# ── Static frontend ───────────────────────────────────────────────────────────
class _RevalidatedStaticFiles(StaticFiles):
    """StaticFiles with `Cache-Control: no-cache` so a deploy is picked up on
    the next load (the browser still revalidates cheaply with a 304)."""

    async def get_response(self, path, scope):
        response = await super().get_response(path, scope)
        response.headers.setdefault("Cache-Control", "no-cache")
        return response


app.mount("/static", _RevalidatedStaticFiles(directory=str(STATIC_DIR)), name="static")


@app.get("/")
def index():
    return FileResponse(str(STATIC_DIR / "v3" / "index.html"),
                        headers={"Cache-Control": "no-cache"})


@app.get("/favicon.ico")
def favicon():
    return FileResponse(str(STATIC_DIR / "v3" / "brand" / "favicon-32.png"))


@app.get("/robots.txt")
def robots():
    return PlainTextResponse("User-agent: *\nDisallow: /api/\nDisallow: /ws/\n")

