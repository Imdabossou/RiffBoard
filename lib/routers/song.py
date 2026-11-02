"""Song route: GET /api/song/{filename} returns the per-song info payload.

RiffBoard serves the library read-only, so upload, delete, metadata
write-back, user-meta and gap-fill routes are gone. Songs are added by
dropping packs into the library folder; the periodic scan picks them up.
"""

from fastapi import APIRouter
from fastapi.responses import JSONResponse

import appstate
import sloppak as sloppak_mod
from dlc_paths import _get_dlc_dir, _resolve_dlc_path
from scan_worker import _extract_meta_for_file

import logging
log = logging.getLogger("feedBack.server")
router = APIRouter()


def _playable_stems_payload(filename: str, dlc) -> dict:
    """The playable stems (id/url/default) + full-mix URL for a sloppak.

    Why it exists: the stems plugin could only learn its stem list from the
    highway's WS `ready`, which arrives once the highway is already up. So it
    decoded, and then copied the whole song's PCM to its worklet, with the player
    on screen — half a gigabyte of memcpy in one frame, ~700 ms, freezing the
    venue video. Given the list at `song:loading` it can do all of that BEFORE the
    highway appears, behind the loading overlay where a stall costs nothing.

    The list MUST be the same one the WS sends a moment later. If it is not, the
    plugin preloads a graph and then throws it away and rebuilds — strictly worse
    than not preloading. So this does not reimplement the WS's construction, it
    calls THE SAME FUNCTION: load_song, whose LoadedSloppak already carries the
    partitioned stems and the resolved full mix, and then builds the URLs exactly
    as ws_highway does. Drift is impossible by construction rather than by
    agreement — which matters, because `full_mix` in particular is not simply the
    `full` stem: load_song falls back to the deprecated `original_audio:` key for
    every pack written before feedpak 1.15.0, and reimplementing that (I did, at
    first) silently dropped the pristine full mix for most real libraries.

    Opt-in (`?stems=1`) so the library's own metadata calls — the hot path — pay
    nothing for it. Non-sloppak sources (archives, loose folders) have no stems
    to preload: load_song raises and we return the empty list.
    """
    from urllib.parse import quote

    try:
        loaded = sloppak_mod.load_song(filename, dlc, appstate.sloppak_cache_dir)
    except Exception:
        return {"stems": [], "full_mix_url": None}

    q_fn = quote(filename, safe="")

    def _url(rel: str) -> str:
        return f"/api/sloppak/{q_fn}/file/{quote(rel)}"

    return {
        "stems": [
            {"id": s["id"], "url": _url(s["file"]), "default": s["default"],
             **{k: s[k] for k in ("name", "description") if k in s}}
            for s in loaded.stems
        ],
        "full_mix_url": _url(loaded.full_mix) if loaded.full_mix else None,
    }


@router.get("/api/song/{filename:path}")
async def get_song_info(filename: str, stems: int = 0):
    """Return song metadata, from cache or by extracting it from the song source.

    `?stems=1` additionally returns the playable stem list with URLs, so the
    stems plugin can start fetching/decoding on `song:loading` instead of waiting
    for the highway's WS `ready` (see _playable_stems_payload).
    """
    import asyncio
    dlc = _get_dlc_dir()
    if not dlc:
        return JSONResponse({"error": "DLC folder not configured"}, 404)

    song_path = _resolve_dlc_path(dlc, filename)
    if song_path is None:
        return JSONResponse({"error": "forbidden"}, 403)
    if not song_path.exists():
        return JSONResponse({"error": "File not found"}, 404)

    # Canonicalise the cache key against the resolved path so two URL
    # forms of the same physical file (e.g. `Artist/song.sloppak` vs
    # `Artist/../Artist/song.sloppak`) converge on a single row instead
    # of fragmenting / shadowing each other in appstate.meta_db.
    try:
        cache_key = song_path.relative_to(dlc.resolve()).as_posix()
    except ValueError:
        cache_key = filename

    mtime, size = appstate.stat_for_cache(song_path)
    cached = appstate.meta_db.get(cache_key, mtime, size)
    loop = asyncio.get_event_loop()

    # The stem list is NOT stored in the metadata cache: that is a fixed-column
    # table, and widening it would mean a migration plus a stale row for every
    # song already scanned. It is cheap to read on demand (the pack is unpacked
    # by then, so this is a plain manifest read), and only the opt-in caller pays.
    async def _with_stems(meta: dict) -> dict:
        if not stems:
            return meta
        extra = await loop.run_in_executor(
            None, _playable_stems_payload, filename, dlc)
        return {**meta, **extra}

    if cached:
        return await _with_stems(cached)

    # Extract in thread pool
    def _extract():
        meta = _extract_meta_for_file(song_path, dlc)
        appstate.meta_db.put(cache_key, mtime, size, meta)
        return meta

    meta = await loop.run_in_executor(None, _extract)
    return await _with_stems(meta)
