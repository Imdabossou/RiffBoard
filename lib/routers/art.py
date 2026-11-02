"""Album-art route: GET /api/song/{filename}/art serves the art a pack ships.

RiffBoard keeps only the read path. User overrides, cover search and the
Cover Art Archive lookup are gone: art comes from the pack itself.
"""

import asyncio
import hashlib
from pathlib import Path

from fastapi import APIRouter, Request
from fastapi.responses import FileResponse, JSONResponse, Response

import loosefolder as loosefolder_mod
import sloppak as sloppak_mod
from dlc_paths import _get_dlc_dir, _resolve_dlc_path

import logging
log = logging.getLogger("feedBack.server")
router = APIRouter()

def _if_none_match_hits(header: str | None, etag: str) -> bool:
    """True if an If-None-Match header matches `etag` (weak comparison).

    Handles the `*` wildcard and comma-separated lists, and ignores a weak
    `W/` prefix on either side — the standard semantics for a conditional GET.
    """
    if not header:
        return False
    bare = etag.removeprefix("W/")
    for tok in header.split(","):
        t = tok.strip()
        if t == "*" or t.removeprefix("W/") == bare:
            return True
    return False


# Album art is served with a strong validator (an ETag on the sloppak byte
# path; FileResponse's own ETag/Last-Modified on the file paths) and revalidated
# with `no-cache`. That keeps re-scroll cheap — a conditional GET returns a
# bodyless 304 — without ever serving a stale cover. A long `immutable` max-age
# was rejected: the frontend's `?v=<mtime>` buster is only second-resolution, so
# a same-second cover rewrite would keep the URL and pin the old bytes for the
# cache lifetime. Validation cost is negligible for a localhost backend.
_ART_CACHE_HEADERS = {"Cache-Control": "no-cache"}


def _art_etag(path: Path) -> str | None:
    """Strong validator for an art file: nanosecond mtime + size (so a
    same-second rewrite still changes it). None if the file can't be stat'd."""
    try:
        st = path.stat()
        return f'"{st.st_mtime_ns}-{st.st_size}"'
    except OSError:
        return None


def _art_conditional(etag: str | None, request: Request | None):
    """Return (headers, not_modified) for an art response. `not_modified` is
    True when the client's If-None-Match already matches `etag` → caller should
    return a bodyless 304. Starlette's FileResponse emits an ETag but does NOT
    itself evaluate If-None-Match, so every art path routes through here to get
    real conditional handling."""
    headers = dict(_ART_CACHE_HEADERS)
    if etag:
        headers["ETag"] = etag
    inm = request.headers.get("if-none-match") if request is not None else None
    return headers, bool(etag) and _if_none_match_hits(inm, etag)


def _file_art_response(path: Path, media_type: str, request: Request | None):
    """FileResponse for an on-disk art file, with no-cache + ETag and a bodyless
    304 when the client's validator still matches."""
    headers, not_modified = _art_conditional(_art_etag(path), request)
    if not_modified:
        return Response(status_code=304, headers=headers)
    return FileResponse(str(path), media_type=media_type, headers=headers)


@router.get("/api/song/{filename:path}/art")
async def get_song_art(filename: str, request: Request = None):
    """Serve the cover art a song pack ships. 404 when it has none."""
    dlc = _get_dlc_dir()
    if not dlc:
        return JSONResponse({"error": "not configured"}, 404)

    song_path = _resolve_dlc_path(dlc, filename)
    if song_path is None:
        return JSONResponse({"error": "forbidden"}, 403)
    if not song_path.exists():
        return JSONResponse({"error": "not found"}, 404)

    # Sloppak: read the cover (manifest-declared or default) straight from
    # the package. For a zip-form sloppak this opens just the cover member —
    # NOT the whole archive — so the library grid never triggers a full unpack
    # of stems just to paint a thumbnail.
    if sloppak_mod.is_sloppak(song_path):
        # Read the cover (cheap — single member, no full unpack) and validate by
        # its CONTENT. A stat-based ETag would be wrong for directory-form
        # sloppaks: editing cover.jpg in place changes the file's mtime, not the
        # directory's, so a dir-stat ETag could emit a stale 304. Content hashing
        # is correct for both dir- and zip-form. Raw byte Response lacks
        # FileResponse's validators, so we attach the ETag + honor If-None-Match.
        try:
            art = await asyncio.to_thread(sloppak_mod.read_cover_bytes, song_path)
        except Exception:
            art = None
        if art is not None:
            data, mt = art
            etag = f'"{hashlib.sha1(data).hexdigest()}"'
            headers, not_modified = _art_conditional(etag, request)
            if not_modified:
                return Response(status_code=304, headers=headers)
            return Response(content=data, media_type=mt, headers=headers)

    # Loose folder: serve the discovered art file directly.
    # song_path is already validated against DLC_DIR by _resolve_dlc_path.
    elif loosefolder_mod.is_loose_song(song_path):
        art_path = loosefolder_mod.find_art(song_path)
        if art_path:
            # Re-resolve in case the matched file is a symlink — a crafted
            # custom song could put `album_art.jpg` as a symlink to anywhere on
            # disk. Insist the final target stays inside the song folder.
            art_resolved = art_path.resolve()
            try:
                art_resolved.relative_to(song_path)
            except ValueError:
                return JSONResponse({"error": "forbidden"}, 403)
            if art_resolved.is_file():
                mt = {
                    ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
                    ".png": "image/png", ".webp": "image/webp",
                }.get(art_resolved.suffix.lower(), "image/jpeg")
                return _file_art_response(art_resolved, mt, request)

    return JSONResponse({"error": "no art"}, 404)

