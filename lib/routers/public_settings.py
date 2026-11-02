"""GET /api/settings: the server's DEFAULT player settings, read-only.

Upstream stored one settings file for the whole server and let any client
rewrite it (library path, scraper toggles, update channel...). RiffBoard
splits that in two:

* Server settings (library folder, scan behaviour) come from the environment
  and the operator's config file, and are never exposed or writable over HTTP.
* Player settings (instrument profile, A/V offset, difficulty, countdown...)
  start from the defaults returned here and are then overridden per visitor in
  their own browser by static/riffboard/local-api.js.

The operator can change the defaults every visitor starts from by putting a
``player_defaults`` object in ``<CONFIG_DIR>/config.json``; only keys listed in
PLAYER_KEYS are honoured.
"""

from fastapi import APIRouter

import appstate
from appconfig import _load_config
from tunings import settings_with_instrument_profiles

router = APIRouter()

# Player-facing keys a browser may hold. Anything else from the operator's
# config is ignored so server-only settings can never leak to the client.
PLAYER_KEYS = frozenset({
    "default_arrangement",
    "master_difficulty",
    "av_offset_ms",
    "countdown_before_song",
    "miss_penalty",
    "fail_behavior",
    "instrument_profiles",
    "active_instrument_profile",
    "instrument",
    "string_count",
    "tuning",
    "reference_pitch",
    "pathway",
})

_BASE_DEFAULTS = {
    "countdown_before_song": True,
    "miss_penalty": "none",
    "fail_behavior": "continue",
    "master_difficulty": 100,
    "av_offset_ms": 0,
    "default_arrangement": "",
}


def player_defaults() -> dict:
    out = dict(_BASE_DEFAULTS)
    cfg = _load_config(appstate.config_dir / "config.json") or {}
    extra = cfg.get("player_defaults") if isinstance(cfg, dict) else None
    if isinstance(extra, dict):
        out.update({k: v for k, v in extra.items() if k in PLAYER_KEYS})
    return settings_with_instrument_profiles(out)


@router.get("/api/settings")
def get_settings():
    return player_defaults()
