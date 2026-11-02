"""GET /api/settings: player settings, computed but never stored server-side.

Upstream stored one settings file for the whole server and let any client
rewrite it (library path, scraper toggles, update channel...). RiffBoard
splits that in two:

* Server settings (library folder, scan behaviour) come from the environment
  and the operator's config file, and are never exposed or writable over HTTP.
* Player settings (instrument profile, A/V offset, difficulty, countdown...)
  belong to each visitor and live in their browser. The server only does the
  arithmetic: the browser sends its saved state (``state``) and optionally a
  change (``patch``) as query parameters, and gets back the normalised
  settings plus the new state to save. Nothing is written on the server.

static/riffboard/local-api.js is the browser half; it answers the app's
existing ``GET``/``POST /api/settings`` calls through this route.

The operator can change the defaults every visitor starts from with a
``player_defaults`` object in ``<CONFIG_DIR>/config.json``; only keys listed in
PLAYER_KEYS are honoured.
"""

from __future__ import annotations

import json
import math

from fastapi import APIRouter
from fastapi.responses import JSONResponse

import appstate
from appconfig import _load_config
from tunings import (
    PROFILE_IDS, PROFILE_PATHWAYS, apply_flat_instrument_patch_to_profiles,
    normalize_instrument_profile, normalize_instrument_profiles,
    settings_with_instrument_profiles,
)

router = APIRouter()

PLAYER_KEYS = frozenset({
    "default_arrangement", "master_difficulty", "av_offset_ms",
    "countdown_before_song", "miss_penalty", "fail_behavior",
    "instrument_profiles", "active_instrument_profile",
    "instrument", "string_count", "tuning", "reference_pitch", "pathway",
})
_PROFILE_KEYS = ("instrument", "string_count", "tuning", "reference_pitch",
                 "pathway", "instrument_profiles", "active_instrument_profile")
_MAX_PARAM = 8192

_BASE_DEFAULTS = {
    "countdown_before_song": True,
    "miss_penalty": "none",
    "fail_behavior": "continue",
    "master_difficulty": 100,
    "av_offset_ms": 0,
    "default_arrangement": "",
}


class _Bad(ValueError):
    pass


def _defaults() -> dict:
    out = dict(_BASE_DEFAULTS)
    cfg = _load_config(appstate.config_dir / "config.json") or {}
    extra = cfg.get("player_defaults") if isinstance(cfg, dict) else None
    if isinstance(extra, dict):
        out.update({k: v for k, v in extra.items() if k in PLAYER_KEYS})
    return out


def _num(raw, lo, hi, name):
    if isinstance(raw, bool) or not isinstance(raw, (int, float, str)):
        raise _Bad(f"{name} must be a number between {lo} and {hi}")
    try:
        v = float(raw)
    except ValueError:
        raise _Bad(f"{name} must be a number between {lo} and {hi}") from None
    if not math.isfinite(v):
        raise _Bad(f"{name} must be a number between {lo} and {hi}")
    return max(lo, min(hi, v))


def validate_patch(data: dict) -> tuple[dict, dict | None]:
    """Validate a settings change the way upstream's save did (player keys
    only). Returns (flat updates, instrument-profile patch or None)."""
    updates: dict = {}
    if "default_arrangement" in data and data["default_arrangement"] is not None:
        raw = data["default_arrangement"]
        if not isinstance(raw, str) or len(raw) > 64:
            raise _Bad("default_arrangement must be a short string")
        updates["default_arrangement"] = raw
    if "master_difficulty" in data and data["master_difficulty"] is not None:
        updates["master_difficulty"] = int(_num(data["master_difficulty"], 0, 100, "master_difficulty"))
    if "av_offset_ms" in data and data["av_offset_ms"] is not None:
        updates["av_offset_ms"] = _num(data["av_offset_ms"], -1000.0, 1000.0, "av_offset_ms")
    if "countdown_before_song" in data and data["countdown_before_song"] is not None:
        if not isinstance(data["countdown_before_song"], bool):
            raise _Bad("countdown_before_song must be a boolean")
        updates["countdown_before_song"] = data["countdown_before_song"]
    if "miss_penalty" in data and data["miss_penalty"] is not None:
        if data["miss_penalty"] not in ("none", "low", "medium", "high"):
            raise _Bad("miss_penalty must be one of none, low, medium, high")
        updates["miss_penalty"] = data["miss_penalty"]
    if "fail_behavior" in data and data["fail_behavior"] is not None:
        if data["fail_behavior"] not in ("continue", "restart", "stop"):
            raise _Bad("fail_behavior must be continue, restart or stop")
        updates["fail_behavior"] = data["fail_behavior"]
    if "reference_pitch" in data and data["reference_pitch"] is not None:
        updates["reference_pitch"] = _num(data["reference_pitch"], 430.0, 450.0, "reference_pitch")
    if "instrument" in data and data["instrument"] is not None:
        if data["instrument"] not in ("guitar", "bass"):
            raise _Bad("instrument must be 'guitar' or 'bass'")
        updates["instrument"] = data["instrument"]
    if "string_count" in data and data["string_count"] is not None:
        raw = data["string_count"]
        if isinstance(raw, float) and raw.is_integer():
            raw = int(raw)
        if isinstance(raw, bool) or not isinstance(raw, int) or not 4 <= raw <= 8:
            raise _Bad("string_count must be an integer between 4 and 8")
        updates["string_count"] = raw
    if "tuning" in data and data["tuning"] is not None:
        raw = data["tuning"]
        if isinstance(raw, str):
            if len(raw) > 64:
                raise _Bad("tuning name too long")
        elif isinstance(raw, list):
            if len(raw) > 8 or any(isinstance(o, bool) or not isinstance(o, int)
                                   or not -12 <= o <= 12 for o in raw):
                raise _Bad("tuning offsets must be ≤8 integers between -12 and 12")
        else:
            raise _Bad("tuning must be a name or a list of semitone offsets")
        updates["tuning"] = raw
    if "pathway" in data and data["pathway"] is not None:
        if data["pathway"] not in PROFILE_PATHWAYS:
            raise _Bad("unknown pathway")
        updates["pathway"] = data["pathway"]
    profile_patch = None
    if "instrument_profiles" in data and data["instrument_profiles"] is not None:
        raw = data["instrument_profiles"]
        if not isinstance(raw, dict):
            raise _Bad("instrument_profiles must be an object")
        profile_patch = {}
        for pid, praw in raw.items():
            if pid not in PROFILE_IDS:
                raise _Bad(f"unknown instrument profile: {pid}")
            prof, err = normalize_instrument_profile(pid, praw)
            if err:
                raise _Bad(err)
            profile_patch[pid] = prof
    if "active_instrument_profile" in data and data["active_instrument_profile"] is not None:
        if data["active_instrument_profile"] not in PROFILE_IDS:
            raise _Bad("unknown instrument profile")
        updates["active_instrument_profile"] = data["active_instrument_profile"]
    return updates, profile_patch


def apply(state: dict, patch: dict) -> dict:
    """New stored state = state + validated patch (upstream save semantics)."""
    updates, profile_patch = validate_patch(patch)
    cfg = dict(state)
    cfg.update(updates)
    if profile_patch is not None:
        existing, _ = normalize_instrument_profiles(cfg.get("instrument_profiles"))
        existing = existing or {}
        existing.update(profile_patch)
        cfg["instrument_profiles"] = existing
    if "instrument_profiles" in cfg or any(k in updates for k in _PROFILE_KEYS):
        cfg = apply_flat_instrument_patch_to_profiles(cfg, updates)
        cfg = settings_with_instrument_profiles(cfg)
    return {k: v for k, v in cfg.items() if k in PLAYER_KEYS}


def _parse(raw: str, name: str) -> dict:
    if not raw:
        return {}
    if len(raw) > _MAX_PARAM:
        raise _Bad(f"{name} is too large")
    try:
        val = json.loads(raw)
    except ValueError:
        raise _Bad(f"{name} must be JSON") from None
    if not isinstance(val, dict):
        raise _Bad(f"{name} must be an object")
    return val


@router.get("/api/settings")
def get_settings(state: str = "", patch: str = ""):
    try:
        st = _parse(state, "state")
        # A saved state is re-validated as a patch over the defaults, so a
        # tampered localStorage value can't smuggle in anything invalid.
        try:
            stored = apply({}, {k: v for k, v in st.items() if k in PLAYER_KEYS})
        except (_Bad, ValueError):
            stored = {}
        new_state = apply(stored, _parse(patch, "patch")) if patch else stored
    except (_Bad, ValueError) as exc:
        return JSONResponse({"error": str(exc)}, status_code=400)
    settings = settings_with_instrument_profiles({**_defaults(), **new_state})
    return {**settings, "_state": new_state}
